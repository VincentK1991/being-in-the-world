use crate::{QuadPattern, StoreError, StoreStats, object_as_term, subject_as_term};
use rdf_model::{GraphName, Literal, NamedNode, Object, Predicate, Quad, Subject, Term};
use redb::{Database, ReadableTable, TableDefinition};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

const META_TABLE: TableDefinition<&str, u64> = TableDefinition::new("meta");
const DATASET_BY_NAME_TABLE: TableDefinition<&str, u64> = TableDefinition::new("dataset_by_name");
const NEXT_TERM_BY_DATASET_TABLE: TableDefinition<u64, u64> =
    TableDefinition::new("next_term_by_dataset");
const TERM_TO_ID_TABLE: TableDefinition<&[u8], u64> = TableDefinition::new("term_to_id");
const ID_TO_TERM_TABLE: TableDefinition<&[u8], &[u8]> = TableDefinition::new("id_to_term");

const DGSPO_TABLE: TableDefinition<&[u8], &[u8]> = TableDefinition::new("d_gspo");
const DGPOS_TABLE: TableDefinition<&[u8], &[u8]> = TableDefinition::new("d_gpos");
const DGOSP_TABLE: TableDefinition<&[u8], &[u8]> = TableDefinition::new("d_gosp");
const DSPOG_TABLE: TableDefinition<&[u8], &[u8]> = TableDefinition::new("d_spog");
const DPOSG_TABLE: TableDefinition<&[u8], &[u8]> = TableDefinition::new("d_posg");
const DOSPG_TABLE: TableDefinition<&[u8], &[u8]> = TableDefinition::new("d_ospg");

const META_NEXT_DATASET_ID: &str = "next_dataset_id";
const DEFAULT_GRAPH_ID: u64 = 0;

#[derive(Debug)]
pub struct RedbQuadStorage {
    db: Database,
    #[allow(dead_code)]
    path: PathBuf,
    dataset_id: u64,
}

impl RedbQuadStorage {
    pub fn open(path: impl AsRef<Path>, dataset: &str) -> Result<Self, StoreError> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let db = if path.exists() {
            Database::open(&path).map_err(map_redb)?
        } else {
            Database::create(&path).map_err(map_redb)?
        };

        let dataset_id = ensure_dataset(&db, dataset)?;
        Ok(Self { db, path, dataset_id })
    }

    pub fn insert(&mut self, quad: Quad) -> Result<bool, StoreError> {
        let write_txn = self.db.begin_write().map_err(map_redb)?;

        let [s, p, o, g] = encode_quad_ids_write(&write_txn, self.dataset_id, &quad)?;
        let canonical = encode_ids([s, p, o, g]);
        let canonical_key = index_key(self.dataset_id, [s, p, o, g]);

        {
            let mut spog = write_txn.open_table(DSPOG_TABLE).map_err(map_redb)?;
            if spog.get(canonical_key.as_slice()).map_err(map_redb)?.is_some() {
                drop(spog);
                write_txn.commit().map_err(map_redb)?;
                return Ok(false);
            }

            spog.insert(canonical_key.as_slice(), canonical.as_slice()).map_err(map_redb)?;
        }

        insert_index_entry(&write_txn, DPOSG_TABLE, self.dataset_id, [p, o, s, g], &canonical)?;
        insert_index_entry(&write_txn, DOSPG_TABLE, self.dataset_id, [o, s, p, g], &canonical)?;
        insert_index_entry(&write_txn, DGSPO_TABLE, self.dataset_id, [g, s, p, o], &canonical)?;
        insert_index_entry(&write_txn, DGPOS_TABLE, self.dataset_id, [g, p, o, s], &canonical)?;
        insert_index_entry(&write_txn, DGOSP_TABLE, self.dataset_id, [g, o, s, p], &canonical)?;

        write_txn.commit().map_err(map_redb)?;
        Ok(true)
    }

    pub fn remove(&mut self, quad: &Quad) -> Result<bool, StoreError> {
        let write_txn = self.db.begin_write().map_err(map_redb)?;
        let Some([s, p, o, g]) = resolve_quad_ids_write(&write_txn, self.dataset_id, quad)? else {
            write_txn.commit().map_err(map_redb)?;
            return Ok(false);
        };

        let canonical_key = index_key(self.dataset_id, [s, p, o, g]);

        let existed = {
            let mut spog = write_txn.open_table(DSPOG_TABLE).map_err(map_redb)?;
            let existed = spog.get(canonical_key.as_slice()).map_err(map_redb)?.is_some();
            if existed {
                spog.remove(canonical_key.as_slice()).map_err(map_redb)?;
            }
            existed
        };

        if existed {
            remove_index_entry(&write_txn, DPOSG_TABLE, self.dataset_id, [p, o, s, g])?;
            remove_index_entry(&write_txn, DOSPG_TABLE, self.dataset_id, [o, s, p, g])?;
            remove_index_entry(&write_txn, DGSPO_TABLE, self.dataset_id, [g, s, p, o])?;
            remove_index_entry(&write_txn, DGPOS_TABLE, self.dataset_id, [g, p, o, s])?;
            remove_index_entry(&write_txn, DGOSP_TABLE, self.dataset_id, [g, o, s, p])?;
        }

        write_txn.commit().map_err(map_redb)?;
        Ok(existed)
    }

    pub fn all_quads(&self) -> Result<Vec<Quad>, StoreError> {
        let read_txn = self.db.begin_read().map_err(map_redb)?;
        let table = read_txn.open_table(DSPOG_TABLE).map_err(map_redb)?;
        let (lower, upper) = range_for_dataset_prefix(self.dataset_id, [None, None, None, None]);

        let mut quads = Vec::new();
        let iter = table.range(lower.as_slice()..=upper.as_slice()).map_err(map_redb)?;
        for entry in iter {
            let (_, value) = entry.map_err(map_redb)?;
            let ids = decode_ids(value.value())?;
            quads.push(decode_quad_from_ids(&read_txn, self.dataset_id, ids)?);
        }
        Ok(quads)
    }

    pub fn quads_matching(&self, pattern: &QuadPattern) -> Result<Vec<Quad>, StoreError> {
        let read_txn = self.db.begin_read().map_err(map_redb)?;
        let Some(ids_pattern) = resolve_pattern_ids_read(&read_txn, self.dataset_id, pattern)?
        else {
            return Ok(Vec::new());
        };

        let index = RedbIndex::best_for(&ids_pattern);
        let ordered = index.order_pattern(&ids_pattern);
        let (lower, upper) = range_for_dataset_prefix(self.dataset_id, ordered);

        let table = read_txn.open_table(index.table()).map_err(map_redb)?;
        let iter = table.range(lower.as_slice()..=upper.as_slice()).map_err(map_redb)?;

        let mut results = Vec::new();
        for entry in iter {
            let (_, value) = entry.map_err(map_redb)?;
            let ids = decode_ids(value.value())?;
            if !matches_pattern(ids, &ids_pattern) {
                continue;
            }
            results.push(decode_quad_from_ids(&read_txn, self.dataset_id, ids)?);
        }

        Ok(results)
    }

    pub fn stats(&self) -> Result<StoreStats, StoreError> {
        let read_txn = self.db.begin_read().map_err(map_redb)?;
        let table = read_txn.open_table(DSPOG_TABLE).map_err(map_redb)?;
        let (lower, upper) = range_for_dataset_prefix(self.dataset_id, [None, None, None, None]);

        let mut subjects = HashSet::new();
        let mut predicates = HashSet::new();
        let mut objects = HashSet::new();
        let mut graphs = HashSet::new();
        let mut quads = 0_usize;

        let iter = table.range(lower.as_slice()..=upper.as_slice()).map_err(map_redb)?;
        for entry in iter {
            let (_, value) = entry.map_err(map_redb)?;
            let [s, p, o, g] = decode_ids(value.value())?;
            quads += 1;
            subjects.insert(s);
            predicates.insert(p);
            objects.insert(o);
            graphs.insert(g);
        }

        Ok(StoreStats {
            quads,
            distinct_subjects: subjects.len(),
            distinct_predicates: predicates.len(),
            distinct_objects: objects.len(),
            distinct_graphs: graphs.len(),
        })
    }
}

#[derive(Clone, Copy)]
enum RedbIndex {
    Dgspo,
    Dgpos,
    Dgosp,
    Dspog,
    Dposg,
    Dospg,
}

impl RedbIndex {
    fn table(self) -> TableDefinition<'static, &'static [u8], &'static [u8]> {
        match self {
            Self::Dgspo => DGSPO_TABLE,
            Self::Dgpos => DGPOS_TABLE,
            Self::Dgosp => DGOSP_TABLE,
            Self::Dspog => DSPOG_TABLE,
            Self::Dposg => DPOSG_TABLE,
            Self::Dospg => DOSPG_TABLE,
        }
    }

    fn order_pattern(self, pattern: &[Option<u64>; 4]) -> [Option<u64>; 4] {
        match self {
            Self::Dspog => [pattern[0], pattern[1], pattern[2], pattern[3]],
            Self::Dposg => [pattern[1], pattern[2], pattern[0], pattern[3]],
            Self::Dospg => [pattern[2], pattern[0], pattern[1], pattern[3]],
            Self::Dgspo => [pattern[3], pattern[0], pattern[1], pattern[2]],
            Self::Dgpos => [pattern[3], pattern[1], pattern[2], pattern[0]],
            Self::Dgosp => [pattern[3], pattern[2], pattern[0], pattern[1]],
        }
    }

    fn best_for(pattern: &[Option<u64>; 4]) -> Self {
        let candidates = [
            (Self::Dspog, prefix_len([pattern[0], pattern[1], pattern[2], pattern[3]])),
            (Self::Dposg, prefix_len([pattern[1], pattern[2], pattern[0], pattern[3]])),
            (Self::Dospg, prefix_len([pattern[2], pattern[0], pattern[1], pattern[3]])),
            (Self::Dgspo, prefix_len([pattern[3], pattern[0], pattern[1], pattern[2]])),
            (Self::Dgpos, prefix_len([pattern[3], pattern[1], pattern[2], pattern[0]])),
            (Self::Dgosp, prefix_len([pattern[3], pattern[2], pattern[0], pattern[1]])),
        ];
        candidates
            .into_iter()
            .max_by_key(|(_, prefix)| *prefix)
            .map(|(index, _)| index)
            .expect("index list must be non-empty")
    }
}

fn ensure_dataset(db: &Database, dataset: &str) -> Result<u64, StoreError> {
    let write_txn = db.begin_write().map_err(map_redb)?;

    // Ensure all persistent tables exist even before first insert.
    write_txn.open_table(META_TABLE).map_err(map_redb)?;
    write_txn.open_table(DATASET_BY_NAME_TABLE).map_err(map_redb)?;
    write_txn.open_table(NEXT_TERM_BY_DATASET_TABLE).map_err(map_redb)?;
    write_txn.open_table(TERM_TO_ID_TABLE).map_err(map_redb)?;
    write_txn.open_table(ID_TO_TERM_TABLE).map_err(map_redb)?;
    write_txn.open_table(DGSPO_TABLE).map_err(map_redb)?;
    write_txn.open_table(DGPOS_TABLE).map_err(map_redb)?;
    write_txn.open_table(DGOSP_TABLE).map_err(map_redb)?;
    write_txn.open_table(DSPOG_TABLE).map_err(map_redb)?;
    write_txn.open_table(DPOSG_TABLE).map_err(map_redb)?;
    write_txn.open_table(DOSPG_TABLE).map_err(map_redb)?;

    let dataset_id = {
        let mut dataset_table = write_txn.open_table(DATASET_BY_NAME_TABLE).map_err(map_redb)?;
        if let Some(existing) = dataset_table.get(dataset).map_err(map_redb)? {
            existing.value()
        } else {
            let next_id = {
                let mut meta = write_txn.open_table(META_TABLE).map_err(map_redb)?;
                let current = meta
                    .get(META_NEXT_DATASET_ID)
                    .map_err(map_redb)?
                    .map(|value| value.value())
                    .unwrap_or(1);
                meta.insert(META_NEXT_DATASET_ID, current + 1).map_err(map_redb)?;
                current
            };
            dataset_table.insert(dataset, next_id).map_err(map_redb)?;
            let mut counters =
                write_txn.open_table(NEXT_TERM_BY_DATASET_TABLE).map_err(map_redb)?;
            counters.insert(next_id, 1).map_err(map_redb)?;
            next_id
        }
    };
    write_txn.commit().map_err(map_redb)?;
    Ok(dataset_id)
}

fn insert_index_entry(
    write_txn: &redb::WriteTransaction,
    table_def: TableDefinition<'static, &'static [u8], &'static [u8]>,
    dataset_id: u64,
    ordered_ids: [u64; 4],
    canonical: &[u8],
) -> Result<(), StoreError> {
    let mut table = write_txn.open_table(table_def).map_err(map_redb)?;
    let key = index_key(dataset_id, ordered_ids);
    table.insert(key.as_slice(), canonical).map_err(map_redb)?;
    Ok(())
}

fn remove_index_entry(
    write_txn: &redb::WriteTransaction,
    table_def: TableDefinition<'static, &'static [u8], &'static [u8]>,
    dataset_id: u64,
    ordered_ids: [u64; 4],
) -> Result<(), StoreError> {
    let mut table = write_txn.open_table(table_def).map_err(map_redb)?;
    let key = index_key(dataset_id, ordered_ids);
    table.remove(key.as_slice()).map_err(map_redb)?;
    Ok(())
}

fn encode_quad_ids_write(
    write_txn: &redb::WriteTransaction,
    dataset_id: u64,
    quad: &Quad,
) -> Result<[u64; 4], StoreError> {
    let subject = term_id_for_write(write_txn, dataset_id, &subject_as_term(quad.subject())?)?;
    let predicate = term_id_for_write(
        write_txn,
        dataset_id,
        &Term::NamedNode(quad.predicate().as_named_node().clone()),
    )?;
    let object = term_id_for_write(write_txn, dataset_id, &object_as_term(quad.object())?)?;
    let graph = match quad.graph_name() {
        GraphName::DefaultGraph => DEFAULT_GRAPH_ID,
        GraphName::NamedNode(node) => {
            term_id_for_write(write_txn, dataset_id, &Term::NamedNode(node.clone()))?
        }
        GraphName::BlankNode(node) => {
            term_id_for_write(write_txn, dataset_id, &Term::BlankNode(node.clone()))?
        }
    };
    Ok([subject, predicate, object, graph])
}

fn resolve_quad_ids_write(
    write_txn: &redb::WriteTransaction,
    dataset_id: u64,
    quad: &Quad,
) -> Result<Option<[u64; 4]>, StoreError> {
    let Some(subject) =
        term_id_for_read_from_write(write_txn, dataset_id, &subject_as_term(quad.subject())?)?
    else {
        return Ok(None);
    };
    let Some(predicate) = term_id_for_read_from_write(
        write_txn,
        dataset_id,
        &Term::NamedNode(quad.predicate().as_named_node().clone()),
    )?
    else {
        return Ok(None);
    };
    let Some(object) =
        term_id_for_read_from_write(write_txn, dataset_id, &object_as_term(quad.object())?)?
    else {
        return Ok(None);
    };
    let graph = match quad.graph_name() {
        GraphName::DefaultGraph => Some(DEFAULT_GRAPH_ID),
        GraphName::NamedNode(node) => {
            term_id_for_read_from_write(write_txn, dataset_id, &Term::NamedNode(node.clone()))?
        }
        GraphName::BlankNode(node) => {
            term_id_for_read_from_write(write_txn, dataset_id, &Term::BlankNode(node.clone()))?
        }
    };
    let Some(graph) = graph else {
        return Ok(None);
    };

    Ok(Some([subject, predicate, object, graph]))
}

fn resolve_pattern_ids_read(
    read_txn: &redb::ReadTransaction,
    dataset_id: u64,
    pattern: &QuadPattern,
) -> Result<Option<[Option<u64>; 4]>, StoreError> {
    let subject = match &pattern.subject {
        Some(subject) => {
            let Some(id) =
                term_id_for_read_from_read(read_txn, dataset_id, &subject_as_term(subject)?)?
            else {
                return Ok(None);
            };
            Some(id)
        }
        None => None,
    };

    let predicate = match &pattern.predicate {
        Some(predicate) => {
            let Some(id) = term_id_for_read_from_read(
                read_txn,
                dataset_id,
                &Term::NamedNode(predicate.as_named_node().clone()),
            )?
            else {
                return Ok(None);
            };
            Some(id)
        }
        None => None,
    };

    let object = match &pattern.object {
        Some(object) => {
            let Some(id) =
                term_id_for_read_from_read(read_txn, dataset_id, &object_as_term(object)?)?
            else {
                return Ok(None);
            };
            Some(id)
        }
        None => None,
    };

    let graph_name = match &pattern.graph_name {
        Some(GraphName::DefaultGraph) => Some(DEFAULT_GRAPH_ID),
        Some(GraphName::NamedNode(node)) => {
            let Some(id) =
                term_id_for_read_from_read(read_txn, dataset_id, &Term::NamedNode(node.clone()))?
            else {
                return Ok(None);
            };
            Some(id)
        }
        Some(GraphName::BlankNode(node)) => {
            let Some(id) =
                term_id_for_read_from_read(read_txn, dataset_id, &Term::BlankNode(node.clone()))?
            else {
                return Ok(None);
            };
            Some(id)
        }
        None => None,
    };

    Ok(Some([subject, predicate, object, graph_name]))
}

fn term_id_for_write(
    write_txn: &redb::WriteTransaction,
    dataset_id: u64,
    term: &Term,
) -> Result<u64, StoreError> {
    let term_bytes = encode_term(term)?;
    let term_key = term_key(dataset_id, &term_bytes);

    {
        let term_to_id = write_txn.open_table(TERM_TO_ID_TABLE).map_err(map_redb)?;
        if let Some(existing) = term_to_id.get(term_key.as_slice()).map_err(map_redb)? {
            return Ok(existing.value());
        }
    }

    let next_term_id = {
        let mut counters = write_txn.open_table(NEXT_TERM_BY_DATASET_TABLE).map_err(map_redb)?;
        let next =
            counters.get(dataset_id).map_err(map_redb)?.map(|value| value.value()).unwrap_or(1);
        counters.insert(dataset_id, next + 1).map_err(map_redb)?;
        next
    };

    {
        let mut term_to_id = write_txn.open_table(TERM_TO_ID_TABLE).map_err(map_redb)?;
        term_to_id.insert(term_key.as_slice(), next_term_id).map_err(map_redb)?;
    }
    {
        let mut id_to_term = write_txn.open_table(ID_TO_TERM_TABLE).map_err(map_redb)?;
        let id_key = id_term_key(dataset_id, next_term_id);
        id_to_term.insert(id_key.as_slice(), term_bytes.as_slice()).map_err(map_redb)?;
    }

    Ok(next_term_id)
}

fn term_id_for_read_from_read(
    read_txn: &redb::ReadTransaction,
    dataset_id: u64,
    term: &Term,
) -> Result<Option<u64>, StoreError> {
    let term_bytes = encode_term(term)?;
    let key = term_key(dataset_id, &term_bytes);
    let table = read_txn.open_table(TERM_TO_ID_TABLE).map_err(map_redb)?;
    Ok(table.get(key.as_slice()).map_err(map_redb)?.map(|value| value.value()))
}

fn term_id_for_read_from_write(
    write_txn: &redb::WriteTransaction,
    dataset_id: u64,
    term: &Term,
) -> Result<Option<u64>, StoreError> {
    let term_bytes = encode_term(term)?;
    let key = term_key(dataset_id, &term_bytes);
    let table = write_txn.open_table(TERM_TO_ID_TABLE).map_err(map_redb)?;
    Ok(table.get(key.as_slice()).map_err(map_redb)?.map(|value| value.value()))
}

fn decode_quad_from_ids(
    read_txn: &redb::ReadTransaction,
    dataset_id: u64,
    ids: [u64; 4],
) -> Result<Quad, StoreError> {
    let subject_term = term_for_id(read_txn, dataset_id, ids[0])?;
    let predicate_term = term_for_id(read_txn, dataset_id, ids[1])?;
    let object_term = term_for_id(read_txn, dataset_id, ids[2])?;

    let subject = match subject_term {
        Term::NamedNode(node) => Subject::NamedNode(node),
        Term::BlankNode(node) => Subject::BlankNode(node),
        #[allow(unreachable_patterns)]
        _ => {
            return Err(StoreError::Backend(
                "invalid stored subject term kind for quad".to_string(),
            ));
        }
    };

    let predicate = match predicate_term {
        Term::NamedNode(node) => Predicate::new(node),
        #[allow(unreachable_patterns)]
        _ => {
            return Err(StoreError::Backend(
                "invalid stored predicate term kind for quad".to_string(),
            ));
        }
    };

    let object = match object_term {
        Term::NamedNode(node) => Object::NamedNode(node),
        Term::BlankNode(node) => Object::BlankNode(node),
        Term::Literal(literal) => Object::Literal(literal),
        #[allow(unreachable_patterns)]
        _ => {
            return Err(StoreError::Backend(
                "invalid stored object term kind for quad".to_string(),
            ));
        }
    };

    let graph_name = if ids[3] == DEFAULT_GRAPH_ID {
        GraphName::DefaultGraph
    } else {
        let graph_term = term_for_id(read_txn, dataset_id, ids[3])?;
        match graph_term {
            Term::NamedNode(node) => GraphName::NamedNode(node),
            Term::BlankNode(node) => GraphName::BlankNode(node),
            #[allow(unreachable_patterns)]
            _ => {
                return Err(StoreError::Backend(
                    "invalid stored graph term kind for quad".to_string(),
                ));
            }
        }
    };

    Ok(Quad::new(subject, predicate, object, graph_name))
}

fn term_for_id(
    read_txn: &redb::ReadTransaction,
    dataset_id: u64,
    id: u64,
) -> Result<Term, StoreError> {
    let key = id_term_key(dataset_id, id);
    let table = read_txn.open_table(ID_TO_TERM_TABLE).map_err(map_redb)?;
    let bytes = table.get(key.as_slice()).map_err(map_redb)?.ok_or_else(|| {
        StoreError::Backend(format!("missing term for id {id} in dataset {dataset_id}"))
    })?;
    decode_term(bytes.value())
}

fn range_for_dataset_prefix(dataset_id: u64, ordered: [Option<u64>; 4]) -> (Vec<u8>, Vec<u8>) {
    let prefix = prefix_len(ordered);
    let mut lower = Vec::with_capacity(40);
    let mut upper = Vec::with_capacity(40);
    lower.extend_from_slice(&dataset_id.to_be_bytes());
    upper.extend_from_slice(&dataset_id.to_be_bytes());

    for (i, entry) in ordered.iter().enumerate() {
        if i < prefix {
            let value = entry.expect("prefix fields are bound");
            lower.extend_from_slice(&value.to_be_bytes());
            upper.extend_from_slice(&value.to_be_bytes());
        } else {
            lower.extend_from_slice(&0_u64.to_be_bytes());
            upper.extend_from_slice(&u64::MAX.to_be_bytes());
        }
    }

    (lower, upper)
}

fn prefix_len(ordered: [Option<u64>; 4]) -> usize {
    let mut len = 0;
    while len < ordered.len() && ordered[len].is_some() {
        len += 1;
    }
    len
}

fn matches_pattern(ids: [u64; 4], pattern: &[Option<u64>; 4]) -> bool {
    pattern[0].is_none_or(|subject| ids[0] == subject)
        && pattern[1].is_none_or(|predicate| ids[1] == predicate)
        && pattern[2].is_none_or(|object| ids[2] == object)
        && pattern[3].is_none_or(|graph| ids[3] == graph)
}

fn term_key(dataset_id: u64, term_bytes: &[u8]) -> Vec<u8> {
    let mut key = Vec::with_capacity(8 + term_bytes.len());
    key.extend_from_slice(&dataset_id.to_be_bytes());
    key.extend_from_slice(term_bytes);
    key
}

fn id_term_key(dataset_id: u64, term_id: u64) -> Vec<u8> {
    let mut key = Vec::with_capacity(16);
    key.extend_from_slice(&dataset_id.to_be_bytes());
    key.extend_from_slice(&term_id.to_be_bytes());
    key
}

fn index_key(dataset_id: u64, ordered_ids: [u64; 4]) -> Vec<u8> {
    let mut key = Vec::with_capacity(40);
    key.extend_from_slice(&dataset_id.to_be_bytes());
    for id in ordered_ids {
        key.extend_from_slice(&id.to_be_bytes());
    }
    key
}

fn encode_ids(ids: [u64; 4]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(32);
    for id in ids {
        bytes.extend_from_slice(&id.to_be_bytes());
    }
    bytes
}

fn decode_ids(bytes: &[u8]) -> Result<[u64; 4], StoreError> {
    if bytes.len() != 32 {
        return Err(StoreError::Backend(format!(
            "invalid canonical id bytes length: expected 32, got {}",
            bytes.len()
        )));
    }
    Ok([
        u64::from_be_bytes(bytes[0..8].try_into().expect("slice length checked")),
        u64::from_be_bytes(bytes[8..16].try_into().expect("slice length checked")),
        u64::from_be_bytes(bytes[16..24].try_into().expect("slice length checked")),
        u64::from_be_bytes(bytes[24..32].try_into().expect("slice length checked")),
    ])
}

fn encode_term(term: &Term) -> Result<Vec<u8>, StoreError> {
    let mut out = Vec::new();
    match term {
        Term::NamedNode(node) => {
            out.push(1);
            push_string(&mut out, node.as_str());
        }
        Term::BlankNode(node) => {
            out.push(2);
            push_string(&mut out, node.label());
        }
        Term::Literal(literal) => {
            if let Some(lang) = literal.language() {
                out.push(4);
                push_string(&mut out, literal.lexical_form());
                push_string(&mut out, lang);
            } else if literal.datatype().as_str() == rdf_model::XSD_STRING_IRI {
                out.push(3);
                push_string(&mut out, literal.lexical_form());
            } else {
                out.push(5);
                push_string(&mut out, literal.lexical_form());
                push_string(&mut out, literal.datatype().as_str());
            }
        }
        #[allow(unreachable_patterns)]
        _ => return Err(StoreError::UnsupportedQuotedTriple),
    }
    Ok(out)
}

fn decode_term(bytes: &[u8]) -> Result<Term, StoreError> {
    if bytes.is_empty() {
        return Err(StoreError::Backend("empty term payload".to_string()));
    }
    let tag = bytes[0];
    let mut cursor = 1_usize;

    match tag {
        1 => {
            let iri = pull_string(bytes, &mut cursor)?;
            Ok(Term::NamedNode(NamedNode::new(iri)?))
        }
        2 => {
            let label = pull_string(bytes, &mut cursor)?;
            Ok(Term::BlankNode(rdf_model::BlankNode::new(label)?))
        }
        3 => {
            let lexical = pull_string(bytes, &mut cursor)?;
            Ok(Term::Literal(Literal::new_simple(lexical)))
        }
        4 => {
            let lexical = pull_string(bytes, &mut cursor)?;
            let lang = pull_string(bytes, &mut cursor)?;
            Ok(Term::Literal(Literal::new_language_tagged(lexical, lang)?))
        }
        5 => {
            let lexical = pull_string(bytes, &mut cursor)?;
            let datatype = pull_string(bytes, &mut cursor)?;
            Ok(Term::Literal(Literal::new_typed(lexical, NamedNode::new(datatype)?)?))
        }
        _ => Err(StoreError::Backend(format!("unsupported term tag {tag}"))),
    }
}

fn push_string(buffer: &mut Vec<u8>, value: &str) {
    let len: u32 = value.len().try_into().expect("string length must fit in u32");
    buffer.extend_from_slice(&len.to_be_bytes());
    buffer.extend_from_slice(value.as_bytes());
}

fn pull_string(bytes: &[u8], cursor: &mut usize) -> Result<String, StoreError> {
    if *cursor + 4 > bytes.len() {
        return Err(StoreError::Backend("truncated string length".to_string()));
    }
    let len =
        u32::from_be_bytes(bytes[*cursor..*cursor + 4].try_into().expect("slice length checked"))
            as usize;
    *cursor += 4;

    if *cursor + len > bytes.len() {
        return Err(StoreError::Backend("truncated string payload".to_string()));
    }

    let value = String::from_utf8(bytes[*cursor..*cursor + len].to_vec())
        .map_err(|error| StoreError::Backend(format!("invalid utf8 in term payload: {error}")))?;
    *cursor += len;
    Ok(value)
}

fn map_redb<E: std::fmt::Display>(error: E) -> StoreError {
    StoreError::Backend(format!("redb error: {error}"))
}
