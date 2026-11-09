//! In-memory quad storage and pattern query APIs.

#![forbid(unsafe_code)]

use rdf_model::{
    Dictionary, DictionaryError, GraphName, InMemoryDictionary, ModelError, Object, Predicate,
    Quad, Subject, Term, TermId,
};
use std::collections::{BTreeMap, HashMap, HashSet};
use thiserror::Error;

const DEFAULT_GRAPH_ID: u64 = 0;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct QuadPattern {
    pub subject: Option<Subject>,
    pub predicate: Option<Predicate>,
    pub object: Option<Object>,
    pub graph_name: Option<GraphName>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StoreStats {
    pub quads: usize,
    pub distinct_subjects: usize,
    pub distinct_predicates: usize,
    pub distinct_objects: usize,
    pub distinct_graphs: usize,
}

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("dictionary error: {0}")]
    Dictionary(#[from] DictionaryError),
    #[error("model error: {0}")]
    Model(#[from] ModelError),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("rdf-star quoted triples are not supported in MemQuadStorage")]
    UnsupportedQuotedTriple,
    #[error("{0}")]
    Backend(String),
}

#[derive(Debug)]
pub struct MemQuadStorage {
    dictionary: InMemoryDictionary,
    term_to_id: HashMap<Term, TermId>,
    spog: BTreeMap<[u64; 4], [u64; 4]>,
    posg: BTreeMap<[u64; 4], [u64; 4]>,
    ospg: BTreeMap<[u64; 4], [u64; 4]>,
    quads_by_key: BTreeMap<[u64; 4], Quad>,
}

impl Default for MemQuadStorage {
    fn default() -> Self {
        Self {
            dictionary: InMemoryDictionary::new(),
            term_to_id: HashMap::new(),
            spog: BTreeMap::new(),
            posg: BTreeMap::new(),
            ospg: BTreeMap::new(),
            quads_by_key: BTreeMap::new(),
        }
    }
}

impl MemQuadStorage {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.quads_by_key.is_empty()
    }

    pub fn len(&self) -> usize {
        self.quads_by_key.len()
    }

    pub fn insert(&mut self, quad: Quad) -> Result<bool, StoreError> {
        let key = self.encode_quad_ids(&quad)?;
        if self.quads_by_key.contains_key(&key) {
            return Ok(false);
        }

        self.spog.insert(key, key);
        self.posg.insert([key[1], key[2], key[0], key[3]], key);
        self.ospg.insert([key[2], key[0], key[1], key[3]], key);
        self.quads_by_key.insert(key, quad);
        Ok(true)
    }

    pub fn remove(&mut self, quad: &Quad) -> Result<bool, StoreError> {
        let Some(key) = self.resolve_quad_ids(quad)? else {
            return Ok(false);
        };

        if self.quads_by_key.remove(&key).is_none() {
            return Ok(false);
        }

        self.spog.remove(&key);
        self.posg.remove(&[key[1], key[2], key[0], key[3]]);
        self.ospg.remove(&[key[2], key[0], key[1], key[3]]);
        Ok(true)
    }

    pub fn all_quads(&self) -> Vec<Quad> {
        self.quads_by_key.values().cloned().collect()
    }

    pub fn quads_matching(&self, pattern: &QuadPattern) -> Vec<Quad> {
        let Some(pattern_ids) = self.resolve_pattern_ids(pattern).ok().flatten() else {
            return Vec::new();
        };

        let mut matches = Vec::new();
        let index = SelectIndex::best_for(&pattern_ids);

        for canonical_key in self.iter_candidates(index, &pattern_ids) {
            if !matches_pattern(canonical_key, &pattern_ids) {
                continue;
            }
            if let Some(quad) = self.quads_by_key.get(canonical_key) {
                matches.push(quad.clone());
            }
        }

        matches
    }

    pub fn stats(&self) -> StoreStats {
        let mut subjects = HashSet::new();
        let mut predicates = HashSet::new();
        let mut objects = HashSet::new();
        let mut graphs = HashSet::new();

        for key in self.quads_by_key.keys() {
            subjects.insert(key[0]);
            predicates.insert(key[1]);
            objects.insert(key[2]);
            graphs.insert(key[3]);
        }

        StoreStats {
            quads: self.quads_by_key.len(),
            distinct_subjects: subjects.len(),
            distinct_predicates: predicates.len(),
            distinct_objects: objects.len(),
            distinct_graphs: graphs.len(),
        }
    }

    fn iter_candidates<'a>(
        &'a self,
        index: SelectIndex,
        pattern_ids: &'a [Option<u64>; 4],
    ) -> Box<dyn Iterator<Item = &'a [u64; 4]> + 'a> {
        match index {
            SelectIndex::Spog => {
                Box::new(index_iter(&self.spog, &pattern_ids_in_order(pattern_ids, [0, 1, 2, 3])))
            }
            SelectIndex::Posg => {
                Box::new(index_iter(&self.posg, &pattern_ids_in_order(pattern_ids, [1, 2, 0, 3])))
            }
            SelectIndex::Ospg => {
                Box::new(index_iter(&self.ospg, &pattern_ids_in_order(pattern_ids, [2, 0, 1, 3])))
            }
        }
    }

    fn encode_quad_ids(&mut self, quad: &Quad) -> Result<[u64; 4], StoreError> {
        let subject = self.encode_subject(quad.subject())?;
        let predicate = self.encode_predicate(quad.predicate())?;
        let object = self.encode_object(quad.object())?;
        let graph = self.encode_graph_name(quad.graph_name())?;
        Ok([subject, predicate, object, graph])
    }

    fn resolve_quad_ids(&self, quad: &Quad) -> Result<Option<[u64; 4]>, StoreError> {
        let Some(subject) = self.resolve_subject_id(quad.subject())? else {
            return Ok(None);
        };
        let Some(predicate) = self.resolve_predicate_id(quad.predicate())? else {
            return Ok(None);
        };
        let Some(object) = self.resolve_object_id(quad.object())? else {
            return Ok(None);
        };
        let Some(graph) = self.resolve_graph_name_id(quad.graph_name())? else {
            return Ok(None);
        };
        Ok(Some([subject, predicate, object, graph]))
    }

    fn resolve_pattern_ids(
        &self,
        pattern: &QuadPattern,
    ) -> Result<Option<[Option<u64>; 4]>, StoreError> {
        let subject = match &pattern.subject {
            Some(subject) => {
                let Some(id) = self.resolve_subject_id(subject)? else {
                    return Ok(None);
                };
                Some(id)
            }
            None => None,
        };

        let predicate = match &pattern.predicate {
            Some(predicate) => {
                let Some(id) = self.resolve_predicate_id(predicate)? else {
                    return Ok(None);
                };
                Some(id)
            }
            None => None,
        };

        let object = match &pattern.object {
            Some(object) => {
                let Some(id) = self.resolve_object_id(object)? else {
                    return Ok(None);
                };
                Some(id)
            }
            None => None,
        };

        let graph = match &pattern.graph_name {
            Some(graph) => {
                let Some(id) = self.resolve_graph_name_id(graph)? else {
                    return Ok(None);
                };
                Some(id)
            }
            None => None,
        };

        Ok(Some([subject, predicate, object, graph]))
    }

    fn encode_subject(&mut self, subject: &Subject) -> Result<u64, StoreError> {
        Ok(self.encode_term(&subject_as_term(subject)?)?.into_raw())
    }

    fn encode_predicate(&mut self, predicate: &Predicate) -> Result<u64, StoreError> {
        Ok(self.encode_term(&Term::NamedNode(predicate.as_named_node().clone()))?.into_raw())
    }

    fn encode_object(&mut self, object: &Object) -> Result<u64, StoreError> {
        Ok(self.encode_term(&object_as_term(object)?)?.into_raw())
    }

    fn encode_graph_name(&mut self, graph_name: &GraphName) -> Result<u64, StoreError> {
        match graph_name {
            GraphName::DefaultGraph => Ok(DEFAULT_GRAPH_ID),
            GraphName::NamedNode(node) => {
                Ok(self.encode_term(&Term::NamedNode(node.clone()))?.into_raw())
            }
            GraphName::BlankNode(node) => {
                Ok(self.encode_term(&Term::BlankNode(node.clone()))?.into_raw())
            }
        }
    }

    fn encode_term(&mut self, term: &Term) -> Result<TermId, StoreError> {
        if let Some(existing) = self.term_to_id.get(term).copied() {
            return Ok(existing);
        }

        let id = self.dictionary.encode(term)?;
        self.term_to_id.insert(term.clone(), id);
        Ok(id)
    }

    fn resolve_subject_id(&self, subject: &Subject) -> Result<Option<u64>, StoreError> {
        Ok(self.resolve_term_id(&subject_as_term(subject)?).map(TermId::into_raw))
    }

    fn resolve_predicate_id(&self, predicate: &Predicate) -> Result<Option<u64>, StoreError> {
        Ok(self
            .resolve_term_id(&Term::NamedNode(predicate.as_named_node().clone()))
            .map(TermId::into_raw))
    }

    fn resolve_object_id(&self, object: &Object) -> Result<Option<u64>, StoreError> {
        Ok(self.resolve_term_id(&object_as_term(object)?).map(TermId::into_raw))
    }

    fn resolve_graph_name_id(&self, graph_name: &GraphName) -> Result<Option<u64>, StoreError> {
        match graph_name {
            GraphName::DefaultGraph => Ok(Some(DEFAULT_GRAPH_ID)),
            GraphName::NamedNode(node) => {
                Ok(self.resolve_term_id(&Term::NamedNode(node.clone())).map(TermId::into_raw))
            }
            GraphName::BlankNode(node) => {
                Ok(self.resolve_term_id(&Term::BlankNode(node.clone())).map(TermId::into_raw))
            }
        }
    }

    fn resolve_term_id(&self, term: &Term) -> Option<TermId> {
        self.term_to_id.get(term).copied()
    }
}

pub(crate) fn subject_as_term(subject: &Subject) -> Result<Term, StoreError> {
    match subject {
        Subject::NamedNode(node) => Ok(Term::NamedNode(node.clone())),
        Subject::BlankNode(node) => Ok(Term::BlankNode(node.clone())),
        #[allow(unreachable_patterns)]
        _ => Err(StoreError::UnsupportedQuotedTriple),
    }
}

pub(crate) fn object_as_term(object: &Object) -> Result<Term, StoreError> {
    match object {
        Object::NamedNode(node) => Ok(Term::NamedNode(node.clone())),
        Object::BlankNode(node) => Ok(Term::BlankNode(node.clone())),
        Object::Literal(literal) => Ok(Term::Literal(literal.clone())),
        #[allow(unreachable_patterns)]
        _ => Err(StoreError::UnsupportedQuotedTriple),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SelectIndex {
    Spog,
    Posg,
    Ospg,
}

impl SelectIndex {
    fn best_for(pattern_ids: &[Option<u64>; 4]) -> Self {
        let spog = prefix_len(&pattern_ids_in_order(pattern_ids, [0, 1, 2, 3]));
        let posg = prefix_len(&pattern_ids_in_order(pattern_ids, [1, 2, 0, 3]));
        let ospg = prefix_len(&pattern_ids_in_order(pattern_ids, [2, 0, 1, 3]));

        if posg > spog && posg >= ospg {
            Self::Posg
        } else if ospg > spog && ospg >= posg {
            Self::Ospg
        } else {
            Self::Spog
        }
    }
}

fn matches_pattern(key: &[u64; 4], pattern_ids: &[Option<u64>; 4]) -> bool {
    pattern_ids[0].is_none_or(|subject| key[0] == subject)
        && pattern_ids[1].is_none_or(|predicate| key[1] == predicate)
        && pattern_ids[2].is_none_or(|object| key[2] == object)
        && pattern_ids[3].is_none_or(|graph| key[3] == graph)
}

fn pattern_ids_in_order(pattern_ids: &[Option<u64>; 4], order: [usize; 4]) -> [Option<u64>; 4] {
    [pattern_ids[order[0]], pattern_ids[order[1]], pattern_ids[order[2]], pattern_ids[order[3]]]
}

fn prefix_len(ordered: &[Option<u64>; 4]) -> usize {
    let mut prefix = 0;
    while prefix < ordered.len() && ordered[prefix].is_some() {
        prefix += 1;
    }
    prefix
}

fn index_iter<'a>(
    index: &'a BTreeMap<[u64; 4], [u64; 4]>,
    ordered_pattern: &[Option<u64>; 4],
) -> Box<dyn Iterator<Item = &'a [u64; 4]> + 'a> {
    let prefix = prefix_len(ordered_pattern);
    if prefix == 0 {
        return Box::new(index.values());
    }

    let mut lower = [0_u64; 4];
    let mut upper = [u64::MAX; 4];
    for i in 0..prefix {
        let value = ordered_pattern[i].expect("prefix terms must be bound");
        lower[i] = value;
        upper[i] = value;
    }
    Box::new(index.range(lower..=upper).map(|(_, canonical)| canonical))
}

pub mod comparison;
pub mod engine;
mod policy;
mod redb_store;
pub use oxigraph;

pub use redb_store::RedbQuadStorage;

pub mod work_budget;
