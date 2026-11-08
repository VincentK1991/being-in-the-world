# Oxigraph implementation validation

Implementation is in progress; this document distinguishes observed checks from
release acceptance criteria that remain open.

## Observed

- Final workspace run: `cargo nextest run --workspace --all-targets --locked
  --offline` passed 84 tests, 0 skipped, in 28.770 seconds. Formatting,
  all-target compilation, Clippy with warnings denied, shell syntax checks,
  and `git diff --check` passed. Localhost tests ran with socket permission.

- CLI integration: parsing four RDF formats, rejection of malformed input without
  overwriting an output file, conversion, JSON stats, file/DB parity, dataset
  isolation, duplicate named-node ingestion, query/update, backup/restore, diff.
- Engine tests: joined SELECT, persistence/reopen, atomic failed ingestion and
  update, OPTIONAL/FILTER, aggregate count, paths, named graphs, result multiset
  and blank-node comparison, migration including an empty dataset with unchanged
  source bytes.
- Live loopback HTTP: query/update, malformed requests, unsupported media types,
  request-size limit, graph PUT rollback, GET/DELETE, CLI remote query, process
  kill and reopen after a committed write.
- Official W3C revision `d01b141c65ddf8e5b615256f264111118bdb2134`:
  N-Triples 70, N-Quads 87, Turtle 313, TriG 357; all passed.
  SPARQL syntax-query 97, syntax-update-1 54, syntax-update-2 1; all passed.
  These are parser checks, not proof of full query/protocol compliance.
- Independent Apache Jena 4.10.0 differential corpus: five queries passed
  (join/order, OPTIONAL with unbound values, aggregate count, duplicate rows,
  and ASK/property paths). Archive SHA-512 is pinned in the reproduction script.
- Release persistent benchmarks on Intel Core i9-14900KF, Linux/WSL:

  | Quads | Bulk ingest + flush | COUNT query | Peak RSS | Store bytes |
  | --- | --- | --- | --- | --- |
  | 100,000 | 0.217 s | 0.0143 s | 83,456 kB | 9,775,555 |
  | 1,000,000 | 1.386 s | 0.1955 s | 208,264 kB | 260,257,465 |

  Single local samples, 256 MiB loader target and two threads. The fixture has
  one predicate and 100 repeated string objects. These are reproducible
  baselines for this workload, not general database throughput promises.

## Release gates still open

- Dependency advisory check: Oxigraph 0.5.13 still transitively pins quick-xml
  0.37.5 through oxrdfxml/sparesults. RUSTSEC-2026-0194 and RUSTSEC-2026-0195
  require quick-xml >=0.41.0. No advisory ignore was added. Upstream-compatible
  remediation or a reviewed maintained patch is required before release.
  A local adapter-version bump was attempted and removed: quick-xml 0.41 changes
  namespace APIs and splits entity references into separate events, requiring
  a tested parser port rather than a dependency-only patch.
- Full SPARQL evaluation/update/result-format and protocol conformance. The
  manifest runner currently marks unsupported result fixture encodings explicitly;
  exploratory aggregate tests also exposed result-comparison differences that
  must be diagnosed. HTTP protocol dataset parameters are not implemented yet.
- Broader independent differential validation and complete crash/interruption,
  lock-contention, saturation, graceful-shutdown and three-way parity coverage.
- Clean-environment reproduction.

BSD-3-Clause, ISC and CDLA-Permissive-2.0 were added to the license allowlist for
the native binding/network dependencies. Existing dependency fixes were retained.
The full 100-test legacy count is no longer applicable: placeholder marker tests
were removed from the active workspace and CLI assertions were replaced with
semantic tests for the intentionally changed machine-readable interface.
