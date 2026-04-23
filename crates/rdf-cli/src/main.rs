use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand, ValueEnum};
use rdf_io::{
    parse_nquads, parse_ntriples, parse_trig, parse_turtle, serialize_nquads_to_string,
    serialize_ntriples_to_string, serialize_trig_to_string, serialize_turtle_to_string,
};
use rdf_model::{
    BlankNode, GraphName, Literal, NamedNode, Object, Predicate, Quad, Subject, Triple,
};
use rdf_store::{MemQuadStorage, QuadPattern, RedbQuadStorage};
use rdf_testkit::manifest::{ManifestStatus, run_manifest};
use std::fs;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use tracing::info;

const RDF_TYPE_IRI: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const RDF_PREFIX: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";
const RDFS_PREFIX: &str = "http://www.w3.org/2000/01/rdf-schema#";
const XSD_PREFIX: &str = "http://www.w3.org/2001/XMLSchema#";

#[derive(Debug, Parser)]
#[command(name = "rdftool", version, about = "RDF store development CLI", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Ingest RDF data into an output target.
    Ingest {
        /// Input RDF file path.
        #[arg(long)]
        from: PathBuf,
        /// Output location (file or db path).
        #[arg(long)]
        to: PathBuf,
        /// Logical dataset name when using a DB target/source.
        #[arg(long, default_value = "default")]
        dataset: String,
    },
    /// Execute a SPARQL query against a source.
    Query {
        /// Source selector: file path, db path, or endpoint URL.
        #[arg(long)]
        source: String,
        /// SPARQL query to execute.
        query: String,
    },
    /// Execute a single quad pattern against a source.
    QueryPattern {
        /// Source selector (Phase 3: file path).
        #[arg(long)]
        source: String,
        /// Logical dataset name when using a DB target/source.
        #[arg(long, default_value = "default")]
        dataset: String,
        /// Subject term, e.g. `<http://ex/s>`, `_:b1`, or variable `?x`.
        #[arg(long)]
        subject: Option<String>,
        /// Predicate term, e.g. `<http://ex/p>`, `rdf:type`, or variable `?p`.
        #[arg(long)]
        predicate: Option<String>,
        /// Object term, e.g. `<http://ex/o>`, `"literal"`, or variable `?o`.
        #[arg(long)]
        object: Option<String>,
        /// Graph term, e.g. `<http://ex/g>`, `default`, `_:g`, or variable `?g`.
        #[arg(long)]
        graph: Option<String>,
    },
    /// Dump a source in the requested RDF serialization.
    Dump {
        /// Source selector: file path, db path, or endpoint URL.
        #[arg(long)]
        source: String,
        /// Logical dataset name when using a DB target/source.
        #[arg(long, default_value = "default")]
        dataset: String,
        /// Output format.
        #[arg(long, value_enum, default_value_t = OutputFormat::NQuads)]
        format: OutputFormat,
    },
    /// Print source-level quad and term cardinalities.
    Stats {
        /// Source selector (Phase 3: file path).
        #[arg(long)]
        source: String,
        /// Logical dataset name when using a DB target/source.
        #[arg(long, default_value = "default")]
        dataset: String,
    },
    /// Compare query outputs across multiple sources.
    Diff {
        /// Query file to execute.
        #[arg(long)]
        query: PathBuf,
        /// Comma-separated sources to compare.
        #[arg(long, value_delimiter = ',', num_args = 1..)]
        sources: Vec<String>,
    },
    /// Enumerate tests from a W3C manifest and report smoke-runner statuses.
    TestSuite {
        /// Manifest root file or directory.
        #[arg(long)]
        manifest: PathBuf,
        /// Source mode, e.g. `fresh`, `file`, `db`.
        #[arg(long, default_value = "fresh")]
        source: String,
    },
    /// Start a local HTTP server exposing RDF endpoints.
    Serve {
        /// Bind address for the HTTP server.
        #[arg(long, default_value = "127.0.0.1:7878")]
        bind: SocketAddr,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum OutputFormat {
    NQuads,
    NTriples,
    Turtle,
    Trig,
}

fn main() -> Result<()> {
    init_tracing();
    let cli = Cli::parse();
    run(cli)
}

fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Command::Ingest { from, to, dataset } => {
            info!(from = %from.display(), to = %to.display(), "ingest command requested");
            let input = fs::read_to_string(&from)
                .with_context(|| format!("failed to read {}", from.display()))?;
            let extension = from.extension().and_then(|ext| ext.to_str()).unwrap_or_default();

            match extension {
                "nt" => {
                    let triples = parse_ntriples(&input).map_err(|error| {
                        anyhow::anyhow!(
                            "ntriples parse failed: line={} column={} kind={}",
                            error.line,
                            error.column,
                            error.kind
                        )
                    })?;
                    let quads = triples_to_default_graph(&triples);
                    let (target_type, written) = write_quads_to_target(&to, &dataset, &quads)?;
                    println!(
                        "ingest: source_format=ntriples target_type={} from={} to={} parsed_triples={} written_quads={}",
                        target_type,
                        from.display(),
                        to.display(),
                        triples.len(),
                        written
                    );
                }
                "nq" => {
                    let quads = parse_nquads(&input).map_err(|error| {
                        anyhow::anyhow!(
                            "nquads parse failed: line={} column={} kind={}",
                            error.line,
                            error.column,
                            error.kind
                        )
                    })?;
                    let (target_type, written) = write_quads_to_target(&to, &dataset, &quads)?;
                    println!(
                        "ingest: source_format=nquads target_type={} from={} to={} parsed_quads={} written_quads={}",
                        target_type,
                        from.display(),
                        to.display(),
                        quads.len(),
                        written
                    );
                }
                "ttl" => {
                    let quads = parse_turtle(&input).map_err(|error| {
                        anyhow::anyhow!(
                            "turtle parse failed: line={} column={} kind={}",
                            error.line,
                            error.column,
                            error.kind
                        )
                    })?;
                    let (target_type, written) = write_quads_to_target(&to, &dataset, &quads)?;
                    println!(
                        "ingest: source_format=turtle target_type={} from={} to={} parsed_quads={} written_quads={}",
                        target_type,
                        from.display(),
                        to.display(),
                        quads.len(),
                        written
                    );
                }
                "trig" => {
                    let quads = parse_trig(&input).map_err(|error| {
                        anyhow::anyhow!(
                            "trig parse failed: line={} column={} kind={}",
                            error.line,
                            error.column,
                            error.kind
                        )
                    })?;
                    let (target_type, written) = write_quads_to_target(&to, &dataset, &quads)?;
                    println!(
                        "ingest: source_format=trig target_type={} from={} to={} parsed_quads={} written_quads={}",
                        target_type,
                        from.display(),
                        to.display(),
                        quads.len(),
                        written
                    );
                }
                _ => bail!(
                    "unsupported ingest input format for {}: expected one of .nt, .nq, .ttl, .trig",
                    from.display()
                ),
            }
        }
        Command::Query { source, query } => {
            info!(%source, "query command requested");
            println!("query: source={source}\n{query}");
        }
        Command::QueryPattern { source, dataset, subject, predicate, object, graph } => {
            info!(%source, "query-pattern command requested");
            let pattern = QuadPattern {
                subject: subject.as_deref().map(parse_subject_selector).transpose()?.flatten(),
                predicate: predicate
                    .as_deref()
                    .map(parse_predicate_selector)
                    .transpose()?
                    .flatten(),
                object: object.as_deref().map(parse_object_selector).transpose()?.flatten(),
                graph_name: graph.as_deref().map(parse_graph_selector).transpose()?.flatten(),
            };

            let matches = query_pattern_on_source(&source, &dataset, &pattern)?;
            println!("query-pattern: source={source} matches={}", matches.len());
            if !matches.is_empty() {
                let serialized = serialize_nquads_to_string(&matches)
                    .context("failed to serialize query-pattern results")?;
                print!("{serialized}");
            }
        }
        Command::Dump { source, dataset, format } => {
            info!(%source, ?format, "dump command requested");
            let quads = load_quads_from_source(&source, &dataset)?;
            let serialized = serialize_for_format(format, &quads)?;
            println!("dump: source={} format={} quads={}", source, format.as_str(), quads.len());
            print!("{serialized}");
        }
        Command::Stats { source, dataset } => {
            info!(%source, "stats command requested");
            let stats = stats_for_source(&source, &dataset)?;
            println!(
                "stats: source={} quads={} distinct_subjects={} distinct_predicates={} distinct_objects={} distinct_graphs={}",
                source,
                stats.quads,
                stats.distinct_subjects,
                stats.distinct_predicates,
                stats.distinct_objects,
                stats.distinct_graphs
            );
        }
        Command::Diff { query, sources } => {
            info!(query = %query.display(), source_count = sources.len(), "diff command requested");
            println!("diff: query={} sources={}", query.display(), sources.join(","));
        }
        Command::TestSuite { manifest, source } => {
            info!(manifest = %manifest.display(), source = %source, "test-suite command requested");
            let results = run_manifest(&manifest)?;
            let pending =
                results.iter().filter(|item| item.status == ManifestStatus::Pending).count();
            let passed =
                results.iter().filter(|item| item.status == ManifestStatus::Passed).count();
            let failed =
                results.iter().filter(|item| item.status == ManifestStatus::Failed).count();
            println!(
                "test-suite: source={} manifest={} discovered={} passed={} failed={} pending={}",
                source,
                manifest.display(),
                results.len(),
                passed,
                failed,
                pending
            );
            for case in results.iter().take(10) {
                println!("{}\t{}", status_label(case.status), case.case.id);
            }
            for case in results.iter().filter(|item| item.status == ManifestStatus::Failed).take(5)
            {
                if let Some(details) = &case.details {
                    println!("failed-detail\t{}\t{}", case.case.id, details);
                }
            }
            if results.len() > 10 {
                println!(
                    "{}\t... {} more",
                    status_label(ManifestStatus::Pending),
                    results.len() - 10
                );
            }
        }
        Command::Serve { bind } => {
            info!(%bind, "serve command requested");
            println!("serve: planned listener on {bind}");
        }
    }

    Ok(())
}

fn load_quads_from_source(source: &str, dataset: &str) -> Result<Vec<Quad>> {
    let path = Path::new(source);
    if !is_rdf_file_path(path) {
        let store = RedbQuadStorage::open(path, dataset)
            .with_context(|| format!("failed to open DB source {}", path.display()))?;
        return store.all_quads().map_err(anyhow::Error::from);
    }

    let input = fs::read_to_string(path)
        .with_context(|| format!("failed to read source file {}", path.display()))?;
    let extension = path.extension().and_then(|ext| ext.to_str()).unwrap_or_default();

    match extension {
        "nt" => parse_ntriples(&input).map(|triples| triples_to_default_graph(&triples)).map_err(
            |error| {
                anyhow::anyhow!(
                    "ntriples parse failed for {}: line={} column={} kind={}",
                    path.display(),
                    error.line,
                    error.column,
                    error.kind
                )
            },
        ),
        "nq" => parse_nquads(&input).map_err(|error| {
            anyhow::anyhow!(
                "nquads parse failed for {}: line={} column={} kind={}",
                path.display(),
                error.line,
                error.column,
                error.kind
            )
        }),
        "ttl" => parse_turtle(&input).map_err(|error| {
            anyhow::anyhow!(
                "turtle parse failed for {}: line={} column={} kind={}",
                path.display(),
                error.line,
                error.column,
                error.kind
            )
        }),
        "trig" => parse_trig(&input).map_err(|error| {
            anyhow::anyhow!(
                "trig parse failed for {}: line={} column={} kind={}",
                path.display(),
                error.line,
                error.column,
                error.kind
            )
        }),
        _ => bail!(
            "unsupported source format for {}: expected one of .nt, .nq, .ttl, .trig",
            path.display()
        ),
    }
}

fn write_quads_to_target(
    path: &Path,
    dataset: &str,
    quads: &[Quad],
) -> Result<(&'static str, usize)> {
    if !is_rdf_file_path(path) {
        let mut store = RedbQuadStorage::open(path, dataset)
            .with_context(|| format!("failed to open DB target {}", path.display()))?;
        let mut written = 0_usize;
        for quad in quads {
            if store
                .insert(quad.clone())
                .with_context(|| format!("failed to persist quad into {}", path.display()))?
            {
                written += 1;
            }
        }
        return Ok(("db", written));
    }

    let extension = path.extension().and_then(|ext| ext.to_str()).unwrap_or_default();
    let serialized = serialize_for_extension(extension, quads)?;
    fs::write(path, serialized)
        .with_context(|| format!("failed to write converted data to {}", path.display()))?;
    Ok(("file", quads.len()))
}

fn query_pattern_on_source(
    source: &str,
    dataset: &str,
    pattern: &QuadPattern,
) -> Result<Vec<Quad>> {
    let path = Path::new(source);
    if !is_rdf_file_path(path) {
        let store = RedbQuadStorage::open(path, dataset)
            .with_context(|| format!("failed to open DB source {}", path.display()))?;
        return store.quads_matching(pattern).map_err(anyhow::Error::from);
    }

    let quads = load_quads_from_source(source, dataset)?;
    let mut store = MemQuadStorage::new();
    for quad in quads {
        store
            .insert(quad)
            .map_err(|error| anyhow::anyhow!("failed to insert quad into memory store: {error}"))?;
    }
    Ok(store.quads_matching(pattern))
}

fn stats_for_source(source: &str, dataset: &str) -> Result<rdf_store::StoreStats> {
    let path = Path::new(source);
    if !is_rdf_file_path(path) {
        let store = RedbQuadStorage::open(path, dataset)
            .with_context(|| format!("failed to open DB source {}", path.display()))?;
        return store.stats().map_err(anyhow::Error::from);
    }

    let quads = load_quads_from_source(source, dataset)?;
    let mut store = MemQuadStorage::new();
    for quad in quads {
        store
            .insert(quad)
            .map_err(|error| anyhow::anyhow!("failed to insert quad into memory store: {error}"))?;
    }
    Ok(store.stats())
}

fn is_rdf_file_path(path: &Path) -> bool {
    matches!(path.extension().and_then(|ext| ext.to_str()), Some("nt" | "nq" | "ttl" | "trig"))
}

fn serialize_for_format(format: OutputFormat, quads: &[Quad]) -> Result<String> {
    match format {
        OutputFormat::NQuads => {
            serialize_nquads_to_string(quads).context("failed to serialize quads in nquads format")
        }
        OutputFormat::NTriples => {
            let triples = quads_to_default_graph_triples(quads)?;
            serialize_ntriples_to_string(&triples)
                .context("failed to serialize quads in ntriples format")
        }
        OutputFormat::Turtle => {
            serialize_turtle_to_string(quads).context("failed to serialize quads in turtle format")
        }
        OutputFormat::Trig => {
            serialize_trig_to_string(quads).context("failed to serialize quads in trig format")
        }
    }
}

fn serialize_for_extension(extension: &str, quads: &[Quad]) -> Result<String> {
    match extension {
        "nq" => serialize_nquads_to_string(quads)
            .context("failed to serialize converted data as nquads"),
        "nt" => {
            let triples = quads_to_default_graph_triples(quads)?;
            serialize_ntriples_to_string(&triples)
                .context("failed to serialize converted data as ntriples")
        }
        "ttl" => serialize_turtle_to_string(quads)
            .context("failed to serialize converted data as turtle"),
        "trig" => {
            serialize_trig_to_string(quads).context("failed to serialize converted data as trig")
        }
        _ => bail!(
            "unsupported output format for {}: expected one of .nt, .nq, .ttl, .trig",
            extension
        ),
    }
}

fn triples_to_default_graph(triples: &[Triple]) -> Vec<Quad> {
    triples
        .iter()
        .map(|triple| {
            Quad::new(
                triple.subject().clone(),
                triple.predicate().clone(),
                triple.object().clone(),
                GraphName::DefaultGraph,
            )
        })
        .collect()
}

fn quads_to_default_graph_triples(quads: &[Quad]) -> Result<Vec<Triple>> {
    let mut triples = Vec::with_capacity(quads.len());
    for quad in quads {
        if quad.graph_name() != &GraphName::DefaultGraph {
            bail!("cannot serialize named-graph quads to ntriples");
        }
        triples.push(Triple::new(
            quad.subject().clone(),
            quad.predicate().clone(),
            quad.object().clone(),
        ));
    }
    Ok(triples)
}

fn parse_subject_selector(token: &str) -> Result<Option<Subject>> {
    if is_variable(token) {
        return Ok(None);
    }
    if let Some(label) = token.strip_prefix("_:") {
        let node = BlankNode::new(label).map_err(|error| anyhow::anyhow!("{error}"))?;
        return Ok(Some(Subject::BlankNode(node)));
    }
    let node = parse_named_node_token(token)?;
    Ok(Some(Subject::NamedNode(node)))
}

fn parse_predicate_selector(token: &str) -> Result<Option<Predicate>> {
    if is_variable(token) {
        return Ok(None);
    }
    Ok(Some(Predicate::new(parse_named_node_token(token)?)))
}

fn parse_object_selector(token: &str) -> Result<Option<Object>> {
    if is_variable(token) {
        return Ok(None);
    }
    if token.starts_with('"') && token.ends_with('"') && token.len() >= 2 {
        return Ok(Some(Object::Literal(Literal::new_simple(token.trim_matches('"')))));
    }
    if let Some(label) = token.strip_prefix("_:") {
        let node = BlankNode::new(label).map_err(|error| anyhow::anyhow!("{error}"))?;
        return Ok(Some(Object::BlankNode(node)));
    }
    Ok(Some(Object::NamedNode(parse_named_node_token(token)?)))
}

fn parse_graph_selector(token: &str) -> Result<Option<GraphName>> {
    if is_variable(token) {
        return Ok(None);
    }
    if token == "default" {
        return Ok(Some(GraphName::DefaultGraph));
    }
    if let Some(label) = token.strip_prefix("_:") {
        let node = BlankNode::new(label).map_err(|error| anyhow::anyhow!("{error}"))?;
        return Ok(Some(GraphName::BlankNode(node)));
    }
    Ok(Some(GraphName::NamedNode(parse_named_node_token(token)?)))
}

fn is_variable(token: &str) -> bool {
    token.starts_with('?')
}

fn parse_named_node_token(token: &str) -> Result<NamedNode> {
    if token == "rdf:type" {
        return NamedNode::new(RDF_TYPE_IRI).map_err(|error| anyhow::anyhow!("{error}"));
    }

    if token.starts_with('<') && token.ends_with('>') && token.len() >= 2 {
        let iri = &token[1..token.len() - 1];
        return NamedNode::new(iri).map_err(|error| anyhow::anyhow!("{error}"));
    }

    if token.starts_with("http://") || token.starts_with("https://") {
        return NamedNode::new(token).map_err(|error| anyhow::anyhow!("{error}"));
    }

    if let Some(expanded) = expand_prefixed_name(token) {
        return NamedNode::new(expanded).map_err(|error| anyhow::anyhow!("{error}"));
    }

    bail!("unsupported IRI token: {token}")
}

fn expand_prefixed_name(token: &str) -> Option<String> {
    let (prefix, local) = token.split_once(':')?;
    let base = match prefix {
        "rdf" => RDF_PREFIX,
        "rdfs" => RDFS_PREFIX,
        "xsd" => XSD_PREFIX,
        _ => return None,
    };
    Some(format!("{base}{local}"))
}

fn status_label(status: ManifestStatus) -> &'static str {
    match status {
        ManifestStatus::Pending => "pending",
        ManifestStatus::Passed => "passed",
        ManifestStatus::Failed => "failed",
    }
}

fn init_tracing() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_target(false)
        .without_time()
        .init();
}

impl OutputFormat {
    fn as_str(self) -> &'static str {
        match self {
            Self::NQuads => "nquads",
            Self::NTriples => "ntriples",
            Self::Turtle => "turtle",
            Self::Trig => "trig",
        }
    }
}
