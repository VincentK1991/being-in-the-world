# Third-party software

This project is licensed under Apache-2.0 (see [LICENSE](LICENSE)). It builds on the software below.
Licences were read from each project's package metadata or repository on 2026-11-08. This is a summary,
not legal advice; the upstream licence texts are authoritative.

## Linked into the Rust binaries

All Rust dependencies are checked by `cargo deny check licenses` against the allow-list in
[deny.toml](deny.toml) (Apache-2.0, MIT, Unicode-3.0, BSD-3-Clause, ISC, CDLA-Permissive-2.0).
Run it after any dependency change.

| Component | Version | Licence |
| --- | --- | --- |
| [Oxigraph](https://github.com/oxigraph/oxigraph) (store, SPARQL, RDF parsers) | 0.5.13 | MIT OR Apache-2.0 (used under Apache-2.0) |
| RocksDB, via Oxigraph | | GPL-2.0 OR Apache-2.0 upstream (used under Apache-2.0) |

## Sidecars (separate processes, reached over HTTP)

These run in their own containers and are not linked into this project's code.

| Component | Version | Licence |
| --- | --- | --- |
| [Qdrant](https://github.com/qdrant/qdrant) | v1.20.0 | Apache-2.0 |
| [Ontop](https://github.com/ontop/ontop) (`ontop/ontop-endpoint`) | pinned by digest | Apache-2.0 |
| PostgreSQL | 16 | PostgreSQL License (permissive) |
| PostgreSQL JDBC driver (`postgresql.jar`, downloaded locally, not committed) | | BSD-2-Clause |
| [Comunica](https://github.com/comunica/comunica) `@comunica/query-sparql` | 5.4.1 | MIT |
| n3, sparqljs (npm) | ^1.26.0, 3.7.4 | MIT |
| [Apache Jena](https://jena.apache.org/) `jena-arq`, `jena-shacl` | 6.2.0 | Apache-2.0 |
| Gson | 2.13.2 | Apache-2.0 |
| SLF4J (`slf4j-nop`) | 2.0.17 | MIT |
| [owlrl](https://github.com/RDFLib/OWL-RL) | 7.6.2 | W3C Software and Document Notice and License (permissive) |
| [rdflib](https://github.com/RDFLib/rdflib) | 7.6.0 | BSD-3-Clause |
| [sentence-transformers](https://github.com/UKPLab/sentence-transformers) | 6.2.0 | Apache-2.0 |
| `sentence-transformers/all-MiniLM-L6-v2` (embedding model, downloaded at runtime) | | Apache-2.0 (per its model card; not verified here) |

Base images (`python`, `node`, `eclipse-temurin`, `maven`, `pytorch/pytorch`) carry their own licences,
including GPL components in the operating system layers. They are pulled by the user and are not
redistributed by this repository.

## Names

Oxigraph, Qdrant, Ontop, Comunica, Apache Jena and the other names above belong to their owners. This
project is built on or integrates with them and is not endorsed by them.
