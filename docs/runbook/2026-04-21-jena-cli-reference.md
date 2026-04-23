---
updated_at: 2026-04-21T23:34:15-0400
status: completed
---

# Jena CLI Reference Documentation

## Summary
- Added `docs/jena-cli/` folder for Apache Jena CLI documentation.
- Added `docs/jena-cli/command-reference.md` with:
  - 31 top-level Jena command entries grouped by category
  - short purpose of each command
  - explicit “independent check” usage for our RDF store development
  - ARQ lower-level command supplement (`arq.query`, `arq.update`, etc.)
  - practical parity-testing playbook (parser/query/update/ingest/export/IRI checks)
  - links to official Jena docs used as source references
- Added `docs/jena-cli/README.md` as folder entrypoint.

## Outcome
- Team now has a centralized command-by-command reference to use Jena as an external oracle during feature development and regression checks.
