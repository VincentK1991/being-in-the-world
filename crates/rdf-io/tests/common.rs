use rdf_model::{GraphName, Object, Quad, Subject};
use std::collections::{BTreeSet, HashMap, HashSet};

pub fn quads_isomorphic(left: &[Quad], right: &[Quad]) -> bool {
    if left.len() != right.len() {
        return false;
    }

    let left_bnodes = collect_bnode_labels(left);
    let right_bnodes = collect_bnode_labels(right);
    if left_bnodes.len() != right_bnodes.len() {
        return false;
    }

    let right_signatures = quad_signatures(right, &HashMap::new());
    if left_bnodes.is_empty() {
        return quad_signatures(left, &HashMap::new()) == right_signatures;
    }

    let mut mapping = HashMap::new();
    let mut used = HashSet::new();
    let left_labels: Vec<String> = left_bnodes.into_iter().collect();
    let right_labels: Vec<String> = right_bnodes.into_iter().collect();
    backtrack_mapping(
        &left_labels,
        &right_labels,
        0,
        &mut mapping,
        &mut used,
        left,
        &right_signatures,
    )
}

fn backtrack_mapping(
    left_labels: &[String],
    right_labels: &[String],
    index: usize,
    mapping: &mut HashMap<String, String>,
    used: &mut HashSet<String>,
    left_quads: &[Quad],
    right_signatures: &BTreeSet<String>,
) -> bool {
    if index == left_labels.len() {
        return quad_signatures(left_quads, mapping) == *right_signatures;
    }

    let current_left = &left_labels[index];
    for candidate in right_labels {
        if used.contains(candidate) {
            continue;
        }
        mapping.insert(current_left.clone(), candidate.clone());
        used.insert(candidate.clone());
        if backtrack_mapping(
            left_labels,
            right_labels,
            index + 1,
            mapping,
            used,
            left_quads,
            right_signatures,
        ) {
            return true;
        }
        used.remove(candidate);
        mapping.remove(current_left);
    }
    false
}

fn collect_bnode_labels(quads: &[Quad]) -> BTreeSet<String> {
    let mut labels = BTreeSet::new();
    for quad in quads {
        if let Subject::BlankNode(node) = quad.subject() {
            labels.insert(node.label().to_string());
        }
        if let Object::BlankNode(node) = quad.object() {
            labels.insert(node.label().to_string());
        }
        if let GraphName::BlankNode(node) = quad.graph_name() {
            labels.insert(node.label().to_string());
        }
    }
    labels
}

fn quad_signatures(quads: &[Quad], mapping: &HashMap<String, String>) -> BTreeSet<String> {
    quads.iter().map(|quad| quad_signature(quad, mapping)).collect()
}

fn quad_signature(quad: &Quad, mapping: &HashMap<String, String>) -> String {
    format!(
        "{} {} {} {}",
        subject_signature(quad.subject(), mapping),
        iri_signature(quad.predicate().as_named_node().as_str()),
        object_signature(quad.object(), mapping),
        graph_signature(quad.graph_name(), mapping)
    )
}

fn subject_signature(subject: &Subject, mapping: &HashMap<String, String>) -> String {
    match subject {
        Subject::NamedNode(node) => iri_signature(node.as_str()),
        Subject::BlankNode(node) => bnode_signature(node.label(), mapping),
    }
}

fn object_signature(object: &Object, mapping: &HashMap<String, String>) -> String {
    match object {
        Object::NamedNode(node) => iri_signature(node.as_str()),
        Object::BlankNode(node) => bnode_signature(node.label(), mapping),
        Object::Literal(literal) => format!(
            "\"{}\"^^{}@{}",
            literal.lexical_form(),
            literal.datatype().as_str(),
            literal.language().unwrap_or("")
        ),
    }
}

fn graph_signature(graph: &GraphName, mapping: &HashMap<String, String>) -> String {
    match graph {
        GraphName::DefaultGraph => String::from("default"),
        GraphName::NamedNode(node) => iri_signature(node.as_str()),
        GraphName::BlankNode(node) => bnode_signature(node.label(), mapping),
    }
}

fn iri_signature(iri: &str) -> String {
    format!("<{}>", iri)
}

fn bnode_signature(label: &str, mapping: &HashMap<String, String>) -> String {
    let mapped = mapping.get(label).map_or(label, String::as_str);
    format!("_:{}", mapped)
}
