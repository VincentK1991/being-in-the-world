# Local semantic search with Qdrant

Requires Docker Compose. CPU inference works without a GPU. CUDA inference requires
an NVIDIA driver and Docker GPU access (Docker Desktop WSL integration on Windows).
The first build/model load downloads container layers and model files.

From the repository root, start **one** of these configurations:

```sh
# CPU
docker compose -f sidecars/semantic/compose.yml up --build -d

# NVIDIA GPU (explicit CUDA; fails instead of silently falling back)
docker compose -f sidecars/semantic/compose.yml \
  -f sidecars/semantic/compose.gpu.yml up --build -d
```

Inspect readiness and the actual inference device:

```sh
curl http://127.0.0.1:16333/healthz
curl http://127.0.0.1:18081/health
```

The embedding response reports `device`, GPU name, model and pinned revision.
Use the CPU configuration to switch back when CUDA is unavailable. The service
also supports `EMBEDDING_DEVICE=auto` for automatic GPU/CPU selection, but explicit
CUDA is used in the GPU test to prove inference actually runs there.

## Ingest, index, query

```sh
cargo build --locked -p rdf-cli
target/debug/rdftool ingest --from plugins/semantic-search/example.ttl --to ./semantic-db

export RDF_SEMANTIC_CONFIG="$PWD/plugins/semantic-search/example-config.json"
target/debug/rdftool plugin --source ./semantic-db --plugin semantic-search \
  --operation rebuild --input '{"index":"descriptions"}'
target/debug/rdftool plugin --source ./semantic-db --plugin semantic-search \
  --operation status --input '{"index":"descriptions"}'

target/debug/rdftool query --source ./semantic-db '
PREFIX search: <urn:rdftool:search:>
SELECT ?subject ?object ?score WHERE {
  (?subject ?predicate ?object ?score)
    search:matches ("descriptions" "battery powered vehicles" 10) .
} ORDER BY DESC(?score)'
```

`--semantic-config PATH` is an alternative to the environment variable. Configuration
is loaded on startup and is not persisted inside the RDF store. Semantic indexing
requires a persistent dataset; transient RDF-file query sources are not supported.

For the UI, build Studio as described in the root README, then start:

```sh
target/debug/rdftool serve --db ./semantic-db --bind 127.0.0.1:7878
```

Open http://127.0.0.1:7878 and choose **Semantic search · descriptions**. Uploads and
updates that change indexed content invalidate the index; rebuild through HTTP
while the server owns the database:

```sh
target/debug/rdftool plugin --source http://127.0.0.1:7878 --plugin semantic-search \
  --operation rebuild --input '{"index":"descriptions"}'
```

The same operation is `POST /api/plugins/semantic-search/operations/rebuild` with
a JSON body. Do not open the database directly from a second process while serving.

## Retrieve subjects or objects

```sparql
PREFIX search: <urn:rdftool:search:>
SELECT ?subject ?score WHERE {
  (?subject ?score) search:subjects ("descriptions" "clean transportation" 10) .
} ORDER BY DESC(?score)
```

```sparql
PREFIX search: <urn:rdftool:search:>
SELECT ?object ?score WHERE {
  (?object ?score) search:objects ("descriptions" "clean transportation" 10) .
} ORDER BY DESC(?score)
```

Qdrant groups subjects/objects and uses the best matching literal score per group.
`matches` retains separate hits for separate quads. Scores are cosine similarities
(`xsd:double`), higher is better, not probabilities. Add a SPARQL score filter if
needed; it filters the top-k candidates, it does not trigger another retrieval.
Use `ORDER BY` explicitly. The model is English-oriented; language tags are
preserved but multilingual relevance is not guaranteed.

The generic `search` operation returns full JSON provenance including graph:

```sh
target/debug/rdftool plugin --source http://127.0.0.1:7878 --plugin semantic-search \
  --operation search --input '{"index":"descriptions","text":"clean transportation","limit":10,"mode":"matches"}'
```

## Configuration and persistence

Each configured index has:

- `name`: unique name used in property arguments.
- `predicates`: allowed text predicates; an empty list indexes all text predicates.
- `graph`: omitted/null for all graphs, `"default"` for the default graph, or an IRI
  for one named graph. Property calls themselves remain outside SPARQL GRAPH.
- `max_entries`: maximum eligible rows allowed during a rebuild/freshness scan;
  default 100,000, configurable up to 1,000,000. This is a guard, not a performance claim.

Only nonempty string/language literals on IRI subjects are indexed. Numbers and
blank-node subjects are excluded. Original literal datatype/language and graph
provenance are stored in payload. Literal size is capped at 64 KiB; longer model
inputs are windowed and pooled, rather than silently truncating their tails.
Top-k is 1–100. All property inputs must be constants under the existing SDK profile.

Vectors and metadata persist in `rdf-semantic_qdrant-data`; the model cache is in
`rdf-semantic_embedding-models`. Rebuilds publish a fresh collection through an
atomic alias switch. Failed rebuilds preserve the previous active index. Old
generations remain for in-flight readers and can be pruned manually after readers
stop; an interrupted rebuild can leave an unused staged collection.

Freshness uses an exact content fingerprint of the configured corpus on every
search. Stale indexes produce an error, never silently serve old hits. This adds
an O(n) RDF scan; incremental indexing/change-feed support is deferred. There is
no cross-store transaction with concurrent RDF writes. Model/profile/configuration
changes require rebuilds. RDF backup/restore excludes vectors: preserve the config
and rebuild after restore. Moving/copying the RDF root also creates a new index
namespace. Use Qdrant snapshots/volume backups separately if needed.

Stop without deleting data:

```sh
docker compose -f sidecars/semantic/compose.yml stop
```

## Tests

```sh
cargo test -p rdf-plugin-semantic-search
# Real sidecars must be ready; this is not a mock embedding test.
REQUIRE_CUDA=1 cargo test -p rdf-plugin-semantic-search --test live semantic_search_rebuild_staleness_isolation_and_reopen -- --ignored --nocapture
# CPU uses the same test without REQUIRE_CUDA=1.
# Browser test also uploads RDF, rebuilds through HTTP, and runs the Studio template:
REQUIRE_CUDA=1 npm test --prefix scripts/ui -- --semantic
```

`TEST_QDRANT_URL` and `TEST_EMBEDDING_URL` override test endpoints. Tests use unique
collection namespaces and clean up their own completed collections. They do not
modify user indexes. See [the architecture decision](../../docs/adr/0004-semantic-search-sidecars.md)
for lifecycle, failure and consistency semantics.

## Multiple embedding providers

See [example-providers.json](example-providers.json) for simultaneous local and
OpenAI indexes. `providers` contains named instances; each index selects one:

```json
{
  "qdrant_url": "http://127.0.0.1:16333",
  "providers": {
    "local_gpu": {"kind": "local", "config": {"url": "http://127.0.0.1:18081"}},
    "openai_small": {"kind": "openai", "config": {
      "base_url": "https://api.openai.com/v1",
      "model": "text-embedding-3-small",
      "dimensions": 1536,
      "api_key_env": "OPENAI_API_KEY",
      "revision": "deployment-1"
    }}
  },
  "indexes": [
    {"name": "descriptions", "provider": "local_gpu", "predicates": ["http://www.w3.org/2000/01/rdf-schema#comment"]},
    {"name": "descriptions_openai", "provider": "openai_small", "predicates": ["http://www.w3.org/2000/01/rdf-schema#comment"]}
  ]
}
```

Set `OPENAI_API_KEY` securely in the environment of the CLI/server; never put its
value in this file. The hosted index sends its literal text and search text to the
configured provider and uses that account's API billing. Local indexes do not need
this key. Start the server with the same configuration used when building indexes.

```sh
export RDF_SEMANTIC_CONFIG="$PWD/plugins/semantic-search/example-providers.json"
# With OPENAI_API_KEY already set:
target/debug/rdftool plugin --source ./semantic-db --plugin semantic-search \
  --operation rebuild --input '{"index":"descriptions_openai"}'
```

Change only the index string in SPARQL to search this index. Keep scores from
different models separate: they are not calibrated for cross-model comparison.
`09-two-providers.rq` returns labelled results from both indexes using UNION.

New configuration remains local by default. The legacy `embedding_url` format is
accepted, but cannot be mixed with the new format. Existing pre-provider indexes
use an older manifest and need a rebuild once; no RDF migration is required.

OpenAI-compatible endpoints may override `base_url` and `api_key_env`. They must
accept explicit dimensions, float encoding and Bearer auth and return model IDs
and indexed vectors. Other API contracts need their own provider adapter.
Hosted inputs over `max_input_bytes` (default/max 8000) are rejected, never silently
truncated; local long-input windowing remains available. Errors do not trigger
fallback or automatic retries. Change the operator `revision` label and rebuild
when changing a remotely managed model behind a stable alias. `status` checks
index metadata/freshness, not hosted API connectivity or credentials.

For the extension interface, registration example and boundaries, see
[embedding providers](../embedding-providers/README.md) and
[ADR 0005](../../docs/adr/0005-embedding-provider-extensions.md).

## Executed SPARQL examples

[Queries and recorded results](examples/README.md) cover ranking, joins/filters,
literal objects, OPTIONAL, aggregation, UNION, ASK and CONSTRUCT.

```sh
# Requires local sidecars; creates target/semantic-provider-demo/db and result files.
python3 plugins/semantic-search/examples/run.py
# Choose a new output directory for each run. Explicitly opt into OpenAI calls:
python3 plugins/semantic-search/examples/run.py --openai --output target/semantic-hosted-demo
```

Opt-in integration tests (choose the exact test; `--ignored` alone also runs hosted tests):

```sh
REQUIRE_CUDA=1 cargo test -p rdf-plugin-semantic-search --test live \
  semantic_search_rebuild_staleness_isolation_and_reopen -- --ignored
cargo test -p rdf-plugin-semantic-search --test live \
  mixed_indexes_with_openai_contract_fixture -- --ignored
# Real hosted test: requires OPENAI_API_KEY, sends only fixture text to OpenAI.
cargo test -p rdf-plugin-semantic-search --test live \
  local_and_openai_indexes_in_one_query -- --ignored
```

## Hybrid and keyword search

Set `"mode": "hybrid"` on an index (see `descriptions-hybrid` in `example-config.json`) and rebuild it. The index then
also stores BM25 term vectors, which enables two more SPARQL properties alongside `search:matches`:

```sparql
PREFIX search: <urn:rdftool:search:>
SELECT ?subject ?object ?score WHERE {
  (?subject ?predicate ?object ?score) search:lexical ("descriptions-hybrid" "city powered" 10)   # exact words, BM25
}
SELECT ?subject ?object ?score WHERE {
  (?subject ?predicate ?object ?score) search:hybrid ("descriptions-hybrid" "battery vehicle" 10)  # meaning + keywords, RRF
}
```

Keyword matching is whole-word and case-insensitive with no stemming. Hybrid scores are rank-based. See
[ADR 0009](../../docs/adr/0009-hybrid-search.md). Live test:
`cargo test -p rdf-plugin-semantic-search --test live -- --ignored`.
