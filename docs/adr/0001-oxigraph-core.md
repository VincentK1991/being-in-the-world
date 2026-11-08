# Oxigraph-backed core

Status: accepted, implementation in progress.

This decision supersedes the custom parser, SPARQL evaluator, optimizer and
redb storage roadmap for the core release. Use Oxigraph 0.5.13, with RocksDB
enabled and network-fetching features disabled. Retain redb and legacy RDF types
only for compatibility tests and migration. Extensions remain deferred.

The application boundary is `rdf_store::engine::Dataset`. Database roots contain
a version-1 JSON catalog mapping logical dataset names to UUID directories.
Each dataset has an independent engine; named graphs remain within it.
Catalog updates are locked and atomically published. Existing redb files must
be explicitly migrated to new directories, never overwritten in place.

Normal ingestion is atomic; bulk ingestion explicitly allows partial progress.
The eventual CLI and localhost HTTP service share this boundary. Machine data
belongs on stdout and diagnostics on stderr. No remote service deployment is
part of this release.

Linux/WSL build prerequisites: Rust, a C++ compiler, Clang/libclang and CMake.
Install using `sudo apt-get update && sudo apt-get install -y build-essential clang libclang-dev cmake`.

The CLI now uses the engine adapter; persistent integration tests pass.
Do not describe the core release as complete
until migration, CLI, HTTP, conformance and operational milestones pass.
