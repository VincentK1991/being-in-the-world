# Semantic search local validation — 2026-11-02

Validated on Linux/WSL with Docker Desktop (Docker 29.0.1, Compose 2.40.3),
NVIDIA RTX 4090, Qdrant 1.20.0, PyTorch 2.15.0+cu132 and
sentence-transformers 6.2.0. Model: all-MiniLM-L6-v2, revision
`1110a243fdf4706b3f48f1d95db1a4f5529b4d41`, 384 dimensions.

## Results

- Workspace regression suite: 97 passed, one explicit real-sidecar test skipped.
- Workspace Clippy, all targets, warnings denied: passed.
- Astro/TypeScript check and production build: passed.
- Real Qdrant/model integration test: passed separately on CUDA and CPU.
  Covers semantic ranking, grouped subjects, literal language tags, named-graph
  provenance, dataset isolation, database reopen, same-count RDF modifications,
  stale-index rejection, replacement generation publication, and an injected
  malformed embedding response preserving the previous active index.
- Chromium behavior test with `--semantic`: passed. Uploaded Turtle through
  Studio, rebuilt through the generic plugin HTTP endpoint, ran the semantic
  SPARQL template, and verified the car subject ranks first. Actual device was
  CUDA. Existing upload rollback, query/result rendering, mobile layout and
  shutdown/reopen checks also passed. Screenshot: `target/ui/semantic.png`.

The CPU test used a separate temporary container on port 18082 with no GPU
request. Its health endpoint reported `device: cpu`, `gpu: null`. The regular
GPU service remained available on port 18081. The temporary CPU container was
removed after validation.

## Reproduce

Follow [the plugin setup guide](../../plugins/semantic-search/README.md), then:

```sh
REQUIRE_CUDA=1 cargo test -p rdf-plugin-semantic-search --test live -- --ignored --nocapture
REQUIRE_CUDA=1 npm test --prefix scripts/ui -- --semantic

# Independent CPU validation with the same cached model:
docker compose -f sidecars/semantic/compose.yml run -d --no-deps \
  --name rdf-semantic-cpu-validation -p 127.0.0.1:18082:8081 embeddings
curl --fail http://127.0.0.1:18082/health
TEST_EMBEDDING_URL=http://127.0.0.1:18082 \
  cargo test -p rdf-plugin-semantic-search --test live -- --ignored --nocapture
docker rm -f rdf-semantic-cpu-validation
```

Wait for the CPU health endpoint to succeed before running its test. The browser
test requires the Studio production build, a current `rdftool` binary and
Playwright Chromium installed. Tests use isolated databases/collection names.

These are behavioral integration checks, not a relevance benchmark or SPARQL
standards certification. Large-corpus latency, multilingual relevance, automatic
incremental indexing and cross-store concurrent-write snapshot consistency are
not established by these tests. Freshness currently scans the indexed RDF corpus
on each search. RDF backups exclude the derived vector index; rebuild it after
restore. Qdrant crash/restart recovery was not fault-injected in this run.
