# Federation and virtual-graph plugin plan

Status: proposed, 2026-11-02. This is a design and adoption plan only. It
does not change the current query endpoint, plugin SDK, or any code.

This plan supersedes Phase 16 (OBDA/R2RML in Rust) of `rdfstore-rust-build-plan.md`, which is rejected in favour of
the plugin/sidecar model so the feature can ship quickly.

## Decision summary

Add two separately deployable, opt-in plugin packages.

1. **Federation gateway:** use [Comunica](https://comunica.dev/) as the
   query-engine sidecar, exposed as a SPARQL Protocol endpoint. It is the
   recommended implementation for standard `SERVICE` evaluation across the
   approved business-unit endpoints.
2. **Virtual graph:** use [Ontop](https://ontop-vkg.org/) as the R2RML/SQL
   sidecar, exposed as a SPARQL Protocol endpoint. It is the recommended
   implementation for SPARQL-to-SQL rewriting over a business unit's
   relational source.

Both packages are integrated through the existing plugin/sidecar operating
model, analogous to the Qdrant semantic-search plugin. Rust remains the owner
of local RDF storage, plugin configuration and policy; it does **not** grow a
new federated optimizer or an R2RML-to-SQL compiler.

The initial system remains read-only across federation boundaries. Neither
gateway is an authority to write remote RDF or source SQL databases.

## The problem being solved

Each business unit (BU) owns its graph and answers the overwhelming majority
of its requests locally. A smaller, intentional set of queries enriches a
local HR, finance, or engineering result with data owned by another BU. The
architecture must make those crossings explicit, governed, observable and
safe, instead of flattening all data into one global graph.

The same federation mechanism must be able to reach a virtual graph. A BU may
therefore leave relational data in its source database while publishing an
R2RML-mapped SPARQL endpoint that other approved graphs can use.

## Standards contract

### Query syntax and transport

The public syntax is SPARQL federated query syntax, not a new plugin language:

```sparql
PREFIX norm: <https://kg.example.org/vocab/normalization#>

SELECT ?employee ?costCentre ?project
WHERE {
  ?employee norm:canonicalId ?person .
  SERVICE <https://kg.example.org/finance/sparql> {
    ?financePerson norm:canonicalId ?person ;
                   <https://kg.example.org/vocab/finance#costCentre> ?costCentre .
  }
  SERVICE <https://kg.example.org/engineering/sparql> {
    ?member norm:canonicalId ?person ;
            <https://kg.example.org/vocab/engineering#project> ?project .
  }
}
```

- Preserve ordinary `SERVICE`, `SERVICE SILENT`, service IRIs, result
  multiplicity and error behavior from the [SPARQL Federated Query
  specification](https://www.w3.org/2009/sparql/docs/fed/gen.html).
- Use the [SPARQL Protocol](https://www.w3.org/TR/sparql11-protocol/) for the
  gateway and every BU endpoint: `query` over HTTP with content negotiation.
- Each endpoint publishes an RDF [SPARQL Service
  Description](https://www.w3.org/2009/sparql/docs/service-description-1.1/gen.html)
  on `GET` of its endpoint URI.
- A non-`SILENT` remote failure fails the query; `SERVICE SILENT` has only the
  standard empty-binding fallback. It is not a general availability mechanism.

The first release requires a concrete, registered endpoint IRI in every
`SERVICE` clause. `SERVICE ?service` is valid SPARQL, but is deferred until
there is a bounded source-selection policy and an explicit fan-out budget.
There is no `SERVICE bu:finance`, no hidden endpoint substitution and no
proprietary federation syntax.

### Normalization semantics

The federation layer must not silently turn unlike RDF terms into equal terms.
That would change SPARQL equality, obscure provenance and make incorrect
identity assertions hard to undo. Instead, each participating graph publishes
an explicit link from its local resource to a stable enterprise canonical IRI:

```turtle
@prefix norm: <https://kg.example.org/vocab/normalization#> .
@prefix prov: <http://www.w3.org/ns/prov#> .

<https://hr.example.org/employee/4821>
  norm:canonicalId <https://id.example.org/person/01J...> ;
  prov:wasDerivedFrom <https://hr.example.org/system/hris> .
```

`norm:canonicalId` is an enterprise predicate with an exact, documented
meaning: this resource denotes the enterprise identity named by the object. It
is **not** a shorthand for `owl:sameAs`; it makes no claims about every
property, graph or temporal state of the two resources. Canonical IDs are IRIs,
never mutable display keys or database primary keys. The plan must define the
domain/range, lifecycle, deprecation and proof/provenance requirements for this
vocabulary before production mappings are approved.

Query authors bind and join the canonical IRI explicitly, as in the example.
Automatic result rewriting, implicit join injection and fuzzy/entity-resolution
matching are out of scope for the first release.

## Registry: RDF control plane, not a key/value sidecar

The central registry is a protected RDF dataset (a dedicated store is allowed;
a protected named graph in the central store is acceptable). It is logically
authoritative and may be replicated/cached for availability. A local key/value
cache is permitted only as a read-through implementation detail; it is never
the source of truth.

The registry records:

| Concern | Registry record |
| --- | --- |
| Endpoint | `sd:Service`, `sd:endpoint`, supported query language/result formats, BU owner, health policy |
| Dataset | dataset/graph IRIs, vocabulary/profile declaration, data classification and allowed consumers; use `void:sparqlEndpoint`/linkset metadata where useful |
| Access | caller identity/BU, allowed target endpoint/graph and query limits; credentials stay in the gateway secret store, never in RDF |
| Identity | canonical-ID namespace, local mapping version, owner, provenance, effective interval and approval state |
| Virtual graph | Ontop endpoint, mapping artifact digest, database-source class, SQL dialect, supported feature profile and approval state |
| Audit | immutable version, submitter, reviewer, approval timestamp, supersedes/revokes relation and change ticket |

Use W3C `sd:` and VoID terms for interoperable endpoint/dataset description;
put organization-specific approval, access and normalization metadata under a
versioned `norm:` vocabulary. The registry has two views:

- **staging:** a BU may submit/update only its own endpoint and mapping records;
- **active:** only centrally reviewed, complete, non-revoked records are visible
  to federation resolution.

The plugin resolves the literal `SERVICE` IRI against active registry policy
before dispatch. The IRI remains the actual endpoint URI, so the query stays
portable SPARQL and review cannot turn it into an arbitrary SSRF request.

## Target topology

```text
                         protected RDF registry
                  endpoints + policy + mappings + approvals
                                      |
                                      v
caller --> federation SPARQL endpoint (Comunica sidecar)
                         |         |          |
                 local BU graph    |          +--> Ontop virtual-graph endpoint
                   (Rust store)    |                   |  R2RML + read-only JDBC
                                  HR/finance/             v
                                  engineering          BU SQL database
                                  SPARQL endpoints
```

The federation package has a small Rust-facing control adapter, not a query
engine: it renders an allowlisted sidecar configuration from the active
registry, supplies short-lived credentials, performs readiness checks, exposes
status/metrics and invalidates its cache on an approved registry version. The
Comunica sidecar owns parsing/evaluation of remote SPARQL; the Ontop sidecar
owns R2RML interpretation and SQL generation.

For a query with no `SERVICE`, use the current local Rust endpoint unchanged.
For a federated query, route to the federation endpoint. A later host-level
query-router may make this a single public endpoint, but must not be smuggled
in as a plugin implementation detail.

## Open-source evaluation and selection

| Need | Candidate | Evidence and fit | Decision |
| --- | --- | --- | --- |
| Standards-aware federated endpoint | **Comunica SPARQL HTTP** | MIT-licensed; documents support for SPARQL 1.2 Federated Query and provides `comunica-sparql-http` for serving a SPARQL endpoint. Its modular sources/actors suit an allowlisted registry-driven deployment. | **Adopt first; benchmark and pin an image/digest.** |
| Optimized configured federation | **Eclipse RDF4J FedX** | Mature transparent federation with source selection, bind joins, per-source monitoring and a server deployment path. Its default model automatically selects sources rather than preserving the requested explicit `SERVICE`-first contract. JVM/RDF4J operation is a heavier fit. | **Keep as benchmark/fallback, not the initial endpoint.** Reconsider if the measured selective-join workload exceeds Comunica's limits. |
| Basic SPARQL `SERVICE` | **Apache Jena ARQ/Fuseki** | Mature standards-oriented implementation. Jena explicitly describes `SERVICE` as a basic building block, cautions that it is not a complete distributed-query optimizer, and provides request controls. | **Use as a conformance oracle/reference, not the primary federation engine.** |
| Virtual RDF graph over SQL | **Ontop** | Apache-2.0 project; supports R2RML, translates a large SPARQL 1.1 fragment to SQL, and publishes a CLI/Docker SPARQL endpoint. This is exactly the compiler/runtime that should not be recreated in Rust. | **Adopt first; validate against every target DB dialect and required query fragment.** |

Sources: [Comunica supported specifications](https://comunica.dev/docs/query/advanced/specifications/),
[Comunica HTTP endpoint release note](https://comunica.dev/blog/2026-04-14-release_5_2/),
[Comunica license](https://github.com/comunica/comunica),
[FedX federation guide](https://rdf4j.org/documentation/programming/federation/),
[Jena SERVICE guide](https://jena.apache.org/documentation/query/service.html),
[Ontop concepts](https://ontop-vkg.org/guide/concepts.html), and
[Ontop deployment guide](https://ontop-vkg.org/guide/getting-started.html).

Adoption is conditional, not assumed: complete the proof-of-concept and
security/license review below before making either sidecar a supported product
dependency. Pin exact images/binaries and their checksums; do not float `latest`.

## Plugin boundaries and required host work

Today the normal Rust HTTP layer rejects `SERVICE` and `LOAD`, and the current
SDK has no query interception or external-SPARQL-service contract. A plugin
cannot honestly add a standards-compliant general federation endpoint solely
through the existing read-context/property-function API.

Plan the work as an explicit SDK/host extension, after the sidecar proof of
concept:

1. Define a versioned `ExternalSparqlService`/gateway contract: endpoint ID,
   registry version, query/result streaming limits, cancellation, timeout,
   structured error and telemetry schema. Do not expose database files or
   arbitrary network clients to plugins.
2. Add composition-time registration of a federation gateway. The generic
   server chooses local versus federated execution before local SPARQL
   evaluation; it does not add plugin-specific HTTP routes.
3. Publish a separate federation SPARQL endpoint first. Unifying it with
   `/query` is a later, tested routing decision once result/error/cancellation
   semantics match exactly.
4. Add an administrative registry-sync/status operation, but do not allow an
   ordinary data query to mutate registry state or sidecar configuration.
5. Keep `LOAD`, remote updates, arbitrary URL dereferencing and arbitrary
   SQL/connection strings disabled.

The virtual-graph package conforms to the same external-service contract.
From the federation gateway's point of view, an Ontop endpoint is simply an
approved SPARQL service. This lets a query federate local RDF, another BU's RDF
store and an R2RML/SQL graph through the same `SERVICE` semantics.

## Delivery phases and exit criteria

### Phase 0 — contract and governance

- Define the `norm:` ontology, canonical-ID lifecycle, endpoint ownership,
  approval/revocation workflow, data-classification rules and audit retention.
- Define the active/staging registry shapes (SHACL is recommended) and a
  policy decision record for every `SERVICE` target.
- Decide authentication propagation (gateway service identity versus delegated
  user identity) and the allowed credential mechanism.
- Write threat model and operational SLOs: maximum endpoints/query, request and
  total deadlines, result/byte caps, concurrency, registry-cache TTL, and
  `SERVICE SILENT` alerting.

**Exit:** security/data owners approve a sample HR-to-finance canonical-ID
mapping and a revocation procedure; no query may contact an unregistered host.

### Phase 1 — isolated federation proof of concept

- Deploy pinned Comunica SPARQL HTTP, two disposable SPARQL endpoints and a
  read-only active registry fixture.
- Verify SPARQL syntax/evaluation, `SERVICE SILENT`, `VALUES`-bound selective
  joins, result formats, endpoint-down behavior, timeouts and cancellation.
- Verify policy deny, redirect denial, DNS/IP allowlisting, credential
  isolation, request/result limits and per-BU audit events.
- Differential-test supported queries against Jena ARQ; benchmark the same
  representative workload against FedX before committing the engine choice.

**Exit:** all semantic fixtures match their oracle; unapproved endpoints cannot
be reached; the 90/10 workload meets agreed latency and remote-request budgets.

### Phase 2 — registry-backed federation package

- Implement the control adapter, sidecar configuration renderer, registry
  version cache and health/status reporting.
- Introduce the SDK/host contract described above, with no regression to local
  query behavior.
- Release the standalone federation endpoint behind feature/config opt-in.
- Add runbooks for endpoint onboarding, approved mapping promotion, rotation,
  revocation, sidecar rollback and incident triage.

**Exit:** a staged mapping never affects query results; an approved promotion is
atomic from the gateway's observed registry version; rollback restores the
previous configuration without changing source graphs.

### Phase 3 — R2RML virtual-graph proof of concept

- Deploy pinned Ontop with a read-only database account, one representative
  schema and an R2RML mapping artifact stored/versioned in the registry.
- Test basic graph patterns, joins, filters, `OPTIONAL`, `UNION`, `VALUES`,
  language/datatype handling, nulls, pagination and the target SQL dialect.
- Capture unsupported SPARQL and R2RML features as explicit failures; do not
  claim complete SPARQL coverage merely because simple examples work.
- Demonstrate the virtual graph through `SERVICE` from the federation gateway,
  including a canonical-ID join to a local RDF graph.

**Exit:** generated SQL and result multiplicities match golden datasets; the
SQL account cannot write; mapping-digest/config mismatch prevents publication.

### Phase 4 — controlled production rollout

- Start with one non-sensitive read-only cross-BU use case and one virtual SQL
  source. Keep direct local queries as the default path.
- Publish dashboards for registry version, service health, remote calls/query,
  latency, bytes/results, denials, mapping version and sidecar errors.
- Load-test bounded fan-out, endpoint loss, registry rollback and database
  failover; rehearse credential rotation and mapping revocation.
- Expand BUs only after the metrics and ownership review are satisfactory.

**Exit:** every active endpoint/mapping has an owner, approval, expiry/review
date, observability and tested rollback path.

## Non-goals for the initial release

- A global merged graph or automatic source discovery.
- Arbitrary `SERVICE` URLs, `SERVICE ?service`, redirects, remote updates or
  `LOAD`.
- Rewriting `owl:sameAs`, fuzzy matching, automatic entity resolution or
  implicit normalization joins.
- A Rust federation optimizer, custom distributed-query syntax, or a Rust
  SPARQL-to-SQL/R2RML implementation.
- Cross-system transactions, snapshot consistency guarantees or backup atomicity
  across the RDF store, gateway, Ontop and SQL databases.
- Treating a green unit test/build as proof that browser, sidecar, remote-endpoint
  or database behavior is verified.

## Open decisions to close before implementation

1. Is access evaluated as the calling user (delegated identity) or as the
   federation gateway service identity, and which BUs/data classifications allow
   each mode?
2. Which SQL engines and versions are supported in the first Ontop profile?
3. What exact SPARQL feature profile is guaranteed end-to-end, given the local
   engine, Comunica and Ontop feature intersections?
4. What are the objective latency, request-count and availability budgets for a
   cross-BU query, and when should the client choose asynchronous materialization
   rather than live federation?
5. What license, CVE/SBOM, image provenance and support-ownership policy must a
   Java/Node sidecar meet before approval?
