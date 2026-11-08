use anyhow::{Context, Result, ensure};
use rdf_store::{
    comparison,
    engine::{Dataset, format_for_path},
};
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    ensure!(args.len() == 3, "usage: compare_oracle DATA QUERY EXPECTED_JSON");
    let data = Dataset::memory()?;
    data.ingest(std::fs::File::open(&args[0])?, format_for_path(std::path::Path::new(&args[0]))?)?;
    let query = std::fs::read_to_string(&args[1])?;
    let actual = comparison::from_results(data.query(&query)?, &query)?;
    let expected = comparison::from_bytes(
        &std::fs::read(&args[2])?,
        "application/sparql-results+json",
        &query,
    )
    .context("reading oracle result")?;
    ensure!(actual == expected, "oracle mismatch for {}", args[1]);
    println!("equal: {}", args[1]);
    Ok(())
}
