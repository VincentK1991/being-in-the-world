# RDF Store (Rust Workspace)

This repository contains the phased Rust implementation of an RDF quad store defined in `docs/rdfstore-rust-build-plan.md`.

## Workspace crates

- `crates/rdf-model`
- `crates/rdf-io`
- `crates/rdf-store`
- `crates/sparql-syntax`
- `crates/sparql-eval`
- `crates/sparql-opt`
- `crates/rdf-reasoner`
- `crates/rdf-vec`
- `crates/rdf-server`
- `crates/rdf-cli` (binary: `rdftool`)
- `crates/rdf-testkit`

## Local developer loop

```bash
cargo fmt --all --check
cargo check --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo nextest run --workspace --all-targets
cargo deny check
```

Or run the same stack through `just`:

```bash
just ci
```
