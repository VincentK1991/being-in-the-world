//! Application boundary for the Oxigraph-backed store.
use anyhow::{Context, Result, bail, ensure};
use fs2::FileExt;
use oxigraph::{
    io::{RdfFormat, RdfSerializer},
    sparql::{
        QueryResults, SparqlEvaluator,
        results::{QueryResultsFormat, QueryResultsSerializer},
    },
    store::Store,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::Path,
};

#[derive(Default, Serialize, Deserialize)]
struct Catalog {
    version: u32,
    datasets: BTreeMap<String, String>,
}

/// A single isolated dataset. Clones share the same engine and snapshot machinery.
#[derive(Clone)]
pub struct Dataset {
    store: Store,
    identity: String,
    mutation_gate: std::sync::Arc<std::sync::Mutex<()>>,
}

impl Dataset {
    pub fn memory() -> Result<Self> {
        Ok(Self {
            store: Store::new()?,
            identity: format!("memory:{}", uuid::Uuid::new_v4()),
            mutation_gate: Default::default(),
        })
    }

    /// Opens a catalogued dataset; creation must be explicitly requested.
    pub fn open(root: &Path, name: &str, create: bool) -> Result<Self> {
        ensure!(
            !root.is_file(),
            "legacy database file: use rdftool migrate --from-redb <file> --to <new-directory>"
        );
        ensure!(!name.is_empty(), "dataset name must not be empty");
        if create {
            fs::create_dir_all(root)?;
        }
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(root.join("catalog.lock"))?;
        lock.lock_exclusive()?;
        let catalog_path = root.join("catalog.json");
        let mut catalog: Catalog = if catalog_path.exists() {
            serde_json::from_reader(File::open(&catalog_path)?)?
        } else {
            ensure!(create, "database catalog does not exist");
            Catalog { version: 1, ..Catalog::default() }
        };
        ensure!(catalog.version == 1, "unsupported database catalog version");
        let id = match catalog.datasets.get(name) {
            Some(id) => id.clone(),
            None => {
                ensure!(create, "dataset does not exist: {name}");
                uuid::Uuid::new_v4().to_string()
            }
        };
        ensure!(uuid::Uuid::parse_str(&id).is_ok(), "invalid catalog dataset identifier");
        let identity = format!("{}:{id}", root.canonicalize()?.display());
        if catalog.datasets.contains_key(name) {
            ensure!(
                root.join(&id).is_dir(),
                "catalogued dataset directory is missing; refusing to recreate it"
            );
        }
        let store = Store::open(root.join(&id))
            .context("opening dataset (another process may own its lock)")?;
        if !catalog.datasets.contains_key(name) {
            catalog.datasets.insert(name.to_owned(), id);
            let temporary = root.join(format!("catalog-{}.tmp", uuid::Uuid::new_v4()));
            let mut file = OpenOptions::new().write(true).create_new(true).open(&temporary)?;
            serde_json::to_writer_pretty(&mut file, &catalog)?;
            file.flush()?;
            file.sync_all()?;
            fs::rename(&temporary, &catalog_path)?;
            File::open(root)?.sync_all()?;
        }
        drop(lock);
        Ok(Self { store, identity, mutation_gate: Default::default() })
    }

    /// Host-side namespace for derived state; stable across reopen, distinct for copies/restores.
    pub fn identity(&self) -> &str {
        &self.identity
    }

    pub fn lock_mutation(&self) -> Result<std::sync::MutexGuard<'_, ()>> {
        self.mutation_gate.lock().map_err(|_| anyhow::anyhow!("dataset mutation lock poisoned"))
    }
    pub fn store(&self) -> &Store {
        &self.store
    }

    /// Normal loading is atomic, including parse failures.
    pub fn ingest(&self, reader: impl Read, format: RdfFormat) -> Result<()> {
        let _guard = self.lock_mutation()?;
        self.store.load_from_reader(format, reader)?;
        Ok(())
    }

    /// Bulk loading may commit partial progress; callers must opt in explicitly.
    pub fn ingest_bulk(
        &self,
        reader: impl Read,
        format: RdfFormat,
        memory_mb: usize,
        threads: usize,
    ) -> Result<()> {
        let _guard = self.lock_mutation()?;
        ensure!(memory_mb > 0 && threads > 0, "bulk limits must be positive");
        let mut loader = self
            .store
            .bulk_loader()
            .with_max_memory_size_in_megabytes(memory_mb)
            .with_num_threads(threads);
        loader.load_from_reader(format, reader)?;
        loader.commit()?;
        Ok(())
    }

    pub fn query(&self, query: &str) -> Result<QueryResults<'static>> {
        self.query_prepared(Self::prepare_query(query)?)
    }

    pub fn prepare_query(query: &str) -> Result<spargebra::Query> {
        let query = spargebra::SparqlParser::new().parse_query(query)?;
        crate::policy::query(&query)?;
        Ok(query)
    }

    /// Execute parsed algebra after validating the local-only policy again.
    pub fn query_prepared(&self, query: spargebra::Query) -> Result<QueryResults<'static>> {
        crate::policy::query(&query)?;
        // The dependency is built without http-client: no SERVICE/LOAD network access.
        Ok(SparqlEvaluator::new().for_query(query).on_store(&self.store).execute()?)
    }

    pub fn update(&self, update: &str) -> Result<()> {
        let _guard = self.lock_mutation()?;
        let update = spargebra::SparqlParser::new().parse_update(update)?;
        crate::policy::update(&update)?;
        SparqlEvaluator::new().for_update(update).on_store(&self.store).execute()?;
        Ok(())
    }

    pub fn dump(&self, format: RdfFormat, writer: impl Write) -> Result<()> {
        let mut quads = self.store.iter().collect::<Result<Vec<_>, _>>()?;
        if matches!(format, RdfFormat::NTriples | RdfFormat::Turtle) {
            ensure!(
                quads.iter().all(|q| q.graph_name.is_default_graph()),
                "cannot export named graphs in a triples-only format"
            );
        }
        quads.sort_by_key(|q| q.to_string());
        let mut output = RdfSerializer::from_format(format).for_writer(writer);
        for quad in quads {
            output.serialize_quad(&quad)?;
        }
        output.finish()?;
        Ok(())
    }

    pub fn backup(&self, target: &Path) -> Result<()> {
        ensure!(!target.exists(), "backup destination already exists");
        self.store.backup(target)?;
        Ok(())
    }
}

/// Serializes query results without mixing diagnostics into the result stream.
pub fn write_results(
    results: QueryResults<'_>,
    result_format: QueryResultsFormat,
    graph_format: RdfFormat,
    writer: impl Write,
) -> Result<()> {
    let serializer = QueryResultsSerializer::from_format(result_format);
    match results {
        QueryResults::Boolean(value) => {
            serializer.serialize_boolean_to_writer(writer, value)?;
        }
        QueryResults::Solutions(solutions) => {
            let mut output =
                serializer.serialize_solutions_to_writer(writer, solutions.variables().to_vec())?;
            for row in solutions {
                output.serialize(&row?)?;
            }
            output.finish()?;
        }
        QueryResults::Graph(triples) => {
            let mut output = RdfSerializer::from_format(graph_format).for_writer(writer);
            for triple in triples {
                output.serialize_triple(&triple?)?;
            }
            output.finish()?;
        }
    }
    Ok(())
}

pub fn format_for_path(path: &Path) -> Result<RdfFormat> {
    match path.extension().and_then(|v| v.to_str()) {
        Some("nt") => Ok(RdfFormat::NTriples),
        Some("nq") => Ok(RdfFormat::NQuads),
        Some("ttl") => Ok(RdfFormat::Turtle),
        Some("trig") => Ok(RdfFormat::TriG),
        _ => bail!("expected .nt, .nq, .ttl or .trig"),
    }
}

/// One catalogued dataset: the user-facing name and its on-disk directory identifier.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DatasetEntry {
    pub name: String,
    pub id: String,
    /// Host-side namespace used by plugins for derived state (matches `Dataset::identity`).
    pub identity: String,
}

/// Dataset names appear in URLs, config paths and plugin namespaces.
pub fn valid_dataset_name(name: &str) -> bool {
    (1..=64).contains(&name.len())
        && name.bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.'))
        && !name.starts_with('.')
}

fn read_catalog(root: &Path) -> Result<Catalog> {
    let path = root.join("catalog.json");
    if !path.exists() {
        return Ok(Catalog { version: 1, ..Catalog::default() });
    }
    let catalog: Catalog = serde_json::from_reader(File::open(path)?)?;
    ensure!(catalog.version == 1, "unsupported database catalog version");
    Ok(catalog)
}

fn entry(root: &Path, name: &str, id: &str) -> Result<DatasetEntry> {
    Ok(DatasetEntry {
        name: name.to_owned(),
        id: id.to_owned(),
        identity: format!("{}:{id}", root.canonicalize()?.display()),
    })
}

/// Lists catalogued datasets without opening them, so running servers are unaffected.
pub fn list_datasets(root: &Path) -> Result<Vec<DatasetEntry>> {
    ensure!(!root.is_file(), "legacy database file: migrate it before listing datasets");
    if !root.exists() {
        return Ok(Vec::new());
    }
    read_catalog(root)?.datasets.iter().map(|(name, id)| entry(root, name, id)).collect()
}

/// Removes a dataset's data and catalog entry. Refuses while any process has it open.
/// Returns the removed entry so callers can clean up derived plugin state.
pub fn delete_dataset(root: &Path, name: &str) -> Result<DatasetEntry> {
    ensure!(root.is_dir(), "database root does not exist");
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(root.join("catalog.lock"))?;
    lock.lock_exclusive()?;
    let mut catalog = read_catalog(root)?;
    let id = catalog
        .datasets
        .get(name)
        .cloned()
        .with_context(|| format!("dataset does not exist: {name}"))?;
    ensure!(uuid::Uuid::parse_str(&id).is_ok(), "invalid catalog dataset identifier");
    let removed = entry(root, name, &id)?;
    let directory = root.join(&id);
    if directory.is_dir() {
        // Opening takes the engine's own lock, which fails while another process serves it.
        drop(Store::open(&directory).context("dataset is in use; stop its server first")?);
    }
    catalog.datasets.remove(name);
    let temporary = root.join(format!("catalog-{}.tmp", uuid::Uuid::new_v4()));
    let mut file = OpenOptions::new().write(true).create_new(true).open(&temporary)?;
    serde_json::to_writer_pretty(&mut file, &catalog)?;
    file.flush()?;
    file.sync_all()?;
    fs::rename(&temporary, root.join("catalog.json"))?;
    File::open(root)?.sync_all()?;
    if directory.is_dir() {
        fs::remove_dir_all(&directory)?;
    }
    drop(lock);
    Ok(removed)
}

/// Semantic comparison independent of blank-node labels.
pub fn canonical_quads(quads: impl IntoIterator<Item = oxigraph::model::Quad>) -> String {
    use oxigraph::model::{Dataset as RdfDataset, graph::CanonicalizationAlgorithm};
    let mut dataset = RdfDataset::new();
    for quad in quads {
        dataset.insert(&quad);
    }
    dataset.canonicalize(CanonicalizationAlgorithm::Unstable);
    let mut lines: Vec<_> = dataset.iter().map(|q| q.to_string()).collect();
    lines.sort();
    lines.join("\n")
}

pub fn migrate(source: &Path, destination: &Path) -> Result<()> {
    ensure!(!destination.exists(), "migration destination already exists");
    let parent =
        destination.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let staging = tempfile::tempdir_in(parent)?;
    for (name, quads) in crate::redb_store::export_legacy(source)? {
        let bytes = rdf_io::serialize_nquads_to_string(&quads)?;
        let expected = Dataset::memory()?;
        expected.ingest(bytes.as_bytes(), RdfFormat::NQuads)?;
        let target = Dataset::open(staging.path(), &name, true)?;
        target.ingest(bytes.as_bytes(), RdfFormat::NQuads)?;
        ensure!(
            canonical_quads(expected.store.iter().collect::<Result<Vec<_>, _>>()?)
                == canonical_quads(target.store.iter().collect::<Result<Vec<_>, _>>()?),
            "migration verification failed for {name}"
        );
        target.store.flush()?;
    }
    ensure!(!destination.exists(), "destination appeared during migration");
    fs::rename(staging.path(), destination)?;
    File::open(parent)?.sync_all()?;
    Ok(())
}

pub fn restore(backup: &Path, destination: &Path, name: &str) -> Result<()> {
    ensure!(!destination.exists(), "restore destination already exists");
    let source = Store::open_read_only(backup)?;
    let parent =
        destination.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let staging = tempfile::tempdir_in(parent)?;
    {
        let target = Dataset::open(staging.path(), name, true)?;
        target.store.extend(source.iter().collect::<Result<Vec<_>, _>>()?)?;
        for graph in source.named_graphs() {
            target.store.insert_named_graph(&graph?)?;
        }
        ensure!(
            canonical_quads(source.iter().collect::<Result<Vec<_>, _>>()?)
                == canonical_quads(target.store.iter().collect::<Result<Vec<_>, _>>()?),
            "restore verification failed"
        );
        target.store.flush()?;
    }
    ensure!(!destination.exists(), "restore destination appeared");
    fs::rename(staging.path(), destination)?;
    File::open(parent)?.sync_all()?;
    Ok(())
}
