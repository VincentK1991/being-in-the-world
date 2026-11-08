use anyhow::{Result, ensure};
use rdf_store::{
    engine::Dataset,
    oxigraph::{io::RdfFormat, sparql::QueryResults},
};
use std::{
    fs::{self, File},
    io::{BufWriter, Write},
    path::Path,
    time::Instant,
};
fn bytes(path: &Path) -> Result<u64> {
    let mut total = 0;
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        total += if entry.file_type()?.is_dir() {
            bytes(&entry.path())?
        } else {
            entry.metadata()?.len()
        };
    }
    Ok(total)
}
fn main() -> Result<()> {
    let count: usize = std::env::args().nth(1).unwrap_or_else(|| "100000".into()).parse()?;
    ensure!([100_000, 1_000_000].contains(&count), "use 100000 or 1000000");
    let temp = tempfile::tempdir()?;
    let input = temp.path().join("data.nt");
    {
        let mut output = BufWriter::new(File::create(&input)?);
        for i in 0..count {
            writeln!(output, "<urn:s:{i}> <urn:p> \"{}\" .", i % 100)?;
        }
        output.flush()?;
    }
    let root = temp.path().join("db");
    let data = Dataset::open(&root, "benchmark", true)?;
    let start = Instant::now();
    data.ingest_bulk(File::open(input)?, RdfFormat::NTriples, 256, 2)?;
    data.store().flush()?;
    let ingest = start.elapsed();
    let query = Instant::now();
    let QueryResults::Solutions(rows) =
        data.query("SELECT (COUNT(*) AS ?count) WHERE { ?s <urn:p> ?o }")?
    else {
        anyhow::bail!("expected rows")
    };
    for row in rows {
        ensure!(row?["count"].to_string().starts_with(&format!("\"{count}\"")), "wrong count");
    }
    let query_elapsed = query.elapsed();
    let status = fs::read_to_string("/proc/self/status")?;
    let peak = status.lines().find(|line| line.starts_with("VmHWM:")).unwrap_or("unknown");
    let cpu = fs::read_to_string("/proc/cpuinfo")?;
    println!(
        "{}",
        serde_json::json!({"quads": count, "ingest_seconds": ingest.as_secs_f64(), "query_seconds": query_elapsed.as_secs_f64(), "store_bytes": bytes(&root)?, "peak_memory":peak, "cpu":cpu.lines().find(|line| line.starts_with("model name")).unwrap_or("unknown")})
    );
    Ok(())
}
