use anyhow::{Context, Result, bail, ensure};
use clap::{Parser, Subcommand};
use rdf_store::{
    engine::{self, Dataset, format_for_path, write_results},
    oxigraph::{
        io::{RdfFormat, RdfParser},
        sparql::{QueryResults, results::QueryResultsFormat},
    },
};
use std::{
    collections::HashSet,
    fs::{self, File},
    io::{self, Write},
    net::SocketAddr,
    path::{Path, PathBuf},
};

#[derive(Parser)]
#[command(version, about = "Local RDF database and SPARQL tools")]
struct Cli {
    /// Opt in to local semantic-search sidecars; also reads RDF_SEMANTIC_CONFIG.
    #[arg(long, global = true)]
    semantic_config: Option<PathBuf>,
    /// Opt in to configured inference jobs; also reads RDF_INFERENCE_CONFIG.
    #[arg(long, global = true)]
    inference_config: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    Plugin {
        #[arg(long)]
        source: String,
        #[arg(long, default_value = "default")]
        dataset: String,
        #[arg(long)]
        plugin: String,
        #[arg(long)]
        operation: String,
        #[arg(long, default_value = "{}")]
        input: String,
    },
    Ingest {
        #[arg(long)]
        from: PathBuf,
        #[arg(long)]
        to: PathBuf,
        #[arg(long, default_value = "default")]
        dataset: String,
        #[arg(long)]
        bulk: bool,
        #[arg(long, default_value_t = 256)]
        memory_mb: usize,
        #[arg(long, default_value_t = 2)]
        threads: usize,
    },
    Query {
        #[arg(long)]
        source: String,
        #[arg(long, default_value = "default")]
        dataset: String,
        #[arg(required_unless_present = "query_file", conflicts_with = "query_file")]
        query: Option<String>,
        #[arg(long)]
        query_file: Option<PathBuf>,
        #[arg(long)]
        format: Option<String>,
    },
    Update {
        #[arg(long)]
        source: String,
        #[arg(long, default_value = "default")]
        dataset: String,
        #[arg(required_unless_present = "update_file", conflicts_with = "update_file")]
        update: Option<String>,
        #[arg(long)]
        update_file: Option<PathBuf>,
    },
    QueryPattern {
        #[arg(long)]
        source: String,
        #[arg(long, default_value = "default")]
        dataset: String,
        #[arg(long)]
        subject: Option<String>,
        #[arg(long)]
        predicate: Option<String>,
        #[arg(long)]
        object: Option<String>,
        #[arg(long)]
        graph: Option<String>,
    },
    Dump {
        #[arg(long)]
        source: String,
        #[arg(long, default_value = "default")]
        dataset: String,
        #[arg(long, default_value = "n-quads")]
        format: String,
    },
    Stats {
        #[arg(long)]
        source: String,
        #[arg(long, default_value = "default")]
        dataset: String,
    },
    Diff {
        #[arg(long)]
        query: PathBuf,
        #[arg(long, value_delimiter=',', num_args=1..)]
        sources: Vec<String>,
        #[arg(long, default_value = "default")]
        dataset: String,
    },
    Migrate {
        #[arg(long)]
        from_redb: PathBuf,
        #[arg(long)]
        to: PathBuf,
    },
    Backup {
        #[arg(long)]
        source: PathBuf,
        #[arg(long, default_value = "default")]
        dataset: String,
        #[arg(long)]
        to: PathBuf,
    },
    Restore {
        #[arg(long)]
        from: PathBuf,
        #[arg(long)]
        to: PathBuf,
        #[arg(long, default_value = "default")]
        dataset: String,
    },
    Serve {
        #[arg(long)]
        db: PathBuf,
        #[arg(long, default_value = "default")]
        dataset: String,
        #[arg(long, default_value = "127.0.0.1:7878")]
        bind: SocketAddr,
        #[arg(long, default_value_t=16*1024*1024)]
        max_body_bytes: usize,
        #[arg(long, default_value_t = 4)]
        concurrency: usize,
    },
    /// Serve many datasets: one `serve` process per dataset, routed by `/d/<name>/`.
    Gateway {
        #[arg(long)]
        db: PathBuf,
        /// Dataset that unprefixed requests use; created if missing and never deletable.
        #[arg(long, default_value = "default")]
        default_dataset: String,
        #[arg(long, default_value = "127.0.0.1:7878")]
        bind: SocketAddr,
        /// Optional `<dir>/<dataset>/{semantic,inference,virtual,federation}.json` per dataset.
        #[arg(long)]
        tenant_config_dir: Option<PathBuf>,
        #[arg(long, default_value_t=16*1024*1024)]
        max_body_bytes: usize,
        #[arg(long, default_value_t = 4)]
        concurrency: usize,
    },
    /// List, create or delete datasets in a database directory.
    Datasets {
        #[arg(long)]
        db: PathBuf,
        #[command(subcommand)]
        action: DatasetAction,
    },
    TestSuite {
        #[arg(long)]
        manifest: PathBuf,
        #[arg(long, default_value = "fresh")]
        source: String,
    },
    SmokeSuite {
        #[arg(long)]
        manifest: PathBuf,
    },
}
#[derive(Subcommand)]
enum DatasetAction {
    /// Print the catalogued datasets as JSON.
    List,
    Create {
        name: String,
    },
    /// Delete a dataset and its Qdrant collections. The dataset must not be served.
    Delete {
        name: String,
        /// Confirm the deletion; it cannot be undone.
        #[arg(long)]
        yes: bool,
    },
}
fn main() -> Result<()> {
    let cli = Cli::parse();
    let config =
        cli.semantic_config.or_else(|| std::env::var_os("RDF_SEMANTIC_CONFIG").map(PathBuf::from));
    let inference = cli
        .inference_config
        .or_else(|| std::env::var_os("RDF_INFERENCE_CONFIG").map(PathBuf::from));
    run(cli.command, config.as_deref(), inference.as_deref())
}
fn application_database(
    dataset: Dataset,
    semantic_config: Option<&Path>,
    inference_config: Option<&Path>,
) -> Result<rdf_engine::Database> {
    let identity = dataset.identity().to_owned();
    let mut database = rdf_engine::Database::from_dataset(dataset);
    database.register(rdf_plugin_dataset_summary::DatasetSummary)?;
    database.register(rdf_plugin_custom_properties::CustomProperties::examples())?;
    if let Some(path) = semantic_config {
        ensure!(
            !identity.starts_with("memory:"),
            "semantic search requires a persistent dataset, not a transient RDF-file source"
        );
        let config =
            serde_json::from_reader(File::open(path).context("open semantic configuration")?)?;
        database.register(rdf_plugin_semantic_search::SemanticSearch::new(
            &identity,
            config,
            &rdf_embedding_providers::registry()?,
        )?)?;
    }
    if let Some(path) = inference_config {
        ensure!(
            !identity.starts_with("memory:"),
            "inference requires a persistent dataset, not a transient RDF-file source"
        );
        let config =
            serde_json::from_reader(File::open(path).context("open inference configuration")?)?;
        database.register(rdf_plugin_inference::Inference::with_registry(
            config,
            &rdf_inference_providers::registry()?,
        )?)?;
    }
    // Virtual graph and federation sidecars are opt-in per dataset; the gateway sets these per tenant.
    for (variable, name) in
        [("RDF_VIRTUAL_CONFIG", "virtual graph"), ("RDF_FEDERATION_CONFIG", "federation")]
    {
        let Some(path) = std::env::var_os(variable).map(PathBuf::from) else { continue };
        let file = File::open(&path).with_context(|| format!("open {name} configuration"))?;
        if variable == "RDF_VIRTUAL_CONFIG" {
            database.register(rdf_plugin_federation::VirtualGraph::new(
                serde_json::from_reader(file)?,
            )?)?;
        } else {
            database.register(rdf_plugin_federation::Federation::new(serde_json::from_reader(
                file,
            )?)?)?;
        }
    }
    // Last, so the catalog sees every plugin registered above.
    database.register(rdf_plugin_catalog::PluginCatalog::new(database.catalog()))?;
    Ok(database)
}
/// Removes derived state that lives outside the store (Qdrant collections) for a deleted dataset.
fn purge_derived(entry: &engine::DatasetEntry, semantic_config: Option<&Path>) -> Result<String> {
    let Some(path) = semantic_config else { return Ok("no semantic configuration".into()) };
    let config: serde_json::Value =
        serde_json::from_reader(File::open(path).context("open semantic configuration")?)?;
    let url = config["qdrant_url"].as_str().context("semantic configuration has no qdrant_url")?;
    let removed = rdf_plugin_semantic_search::purge_dataset(url, &entry.identity)?;
    Ok(format!("removed {removed} Qdrant collection(s)"))
}
fn load(source: &str, dataset: &str) -> Result<Dataset> {
    let path = Path::new(source);
    if let Ok(format) = format_for_path(path) {
        let store = Dataset::memory()?;
        store.ingest(File::open(path)?, format)?;
        Ok(store)
    } else {
        Dataset::open(path, dataset, false)
    }
}
fn text(inline: Option<String>, file: Option<PathBuf>) -> Result<String> {
    match (inline, file) {
        (Some(value), None) => Ok(value),
        (None, Some(path)) => Ok(fs::read_to_string(path)?),
        _ => bail!("provide inline text or a file"),
    }
}
fn rdf_format(value: &str) -> Result<RdfFormat> {
    match value {
        "n-quads" | "nquads" => Ok(RdfFormat::NQuads),
        "n-triples" | "ntriples" => Ok(RdfFormat::NTriples),
        "turtle" => Ok(RdfFormat::Turtle),
        "trig" => Ok(RdfFormat::TriG),
        _ => bail!("unsupported RDF format: {value}"),
    }
}
fn remote(source: &str) -> bool {
    source.starts_with("http://") || source.starts_with("https://")
}
fn request(source: &str, operation: &str, body: &str, accept: &str) -> Result<(String, Vec<u8>)> {
    let client =
        reqwest::blocking::Client::builder().redirect(reqwest::redirect::Policy::none()).build()?;
    let url = if source.trim_end_matches('/').ends_with(&format!("/{operation}")) {
        source.to_owned()
    } else {
        format!("{}/{operation}", source.trim_end_matches('/'))
    };
    let response = client
        .post(url)
        .header("Content-Type", format!("application/sparql-{operation}"))
        .header("Accept", accept)
        .body(body.to_owned())
        .send()?;
    let status = response.status();
    let mime = response
        .headers()
        .get("Content-Type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_owned();
    let bytes = response.bytes()?.to_vec();
    ensure!(status.is_success(), "HTTP {status}: {}", String::from_utf8_lossy(&bytes));
    Ok((mime, bytes))
}
fn run(
    command: Command,
    semantic_config: Option<&Path>,
    inference_config: Option<&Path>,
) -> Result<()> {
    match command {
        Command::Plugin { source, dataset, plugin, operation, input } => {
            let input: serde_json::Value =
                serde_json::from_str(&input).context("invalid plugin input JSON")?;
            let result = if remote(&source) {
                ensure!(
                    [&plugin, &operation].iter().all(|name| !name.is_empty()
                        && name
                            .bytes()
                            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')),
                    "invalid plugin or operation name"
                );
                let response = reqwest::blocking::Client::builder()
                    .timeout(std::time::Duration::from_secs(3600))
                    .build()?
                    .post(format!(
                        "{}/api/plugins/{plugin}/operations/{operation}",
                        source.trim_end_matches('/')
                    ))
                    .header("Content-Type", "application/json")
                    .body(serde_json::to_vec(&input)?)
                    .send()?;
                let status = response.status();
                let body = response.text()?;
                ensure!(status.is_success(), "plugin HTTP {status}: {body}");
                serde_json::from_str(&body)?
            } else {
                application_database(load(&source, &dataset)?, semantic_config, inference_config)?
                    .invoke(&plugin, &operation, input)?
            };
            serde_json::to_writer(io::stdout().lock(), &result)?;
            println!();
        }
        Command::Ingest { from, to, dataset, bulk, memory_mb, threads } => {
            let format = format_for_path(&from)?;
            if let Ok(output) = format_for_path(&to) {
                let data = Dataset::memory()?;
                data.ingest(File::open(&from)?, format)?;
                let mut bytes = Vec::new();
                data.dump(output, &mut bytes)?;
                fs::write(to, bytes)?;
            } else {
                let data = Dataset::open(&to, &dataset, true)?;
                if bulk {
                    data.ingest_bulk(File::open(&from)?, format, memory_mb, threads)?;
                } else {
                    data.ingest(File::open(&from)?, format)?;
                }
            }
            eprintln!("ingest complete");
        }
        Command::Query { source, dataset, query, query_file, format } => {
            let query = text(query, query_file)?;
            if remote(&source) {
                let accept = format
                    .as_deref()
                    .map(result_mime)
                    .transpose()?
                    .unwrap_or("application/sparql-results+json, text/turtle");
                let (_, bytes) = request(&source, "query", &query, accept)?;
                io::stdout().write_all(&bytes)?;
            } else {
                let data = application_database(
                    load(&source, &dataset)?,
                    semantic_config,
                    inference_config,
                )?;
                let results = data.query(&query)?;
                let (table, graph) =
                    formats(format.as_deref(), matches!(&results, QueryResults::Graph(_)))?;
                write_results(results, table, graph, io::stdout().lock())?;
            }
        }
        Command::Update { source, dataset, update, update_file } => {
            let update = text(update, update_file)?;
            if remote(&source) {
                request(&source, "update", &update, "*/*")?;
            } else {
                ensure!(
                    format_for_path(Path::new(&source)).is_err(),
                    "file sources are read-only for updates"
                );
                application_database(
                    Dataset::open(Path::new(&source), &dataset, false)?,
                    semantic_config,
                    inference_config,
                )?
                .update(&update)?;
            }
        }
        Command::Dump { source, dataset, format } => {
            load(&source, &dataset)?.dump(rdf_format(&format)?, io::stdout().lock())?
        }
        Command::Stats { source, dataset } => {
            let data = load(&source, &dataset)?;
            let quads = data.store().iter().collect::<Result<Vec<_>, _>>()?;
            let subjects: HashSet<_> = quads.iter().map(|q| &q.subject).collect();
            let predicates: HashSet<_> = quads.iter().map(|q| &q.predicate).collect();
            let objects: HashSet<_> = quads.iter().map(|q| &q.object).collect();
            let graphs: HashSet<_> = quads.iter().map(|q| &q.graph_name).collect();
            println!(
                "{}",
                serde_json::json!({"quads":quads.len(), "distinct_subjects":subjects.len(), "distinct_predicates":predicates.len(), "distinct_objects":objects.len(), "distinct_graphs":graphs.len()})
            );
        }
        Command::QueryPattern { source, dataset, subject, predicate, object, graph } => {
            let data = load(&source, &dataset)?;
            let s = selector(subject.as_deref())?;
            let p = selector(predicate.as_deref())?;
            let o = selector(object.as_deref())?;
            let g = if graph.as_deref() == Some("default") {
                Some(rdf_store::oxigraph::model::GraphName::DefaultGraph.to_string())
            } else {
                selector(graph.as_deref())?
            };
            let mut result = Vec::new();
            for quad in data.store().iter() {
                let q = quad?;
                if s.as_ref().is_none_or(|s| *s == q.subject.to_string())
                    && p.as_ref().is_none_or(|p| *p == q.predicate.to_string())
                    && o.as_ref().is_none_or(|o| *o == q.object.to_string())
                    && g.as_ref().is_none_or(|g| *g == q.graph_name.to_string())
                {
                    result.push(format!("{q} ."));
                }
            }
            result.sort();
            for line in result {
                println!("{line}");
            }
        }
        Command::Migrate { from_redb, to } => engine::migrate(&from_redb, &to)?,
        Command::Backup { source, dataset, to } => {
            Dataset::open(&source, &dataset, false)?.backup(&to)?
        }
        Command::Restore { from, to, dataset } => engine::restore(&from, &to, &dataset)?,
        Command::Serve { db, dataset, bind, max_body_bytes, concurrency } => {
            let database = application_database(
                Dataset::open(&db, &dataset, true)?,
                semantic_config,
                inference_config,
            )?;
            rdf_server::serve(database, bind, max_body_bytes, concurrency)?;
        }
        Command::Gateway {
            db,
            default_dataset,
            bind,
            tenant_config_dir,
            max_body_bytes,
            concurrency,
        } => {
            let (global, tenants) =
                (semantic_config.map(Path::to_path_buf), tenant_config_dir.clone());
            rdf_server::gateway::run(rdf_server::gateway::GatewayConfig {
                root: db,
                executable: std::env::current_exe()?,
                bind,
                default_dataset,
                semantic_config: global.clone(),
                inference_config: inference_config.map(Path::to_path_buf),
                tenant_config_dir,
                max_body: max_body_bytes,
                concurrency,
                on_delete: std::sync::Arc::new(move |entry| {
                    let specific = tenants
                        .as_ref()
                        .map(|dir| dir.join(&entry.name).join("semantic.json"))
                        .filter(|path| path.is_file());
                    purge_derived(entry, specific.as_deref().or(global.as_deref())).map(drop)
                }),
            })?;
        }
        Command::Datasets { db, action } => match action {
            DatasetAction::List => {
                serde_json::to_writer_pretty(io::stdout().lock(), &engine::list_datasets(&db)?)?;
                println!();
            }
            DatasetAction::Create { name } => {
                ensure!(engine::valid_dataset_name(&name), "invalid dataset name: {name}");
                ensure!(
                    !engine::list_datasets(&db)?.iter().any(|d| d.name == name),
                    "dataset already exists: {name}"
                );
                drop(Dataset::open(&db, &name, true)?);
                eprintln!("created dataset {name}");
            }
            DatasetAction::Delete { name, yes } => {
                ensure!(yes, "deleting a dataset cannot be undone; pass --yes to confirm");
                let entry = engine::delete_dataset(&db, &name)?;
                eprintln!("deleted dataset {name}");
                match purge_derived(&entry, semantic_config) {
                    Ok(note) => eprintln!("{note}"),
                    Err(error) => {
                        eprintln!("warning: dataset deleted, but cleanup failed: {error:#}")
                    }
                }
            }
        },
        Command::Diff { query, sources, dataset } => {
            ensure!(sources.len() >= 2, "diff requires at least two sources");
            let query = fs::read_to_string(query)?;
            let mut expected = None;
            for source in sources {
                let value = if remote(&source) {
                    let (mime, bytes) = request(
                        &source,
                        "query",
                        &query,
                        "application/sparql-results+json, application/n-triples",
                    )?;
                    rdf_store::comparison::from_bytes(&bytes, &mime, &query)?
                } else {
                    rdf_store::comparison::from_results(
                        application_database(
                            load(&source, &dataset)?,
                            semantic_config,
                            inference_config,
                        )?
                        .query(&query)?,
                        &query,
                    )?
                };
                if let Some(expected) = &expected {
                    ensure!(*expected == value, "result mismatch: {source}");
                } else {
                    expected = Some(value);
                }
            }
            println!("equal");
        }
        Command::TestSuite { manifest, source } => {
            ensure!(source == "fresh", "test-suite requires --source fresh");
            let report = rdf_testkit::conformance::run(&manifest)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            ensure!(report.failed == 0, "conformance failures");
        }
        Command::SmokeSuite { manifest } => {
            for result in rdf_testkit::manifest::run_manifest_smoke(manifest)? {
                println!("{:?} {}", result.status, result.case.id);
            }
        }
    }
    Ok(())
}
fn selector(value: Option<&str>) -> Result<Option<String>> {
    let Some(value) = value.filter(|v| !v.starts_with('?')) else {
        return Ok(None);
    };
    let expanded = [
        ("rdf:", "http://www.w3.org/1999/02/22-rdf-syntax-ns#"),
        ("rdfs:", "http://www.w3.org/2000/01/rdf-schema#"),
        ("xsd:", "http://www.w3.org/2001/XMLSchema#"),
    ]
    .into_iter()
    .find_map(|(prefix, iri)| value.strip_prefix(prefix).map(|suffix| format!("<{iri}{suffix}>")))
    .unwrap_or_else(|| value.to_owned());
    let input = format!("<urn:s> <urn:p> {expanded} .");
    let quad = RdfParser::from_format(RdfFormat::NTriples)
        .for_reader(input.as_bytes())
        .next()
        .context("empty selector")??;
    Ok(Some(quad.object.to_string()))
}
fn result_mime(format: &str) -> Result<&'static str> {
    Ok(match format {
        "json" => "application/sparql-results+json",
        "xml" => "application/sparql-results+xml",
        "csv" => "text/csv",
        "tsv" => "text/tab-separated-values",
        _ => rdf_format(format)?.media_type(),
    })
}
fn formats(format: Option<&str>, graph: bool) -> Result<(QueryResultsFormat, RdfFormat)> {
    if graph {
        return Ok((
            QueryResultsFormat::Json,
            format.map(rdf_format).transpose()?.unwrap_or(RdfFormat::Turtle),
        ));
    }
    Ok((
        match format.unwrap_or("json") {
            "json" => QueryResultsFormat::Json,
            "xml" => QueryResultsFormat::Xml,
            "csv" => QueryResultsFormat::Csv,
            "tsv" => QueryResultsFormat::Tsv,
            value => bail!("invalid result format: {value}"),
        },
        RdfFormat::Turtle,
    ))
}
