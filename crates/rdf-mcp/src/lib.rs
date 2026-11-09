//! Read-only MCP server for the RDF store. It talks to the gateway over loopback HTTP and never
//! opens database files; see `README.md`.
pub mod client;
pub mod context;
pub mod render;
pub mod sparql;
pub mod tools;
