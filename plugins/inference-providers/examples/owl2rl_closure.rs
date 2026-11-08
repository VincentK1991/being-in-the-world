//! `cargo run --release -p rdf-inference-providers --example owl2rl_closure -- data.nt`: prints the
//! native OWL 2 RL closure's derived triples to stdout and its size and time to stderr.
use anyhow::{Context, Result};
use rdf_inference_providers::NativeOwlRl;
use rdf_plugin_inference::Reasoner;

fn main() -> Result<()> {
    let path = std::env::args().nth(1).context("usage: owl2rl_closure <file.nt>")?;
    let data = std::fs::read_to_string(&path)?;
    let started = std::time::Instant::now();
    let (derived, _) = NativeOwlRl::new(60, 200_000).infer(NativeOwlRl::PROFILE, &data)?;
    eprintln!(
        "{} input lines, {} derived, {:.2}s",
        data.lines().count(),
        derived.lines().count(),
        started.elapsed().as_secs_f64()
    );
    println!("{derived}");
    Ok(())
}
