# ADR 0004: Qdrant semantic-search plugin and local CPU/GPU embeddings

Status: accepted. User selected a local CPU/GPU model, literal-based hits retaining
RDF subject/object provenance, and an existing open-source vector index sidecar.

## Boundaries

```text
Studio / CLI -> Axum -> rdf-engine -> semantic-search plugin
                            |              |          |
                         Oxigraph       Qdrant    local embedding service
                         RDF truth      vectors   MiniLM on CPU / CUDA
```

Qdrant performs vector indexing and cosine retrieval. We do not implement an ANN
index or distance scanner. A separate local Sentence Transformers service produces
embeddings with a pinned all-MiniLM-L6-v2 revision. Its CPU and CUDA modes share a
model/profile; explicit CUDA mode fails if the GPU is unavailable. The CUDA Compose
override requests GPU access only for embeddings; Qdrant uses its normal CPU image.

The Rust plugin depends on the SDK and HTTP client, not Axum or Oxigraph. It uses
the dataset-scoped read context to select RDF and the existing property interface
to return bindings. The backend keeps generic plugin routes. Application
composition opts in using a configuration file. Normal RDF queries do not require
sidecars. The adapter exposes a generic stable dataset identity for derived-state
namespacing; vector-specific code remains in the plugin.

## Source and result semantics

Eligible content is a nonempty `xsd:string` or language-tagged literal attached to
an IRI subject. Predicate allowlists and graph scope are configurable per index.
Blank-node subjects and non-text typed literals are excluded explicitly. Matching
literal text is embedded; subject/predicate/graph IRIs are payload, not embeddings.
Language and datatype metadata are retained. The initial model targets English;
preserving language tags does not imply multilingual retrieval quality.

One point represents one RDF quad. Three properties accept `(index, text, top-k)`:

- `search:matches` returns subject, predicate, original object, and cosine score.
- `search:subjects` uses Qdrant grouping to return distinct subjects and their best
  matching literal score.
- `search:objects` uses grouping by complete RDF literal identity to return
  distinct objects and their best score.

The prefix is `urn:rdftool:search:`. Object literals can be projected directly;
grouping is over retrieved literal meaning, not whole-entity concatenation. The
JSON search operation also returns graph provenance. For scoped SPARQL results,
configure an index to a graph instead of putting the property inside GRAPH.

Long literals are split into overlapping token windows, encoded, mean-pooled and
normalized into one vector per literal. The model ID, immutable revision,
dimension, chunking and pooling policy form the embedding profile. Profiles must
match at search time; changes require rebuilding. CPU/GPU device is reported but
does not change the profile. This simple long-text pooling is not a chunk-level
retrieval index and does not promise optimal long-document relevance.

## Storage, publication and freshness

- Qdrant owns the persistent Docker volume `rdf-semantic_qdrant-data`.
- The model cache uses a separate `rdf-semantic_embedding-models` volume.
- Collections are namespaced by a hash of canonical RDF-root path + dataset UUID,
  index name and generation UUID. No logical name becomes a filesystem path.
- A manifest point records schema, namespace, configuration, model profile and
  an order-independent fingerprint of the eligible RDF corpus. A mandatory kind
  filter excludes this point from every search.
- Rebuild reads a corpus snapshot, builds a new collection in batches, verifies
  point count, and checks the source fingerprint again. It then atomically swaps
  the index alias. Failure before publication leaves the previous alias intact.
- Old generations are retained so in-flight searches against a resolved concrete
  collection remain valid. Operators can inspect/prune old generations after
  stopping readers. Crashes can leave an unreferenced staged collection; it is
  never mistaken for an active index. An ambiguous alias-publish timeout must not
  trigger deletion of a potentially published collection.
- This release uses explicit rebuilds, not synchronous RDF write hooks. Each
  search scans/fingerprints the eligible corpus and refuses stale results. Same
  quad count with changed text still invalidates an index. This O(n) freshness
  scan is a correctness-first baseline and limits end-to-end scalability despite
  Qdrant's ANN index. A durable change-feed/watermark design is the next step.
- Concurrent RDF writes can occur between fingerprint checking and final SPARQL
  execution. There is no cross-store transaction/snapshot guarantee. Rebuilds
  reject observed source changes; strict application workflows should pause
  writes during rebuild/search until snapshot/change-feed support is implemented.

## Backup and operations

RDF backups remain authoritative and do not include Qdrant or executable plugins.
After restore, reapply the semantic configuration and rebuild. Moving/copying a
root changes its derived-state namespace, also requiring rebuild. Back up Compose
volumes/Qdrant snapshots separately when avoiding re-embedding is important; do not
claim atomic backup across the two stores. Never run `compose down -v` to stop the
service if persistence is required.

Sidecar ports bind to loopback only. The Rust plugin accepts only loopback HTTP
origins, disables redirects/proxies, and bounds connect/request timeouts. Inference
occurs locally; initial model downloads contact Hugging Face. A missing/unhealthy
sidecar fails semantic operations explicitly and does not disable ordinary RDF
operations. Rebuild/status/search use generic plugin operations, not new Axum
handlers. The `rebuild` operation writes plugin-owned derived state while the
SDK's RDF read context remains read-only.

## Validation profile

Offline tests cover endpoint/config validation, namespace separation, canonical
fingerprints, literal metadata and invalid arguments. Opt-in integration tests use
real Qdrant and real CPU/CUDA model inference, assert semantic ranking, grouped
subjects/objects, stale-index rejection, generation replacement, dataset isolation
and database reopen. These are separately reported, never replaced by fake vectors.

References: [Qdrant collections and aliases](https://qdrant.tech/documentation/manage-data/collections/),
[grouped queries](https://api.qdrant.tech/api-reference/search/query-points-groups),
[Sentence Transformers CPU/GPU inference](https://sbert.net/docs/sentence_transformer/usage/efficiency.html).
