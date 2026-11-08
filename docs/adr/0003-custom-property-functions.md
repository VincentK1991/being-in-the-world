# ADR 0003: Bounded SPARQL property-function plugins

Status: accepted; extends ADR 0002 and upgrades the SDK to v2.

Search needs a relation such as `(document, score)`, not just one scalar value.
Oxigraph 0.5.13 exposes scalar function hooks but no Jena-style property-function
hook. We introduce a bounded adapter over parsed `spargebra` query algebra, while
Oxigraph continues to execute all SPARQL joins, filters, aggregates and results.

The SDK owns `PropertyFunction`, `PropertyDescriptor`, `RdfValue`, and `RowSink`.
Plugins declare IRI-named relations with fixed input/output arities. SDK v2 adds
`Manifest.properties` and a default-empty `Plugin::property_functions` method;
existing SDK v1 plugins must recompile, add the manifest field and adopt v2.
Registration captures descriptors, validates ownership/arity/IRI uniqueness, and
publishes the plugin and all properties atomically.

The engine validates query policy and the entire property call shape before
executing callbacks. It resolves query-syntax RDF collections, invokes functions
with constant arguments and a dataset-scoped read context, validates bounded RDF
rows, and replaces property patterns with VALUES relations. No string replacement,
HTTP routing changes, external SERVICE fetching or core evaluator fork is used.
Unknown reserved property IRIs and unsupported invocation modes fail explicitly.

The supported profile and limits are defined in the
[plugin contract](../../plugins/custom-properties/README.md). Constant-input
relations provide the first extension point for future literal-query search.
Correlated input bindings, active named-graph context, transactional consistency
across plugin reads, index lifecycle/recovery, scalar functions and dynamic plugin
installation remain separate designs; this change does not claim those features.

The default application composes dataset-summary and custom-properties plugins
once for file/database/HTTP queries and CLI diff. CLI updates now use the runtime
too, so unsupported property use in UPDATE fails consistently. Studio exposes
the discovery metadata and includes a working token/position example.

Compatibility: ordinary RDF predicates keep their meaning unless registered as
virtual properties. Direct use of registered property IRIs selects the virtual
relation; variable predicates enumerate only stored triples. Stored RDF and
backups never contain plugin code. Applications must restore registration on open.

Validation covers multi-column rows, term types, joins and duplicate multiplicity,
OPTIONAL/UNION/FILTER/EXISTS/aggregates/subqueries, errors/limits, atomic
registration, dataset isolation, CLI parity, persistence/reopen, real HTTP and
Studio execution. The network-policy visitor also traverses Oxigraph's LATERAL
extension, preventing SERVICE from bypassing the local-only policy there.
