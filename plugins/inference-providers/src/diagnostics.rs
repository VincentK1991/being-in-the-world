//! Diagnostics for the finite native OWL rule profile, not an OWL structural validator.
use anyhow::{Result, ensure};
use rdf_store::oxigraph::{
    io::{RdfFormat, RdfParser},
    model::Term,
};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
const O: &str = "http://www.w3.org/2002/07/owl#";
const R: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";
pub fn inspect(data: &str) -> Result<Value> {
    ensure!(data.len() <= 16 * 1024 * 1024, "diagnostic input exceeds 16 MiB");
    let mut props: BTreeMap<(String, String), Vec<Term>> = BTreeMap::new();
    let mut unchecked = BTreeSet::new();
    let mut heads = Vec::new();
    let mut issues = BTreeSet::new();
    for (count, q) in
        RdfParser::from_format(RdfFormat::NTriples).for_reader(data.as_bytes()).enumerate()
    {
        ensure!(count < 100_000, "diagnostic input exceeds 100000 triples");
        let q = q?;
        if let Term::Literal(l) = &q.object {
            let kind = l.datatype().as_str();
            if l.language().is_none()
                && ![
                    "string",
                    "integer",
                    "int",
                    "long",
                    "short",
                    "byte",
                    "decimal",
                    "float",
                    "double",
                    "boolean",
                    "nonNegativeInteger",
                    "positiveInteger",
                    "nonPositiveInteger",
                    "negativeInteger",
                    "unsignedLong",
                    "unsignedInt",
                    "unsignedShort",
                    "unsignedByte",
                ]
                .iter()
                .any(|name| kind == format!("http://www.w3.org/2001/XMLSchema#{name}"))
            {
                unchecked.insert(kind.to_owned());
            }
        }
        let p = q.predicate.as_str();
        if [
            "intersectionOf",
            "unionOf",
            "oneOf",
            "members",
            "distinctMembers",
            "propertyChainAxiom",
            "hasKey",
        ]
        .iter()
        .any(|k| p == format!("{O}{k}"))
        {
            heads.push(q.object.to_string());
        }
        if p == format!("{R}type") && q.object.to_string() == format!("<{O}ReflexiveProperty>") {
            issues.insert("unsupported-reflexive-property".into());
        }
        if p == format!("{O}imports") {
            issues.insert("unsupported-ontology-imports".to_string());
        }
        if [
            "hasSelf",
            "disjointUnionOf",
            "onDatatype",
            "minCardinality",
            "cardinality",
            "minQualifiedCardinality",
            "qualifiedCardinality",
            "onDataRange",
            "datatypeComplementOf",
            "withRestrictions",
        ]
        .iter()
        .any(|k| p == format!("{O}{k}"))
        {
            issues.insert(format!("unsupported-logical-construct:{p}"));
        }
        if ["maxCardinality", "maxQualifiedCardinality"].iter().any(|k| p == format!("{O}{k}")) {
            match &q.object {
                Term::Literal(l)
                    if l.datatype().as_str()
                        == "http://www.w3.org/2001/XMLSchema#nonNegativeInteger" =>
                {
                    match l.value().parse::<u64>() {
                        Ok(0..=1) => {}
                        Ok(_) => {
                            issues.insert("unsupported-cardinality-greater-than-one".into());
                        }
                        Err(_) => {
                            issues.insert("malformed-cardinality".into());
                        }
                    }
                }
                _ => {
                    issues.insert("incorrect-cardinality-datatype".into());
                }
            }
        }
        props.entry((q.subject.to_string(), p.to_owned())).or_default().push(q.object);
    }
    for mut head in heads {
        let mut seen = BTreeSet::new();
        while head != format!("<{R}nil>") {
            if !seen.insert(head.clone()) {
                issues.insert("malformed-list-cycle".into());
                break;
            }
            let first = props.get(&(head.clone(), format!("{R}first")));
            let rest = props.get(&(head.clone(), format!("{R}rest")));
            match (first, rest) {
                (Some(a), Some(b)) if a.len() == 1 && b.len() == 1 => head = b[0].to_string(),
                _ => {
                    issues.insert("malformed-list-arcs".into());
                    break;
                }
            }
        }
    }
    let truncated = issues.len() > 256;
    Ok(json!({"issues":issues.into_iter().take(256).collect::<Vec<_>>(),"truncated":truncated,
        "datatype_checks":"partial","unchecked_datatypes":unchecked.into_iter().take(256).collect::<Vec<_>>(),"consistency":"not-proved","profile_complete":false,
        "note":"Native RL rule materialization with scoped axioms; datatype entailment is not implemented"}))
}
