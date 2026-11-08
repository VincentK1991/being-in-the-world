# ADR 0005: Per-index embedding provider extensions

Status: accepted and implemented, 2026-11-02.

## Intent and boundary

Each semantic index chooses a named embedding provider in a JSON configuration.
The database core and SPARQL property syntax stay independent of provider APIs.
Local CPU/GPU inference and hosted OpenAI embeddings can coexist in one dataset.
This supersedes ADR 0004's local-only embedding restriction, not its local-only
Qdrant/server bindings or prohibition of arbitrary SPARQL network fetching.

```
Studio / CLI -> rdf-server / application -> rdf-engine -> semantic-search plugin
                                                        |             |
                                              rdf-embedding-api     Qdrant
                                                        |
                                      registered embedding providers
                                        /                     \
                              local sidecar               OpenAI API
```

`rdf-embedding-api` defines `EmbeddingProvider`, `ProviderFactory`, `Profile` and
`ProviderRegistry`. It depends on neither HTTP nor the RDF engine. The semantic
plugin depends on this contract and receives a registry from the application.
`rdf-embedding-providers` supplies the initial `local` and `openai` factories.
`rdf-cli` is the composition root that registers the built-ins.

A new provider can live in its own Rust crate, implement the factory/provider
traits, and be registered in the application. No edits to semantic-search,
rdf-engine or rdf-store are needed. These are compiled Rust extensions, not
runtime-loaded native libraries: registering a new kind requires rebuilding the
application. Choosing among already registered providers requires only config.
There is no provider selection UI in this release.

## Configuration and secrets

`providers` maps instance names to `{kind, config}`. Each index specifies a
`provider`. Provider-specific configuration is parsed by its factory; the semantic
plugin does not enumerate provider kinds. Unknown kinds, missing references and
mixed legacy/new configuration fail at startup. Secrets are read only from the
configured environment variable at request time, never stored in the vector
manifest, RDF, or config. Missing OpenAI credentials do not prevent local indexes
from operating. Only operations using the hosted index submit text remotely.

The old `embedding_url` config is still accepted exclusively as a legacy local
configuration. It cannot be combined with `providers` or per-index provider names.

## Identity, lifecycle and failure semantics

Manifest schema 2 stores a generic embedding profile. Provider implementation,
endpoint, model, dimensions, revision and preprocessing identity must match on
search. Credentials and CPU/GPU device are not vector-space identity. Existing
schema 1 indexes require an explicit rebuild; RDF and old generations remain
untouched. A different endpoint is conservatively treated as a different space.

Local model revisions come from the sidecar. Hosted `revision` is an explicit
operator deployment label, not a claim that a model alias is immutable. Bump it
and rebuild when a hosted model changes behind a stable name. Undetectable drift
behind an unchanged remote alias remains a limitation. Response model IDs and
vector dimensions are checked, but those cannot establish unchanged weights.

Document and query text use the same adapter/profile. OpenAI inputs are passed
unchanged, with no silent truncation. The initial adapter rejects inputs over a
configurable maximum of 8000 UTF-8 bytes and uses at most 32 inputs per request.
This conservative byte guard is not a tokenizer; provider errors still propagate.
Local token-window pooling remains unchanged. Long-text hosted chunking is deferred.

The OpenAI-compatible adapter uses `/embeddings`, float encoding, explicit model
and dimensions, and Bearer authentication. It restores response order using each
item's index and rejects missing/duplicate indexes, wrong model, invalid dimensions,
non-finite and zero vectors. Compatible endpoints must support this contract;
provider-specific auth/schema differences require a separate adapter. HTTP is
allowed only on loopback; other endpoints require HTTPS. Redirects are disabled.
Failures report status without response bodies to avoid echoing credentials/text.
There are no automatic retries or implicit local fallback.

A hosted provider's status metadata does not contact the API or verify credentials.
`ready` means index compatibility/freshness, not live API availability. The actual
embedding call checks credentials and availability. Rebuild failure keeps the
previous alias intact; queries never silently switch embedding spaces. Qdrant
retains vectors; the embedding provider is not the vector database.

## Validation

Provider HTTP contract tests cover serialization, reordered results, malformed
responses, HTTP failures, credential absence and input policy. An externally
registered test provider proves the semantic plugin has no built-in type switch.
Real Qdrant/local CUDA tests cover ranking, RDF provenance, staleness and reopen.
A mixed-index integration test combines real local embeddings with a separately
labelled OpenAI HTTP fixture. A separate opt-in test requires real OpenAI credentials
and checks both live providers in one query. Fixture results are not hosted model
relevance evidence.

The request contract was checked against the
[official OpenAI embeddings API reference](https://developers.openai.com/api/reference/resources/embeddings/methods/create).
