# ADR 0009: Hybrid (dense + lexical) search in the semantic-search plugin

Status: accepted. Extends ADR 0004; no new plugin, sidecar or SDK version.

## Context

Dense embeddings find meaning but miss exact tokens such as reference numbers, names and codes.
Qdrant supports sparse vectors with a server-side inverse-document-frequency modifier, and the
Query API can fuse several ranked lists. Both are available in the Qdrant version we run.

## Decision

An index gains `"mode": "dense" | "hybrid"` (default `dense`). A `hybrid` index stores, per literal:

- a named dense vector `dense` (the existing embedding), and
- a named sparse vector `lexical` declared with `modifier: idf`, so Qdrant supplies the IDF half of BM25.

The plugin supplies the term-frequency half itself, with no sidecar change: lowercase alphanumeric
tokens hashed to stable `u32` term ids, weighted `tf*(k1+1) / (tf + k1*(1-b+b*len/avglen))` with
`k1 = 1.2`, `b = 0.75`; queries weight each distinct term 1. There is no stemming or stop-word list,
so the tokenizer is language-neutral and "battery" does not match "batteries".

New SPARQL property functions, same shape as `search:matches` (`(?s ?p ?o ?score) search:X (index text top-k)`):

| Property | Ranking | Score |
| --- | --- | --- |
| `search:lexical` | BM25 over the sparse vector | keyword score, higher is better |
| `search:hybrid` | dense and lexical lists fused with reciprocal rank fusion | rank-based; not comparable across queries |

`search:matches`, `subjects` and `objects` keep their dense behaviour on both index modes. `lexical` and
`hybrid` on a dense index fail with a message to set `mode` and rebuild. If a hybrid query has no
searchable words it falls back to its dense branch; `lexical` rejects it.

## Compatibility

`mode` is omitted when serializing a dense index, so existing indexes keep their configuration
digest and do not go stale. Changing an index to `hybrid` changes the digest and requires a rebuild,
which is the normal staleness path. The collection layout differs between modes (unnamed vs named
vectors), so there is no in-place upgrade.

## Consequences

- Exact-term queries work without relying on the embedding model; semantic queries are unchanged.
- Rebuild embeds and tokenizes every literal; the sparse side adds little cost.
- Fusion scores are ranks, so score thresholds that suit cosine scores do not apply to `hybrid`.
- Not done: stemming, phrase queries, per-index tokenizer settings, grouped (`subjects`/`objects`) hybrid variants.

## Tests

`plugins/semantic-search/tests/live.rs` runs against real Qdrant and the embedding sidecar and checks, through
SPARQL, that a keyword-only identifier is found by `lexical` and `hybrid` but not guaranteed by dense search,
that hybrid keeps the semantic hit, that dense search is unchanged on both modes, and that lexical search on a
dense index is refused. Unit tests cover the tokenizer, BM25 weights and the digest compatibility.
