//! `rdf-mcp`: serve the tools over stdio.
//!
//! Environment: `RDF_MCP_GATEWAY` (default `http://127.0.0.1:7878`, must be loopback) and
//! `RDF_MCP_TIMEOUT_SECONDS` (default 60). Logs go to stderr; stdout carries the protocol.
use anyhow::Result;
use rdf_mcp::{client::Gateway, tools::RdfMcp};
use rmcp::ServiceExt;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();
    let mut base =
        std::env::var("RDF_MCP_GATEWAY").unwrap_or_else(|_| "http://127.0.0.1:7878".into());
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--gateway" => {
                base = args.next().ok_or_else(|| anyhow::anyhow!("--gateway needs a URL"))?
            }
            "--help" | "-h" => {
                println!(
                    "rdf-mcp [--gateway http://127.0.0.1:7878]\nRead-only MCP server (stdio) for the RDF store."
                );
                return Ok(());
            }
            other => anyhow::bail!("unknown argument {other}"),
        }
    }
    let timeout =
        std::env::var("RDF_MCP_TIMEOUT_SECONDS").ok().and_then(|s| s.parse().ok()).unwrap_or(60);
    let gateway = Gateway::new(&base, Duration::from_secs(timeout))?;
    tracing::info!(gateway = %base, "rdf-mcp starting on stdio");
    let service = RdfMcp::new(gateway).serve(rmcp::transport::stdio()).await?;
    service.waiting().await?;
    Ok(())
}
