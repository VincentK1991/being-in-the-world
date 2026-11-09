//! Turning store answers into compact, agent-friendly values: terms in SPARQL syntax (so they can
//! be pasted into the next query), graph kinds, and prefixes.
use serde_json::Value;
use std::collections::BTreeMap;

pub const DERIVED: &str = "urn:rdftool:derived:";
pub const INFERENCE: &str = "urn:rdftool:derived:inference:";
pub const SHACL: &str = "urn:rdftool:derived:shacl:";
const XSD: &str = "http://www.w3.org/2001/XMLSchema#";

/// Namespaces of the W3C vocabularies; their terms always "exist", so they are never reported as
/// unknown.
pub const VOCABULARIES: &[&str] = &[
    "http://www.w3.org/1999/02/22-rdf-syntax-ns#",
    "http://www.w3.org/2000/01/rdf-schema#",
    "http://www.w3.org/2002/07/owl#",
    "http://www.w3.org/2001/XMLSchema#",
    "http://www.w3.org/ns/shacl#",
];

const KNOWN_PREFIXES: &[(&str, &str)] = &[
    ("rdf", "http://www.w3.org/1999/02/22-rdf-syntax-ns#"),
    ("rdfs", "http://www.w3.org/2000/01/rdf-schema#"),
    ("owl", "http://www.w3.org/2002/07/owl#"),
    ("xsd", "http://www.w3.org/2001/XMLSchema#"),
    ("sh", "http://www.w3.org/ns/shacl#"),
    ("skos", "http://www.w3.org/2004/02/skos/core#"),
    ("dcterms", "http://purl.org/dc/terms/"),
    ("dc", "http://purl.org/dc/elements/1.1/"),
    ("foaf", "http://xmlns.com/foaf/0.1/"),
    ("schema", "https://schema.org/"),
    ("prov", "http://www.w3.org/ns/prov#"),
    ("search", "urn:rdftool:search:"),
];

/// What a named graph holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GraphKind {
    Asserted,
    Inferred(String),
    ValidationReport(String),
    Metadata,
    OtherDerived,
}
impl GraphKind {
    pub fn of(graph: &str) -> Self {
        if graph.ends_with(":metadata") && graph.starts_with(DERIVED) {
            GraphKind::Metadata
        } else if let Some(job) = graph.strip_prefix(INFERENCE) {
            GraphKind::Inferred(job.to_owned())
        } else if let Some(job) = graph.strip_prefix(SHACL) {
            GraphKind::ValidationReport(job.to_owned())
        } else if graph.starts_with(DERIVED) {
            GraphKind::OtherDerived
        } else {
            GraphKind::Asserted
        }
    }
    pub fn label(&self) -> String {
        match self {
            GraphKind::Asserted => "asserted".into(),
            GraphKind::Inferred(job) => format!("inferred by job {job}"),
            GraphKind::ValidationReport(job) => format!("SHACL report of job {job}"),
            GraphKind::Metadata => "derived-graph metadata".into(),
            GraphKind::OtherDerived => "derived".into(),
        }
    }
    pub fn name(&self) -> &'static str {
        match self {
            GraphKind::Asserted => "asserted",
            GraphKind::Inferred(_) => "inferred",
            GraphKind::ValidationReport(_) => "validation-report",
            GraphKind::Metadata => "metadata",
            GraphKind::OtherDerived => "derived",
        }
    }
}

/// Where a triple came from, as shown next to it: `asserted <g>`, `inferred <job>`, ...
pub fn origin(graph: Option<&str>) -> String {
    match graph {
        None => "default graph".into(),
        Some(g) => match GraphKind::of(g) {
            GraphKind::Asserted => format!("asserted <{g}>"),
            GraphKind::Inferred(job) => format!("inferred {job}"),
            kind => format!("{} <{g}>", kind.name()),
        },
    }
}

/// A SPARQL JSON term, written as it would be in a query.
pub fn term(value: &Value) -> Value {
    let text = value["value"].as_str().unwrap_or_default();
    Value::String(match value["type"].as_str().unwrap_or_default() {
        "uri" => format!("<{text}>"),
        "bnode" => format!("_:{text}"),
        _ => {
            let quoted = serde_json::to_string(text).unwrap_or_else(|_| format!("\"{text}\""));
            if let Some(lang) = value.get("xml:lang").and_then(Value::as_str) {
                format!("{quoted}@{lang}")
            } else {
                match value["datatype"].as_str() {
                    None => quoted,
                    Some(dt) if dt == format!("{XSD}string") => quoted,
                    Some(dt)
                        if [
                            format!("{XSD}integer"),
                            format!("{XSD}decimal"),
                            format!("{XSD}boolean"),
                        ]
                        .iter()
                        .any(|d| d == dt)
                            && plain_number_or_boolean(text) =>
                    {
                        text.to_owned()
                    }
                    Some(dt) => format!("{quoted}^^<{dt}>"),
                }
            }
        }
    })
}
fn plain_number_or_boolean(text: &str) -> bool {
    text == "true"
        || text == "false"
        || (!text.is_empty()
            && text.chars().all(|c| c.is_ascii_digit() || c == '.' || c == '-' || c == '+')
            && text.chars().any(|c| c.is_ascii_digit()))
}

/// The plain value of a binding (IRI or lexical form), for internal use.
pub fn plain<'a>(row: &'a Value, var: &str) -> Option<&'a str> {
    row.get(var)?.get("value")?.as_str()
}

pub fn bindings(results: &Value) -> &[Value] {
    results["results"]["bindings"].as_array().map(Vec::as_slice).unwrap_or_default()
}

/// Split an IRI into namespace and local name.
pub fn split(iri: &str) -> (&str, &str) {
    let cut = iri.rfind('#').or_else(|| iri.rfind('/')).or_else(|| iri.rfind(':'));
    match cut {
        Some(i) if i + 1 < iri.len() && !iri[..=i].ends_with("//") => (&iri[..=i], &iri[i + 1..]),
        _ => (iri, ""),
    }
}

fn valid_local(local: &str) -> bool {
    !local.is_empty()
        && !local.ends_with('.')
        && local.chars().next().is_some_and(|c| c.is_alphanumeric() || c == '_')
        && local.chars().all(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '.'))
}

/// Prefixes for a set of IRIs: the well-known names where they apply, otherwise a short name
/// made from the namespace.
#[derive(Clone, Debug, Default)]
pub struct Prefixes {
    /// prefix -> namespace
    pub map: BTreeMap<String, String>,
}
impl Prefixes {
    pub fn build<'a>(iris: impl IntoIterator<Item = &'a str>) -> Self {
        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        for iri in iris {
            let (ns, local) = split(iri);
            if !local.is_empty() {
                *counts.entry(ns.to_owned()).or_default() += 1;
            }
        }
        let mut map = BTreeMap::new();
        let mut namespaces: Vec<_> = counts.into_iter().collect();
        namespaces.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        for (ns, _) in namespaces.into_iter().take(40) {
            let known = KNOWN_PREFIXES.iter().find(|(_, n)| *n == ns).map(|(p, _)| p.to_string());
            let base = known.unwrap_or_else(|| derive_name(&ns));
            let mut name = base.clone();
            let mut n = 2;
            while map.contains_key(&name)
                || (KNOWN_PREFIXES.iter().any(|(p, k)| *p == name && *k != ns))
            {
                name = format!("{base}{n}");
                n += 1;
            }
            map.insert(name, ns);
        }
        Self { map }
    }
    pub fn curie(&self, iri: &str) -> String {
        let (ns, local) = split(iri);
        if valid_local(local)
            && let Some((prefix, _)) = self.map.iter().find(|(_, n)| n.as_str() == ns)
        {
            return format!("{prefix}:{local}");
        }
        format!("<{iri}>")
    }
    /// Expand `prefix:local` or `<iri>` or a bare IRI.
    pub fn expand(&self, text: &str) -> String {
        let text = text.trim();
        if let Some(inner) = text.strip_prefix('<').and_then(|t| t.strip_suffix('>')) {
            return inner.to_owned();
        }
        if let Some((prefix, local)) = text.split_once(':')
            && let Some(ns) = self.map.get(prefix).cloned().or_else(|| {
                KNOWN_PREFIXES.iter().find(|(p, _)| *p == prefix).map(|(_, n)| n.to_string())
            })
            && !local.starts_with("//")
        {
            return format!("{ns}{local}");
        }
        text.to_owned()
    }
    pub fn block(&self) -> String {
        self.map.iter().map(|(p, n)| format!("PREFIX {p}: <{n}>\n")).collect()
    }
}

fn derive_name(ns: &str) -> String {
    let trimmed = ns.trim_end_matches(['#', '/', ':']);
    let segment = trimmed.rsplit(['/', ':', '#']).find(|s| !s.is_empty()).unwrap_or("ns");
    let mut name: String = segment
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_lowercase();
    name.truncate(12);
    if name.is_empty() || !name.starts_with(|c: char| c.is_ascii_alphabetic()) {
        name = format!("ns{name}");
    }
    name
}

/// A few known terms that look like `missing`: same local name ignoring case, or a small edit
/// distance.
pub fn suggest<'a>(missing: &str, known: impl IntoIterator<Item = &'a String>) -> Vec<String> {
    let (_, local) = split(missing);
    let target = local.to_lowercase();
    let mut scored: Vec<(usize, &String)> = known
        .into_iter()
        .filter_map(|candidate| {
            let (_, l) = split(candidate);
            let distance = edit_distance(&target, &l.to_lowercase());
            let limit = (target.chars().count() / 3).max(2);
            (distance <= limit).then_some((distance, candidate))
        })
        .collect();
    scored.sort();
    scored.dedup_by(|a, b| a.1 == b.1);
    scored.into_iter().take(3).map(|(_, c)| c.clone()).collect()
}

fn edit_distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut current = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != *cb);
            current.push((previous[j] + cost).min(previous[j + 1] + 1).min(current[j] + 1));
        }
        previous = current;
    }
    previous[b.len()]
}

/// `FROM` and `FROM NAMED` for each graph, for queries this server writes itself.
pub fn from_clause(graphs: &[String], named_only: bool) -> String {
    let mut clause = String::new();
    for g in graphs {
        if !named_only {
            clause.push_str(&format!("FROM <{g}>\n"));
        }
        clause.push_str(&format!("FROM NAMED <{g}>\n"));
    }
    clause
}

/// Accept `<iri>` or a bare absolute IRI; refuse anything that could break out of `<...>`.
pub fn iri(text: &str) -> anyhow::Result<String> {
    let inner = text.trim().trim_start_matches('<').trim_end_matches('>');
    anyhow::ensure!(
        !inner.is_empty()
            && inner.contains(':')
            && !inner.chars().any(|c| c.is_whitespace()
                || matches!(c, '<' | '>' | '"' | '{' | '}' | '|' | '^' | '`' | '\\')),
        "{text:?} is not an IRI; pass a full IRI such as <https://example.org/x>"
    );
    Ok(inner.to_owned())
}

/// A SPARQL string literal for user text.
pub fn string_literal(text: &str) -> String {
    serde_json::to_string(text).unwrap_or_else(|_| "\"\"".into())
}
