//! The in-process OWL 2 RL engine: rules, clashes with their premises, the output policy,
//! explanations through intermediate deductions, and limits. The W3C suites live in
//! `tests/conformance/entailment-owlrl-native.json`.
use anyhow::Result;
use rdf_inference_providers::{NativeFactory, NativeOwlRl};
use rdf_plugin_inference::{Inconsistent, Reasoner, ReasonerFactory};
use serde_json::{Value, json};
use std::collections::BTreeSet;

const OWL: &str = "http://www.w3.org/2002/07/owl#";
const RDF: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";
const RDFS: &str = "http://www.w3.org/2000/01/rdf-schema#";

fn reasoner() -> Result<std::sync::Arc<dyn Reasoner>> {
    NativeFactory.create(&json!({}))
}
/// Turtle-ish shorthand: `ex:` is `urn:ex:`, `owl:`/`rdf:`/`rdfs:` expand, one triple per line.
fn nt(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(|line| {
            let terms: Vec<String> = line
                .trim_end_matches(" .")
                .split(' ')
                .map(|t| {
                    if let Some(rest) = t.strip_prefix("ex:") {
                        format!("<urn:ex:{rest}>")
                    } else if let Some(rest) = t.strip_prefix("owl:") {
                        format!("<{OWL}{rest}>")
                    } else if let Some(rest) = t.strip_prefix("rdfs:") {
                        format!("<{RDFS}{rest}>")
                    } else if let Some(rest) = t.strip_prefix("rdf:") {
                        format!("<{RDF}{rest}>")
                    } else if t == "a" {
                        format!("<{RDF}type>")
                    } else {
                        t.to_owned()
                    }
                })
                .collect();
            format!("{} .", terms.join(" "))
        })
        .collect::<Vec<_>>()
        .join("\n")
}
fn derived(text: &str) -> Result<BTreeSet<String>> {
    let (out, _) = reasoner()?.infer(NativeOwlRl::PROFILE, &nt(text))?;
    Ok(out.lines().map(str::to_owned).collect())
}
fn holds(found: &BTreeSet<String>, line: &str) -> bool {
    found.contains(&nt(line))
}
fn clashes(text: &str) -> Vec<Value> {
    match reasoner().and_then(|r| r.infer(NativeOwlRl::PROFILE, &nt(text))) {
        Ok(_) => Vec::new(),
        Err(error) => {
            error.downcast_ref::<Inconsistent>().map(|i| i.clashes.clone()).unwrap_or_default()
        }
    }
}
const LIST_AB: &str =
    "_:l1 rdf:first ex:A\n_:l1 rdf:rest _:l2\n_:l2 rdf:first ex:B\n_:l2 rdf:rest rdf:nil";

#[test]
fn class_expressions_chains_keys_and_restrictions() -> Result<()> {
    let found = derived(&format!(
        "ex:AB owl:intersectionOf _:l1\n{LIST_AB}
ex:x a ex:A\nex:x a ex:B
ex:gp owl:propertyChainAxiom _:c1\n_:c1 rdf:first ex:parent\n_:c1 rdf:rest _:c2\n_:c2 rdf:first ex:parent\n_:c2 rdf:rest rdf:nil
ex:a ex:parent ex:b\nex:b ex:parent ex:c
ex:Policy owl:hasKey _:k\n_:k rdf:first ex:number\n_:k rdf:rest rdf:nil
ex:p1 a ex:Policy\nex:p1 ex:number \"7\"\nex:p2 a ex:Policy\nex:p2 ex:number \"7\"
_:r owl:onProperty ex:owns\n_:r owl:someValuesFrom ex:Car\n_:r rdfs:subClassOf ex:Driver
ex:ann ex:owns ex:mini\nex:mini a ex:Car
ex:Gold owl:hasValue ex:gold\nex:Gold owl:onProperty ex:tier\nex:bob a ex:Gold
ex:knows owl:inverseOf ex:knownBy\nex:ann ex:knows ex:bob
ex:ancestor a owl:TransitiveProperty\nex:a ex:ancestor ex:b\nex:b ex:ancestor ex:c"
    ))?;
    for line in [
        "ex:x a ex:AB",
        "ex:a ex:gp ex:c",
        "ex:p1 owl:sameAs ex:p2",
        "ex:ann a ex:Driver",
        "ex:bob ex:tier ex:gold",
        "ex:bob ex:knownBy ex:ann",
        "ex:a ex:ancestor ex:c",
        "ex:AB rdfs:subClassOf ex:A",
    ] {
        assert!(holds(&found, line), "{line} not derived");
    }
    Ok(())
}

#[test]
fn equality_spreads_facts_and_max_cardinality_merges() -> Result<()> {
    let found = derived(
        "ex:a owl:sameAs ex:b\nex:a ex:p ex:o\nex:s ex:q ex:a
_:r owl:onProperty ex:spouse\n_:r owl:maxCardinality \"1\"^^<http://www.w3.org/2001/XMLSchema#nonNegativeInteger>\nex:x a _:r
ex:x ex:spouse ex:y1\nex:x ex:spouse ex:y2",
    )?;
    for line in
        ["ex:b owl:sameAs ex:a", "ex:b ex:p ex:o", "ex:s ex:q ex:b", "ex:y1 owl:sameAs ex:y2"]
    {
        assert!(holds(&found, line), "{line} not derived");
    }
    Ok(())
}

#[test]
fn a_clash_names_the_rule_the_resources_and_the_triples() {
    let found = clashes("ex:Open owl:disjointWith ex:Closed\nex:c7 a ex:Open\nex:c7 a ex:Closed");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0]["rule"], "cax-dw");
    assert!(found[0]["terms"].as_array().is_some_and(|t| t.contains(&json!("urn:ex:c7"))));
    assert_eq!(found[0]["premises"].as_array().map(Vec::len), Some(3));
    for (data, rule) in [
        ("ex:x owl:sameAs ex:y\nex:x owl:differentFrom ex:y", "eq-diff1"),
        ("ex:x a owl:Nothing", "cls-nothing2"),
        ("ex:p a owl:AsymmetricProperty\nex:a ex:p ex:b\nex:b ex:p ex:a", "prp-asyp"),
        ("ex:p a owl:IrreflexiveProperty\nex:a ex:p ex:a", "prp-irp"),
        ("ex:p owl:propertyDisjointWith ex:q\nex:a ex:p ex:b\nex:a ex:q ex:b", "prp-pdw"),
    ] {
        let found = clashes(data);
        assert_eq!(found.first().map(|c| c["rule"].clone()), Some(json!(rule)), "{data}");
    }
}

#[test]
fn two_values_of_a_functional_data_property_clash_unless_they_are_equal_values() {
    let int = |v: &str| format!("\"{v}\"^^<http://www.w3.org/2001/XMLSchema#integer>");
    let data = |a: &str, b: &str| {
        format!("ex:age a owl:FunctionalProperty\nex:x ex:age {a}\nex:x ex:age {b}")
    };
    assert_eq!(clashes(&data(&int("41"), &int("42")))[0]["rule"], "prp-fp");
    assert!(clashes(&data(&int("42"), &int("042"))).is_empty(), "42 and 042 are the same value");
    assert!(
        clashes(&data("\"Peter\"", "\"Peter\"@en")).is_empty(),
        "different kinds of literal are left alone"
    );
}

#[test]
fn the_output_says_nothing_true_of_everything_or_about_unmentioned_vocabulary() -> Result<()> {
    let found = derived("ex:A rdfs:subClassOf ex:B\nex:x a ex:A\nex:A a owl:Class")?;
    assert!(holds(&found, "ex:x a ex:B"));
    assert!(holds(&found, "ex:A rdfs:subClassOf ex:A"), "reflexive subclass stays");
    assert!(!found.iter().any(|l| l.ends_with(&format!("<{OWL}Thing> ."))), "{found:?}");
    assert!(
        !found.iter().any(|l| l.starts_with(&format!("<{OWL}Nothing>"))),
        "owl:Nothing is not mentioned"
    );
    Ok(())
}

#[test]
fn an_explanation_walks_through_intermediate_deductions() -> Result<()> {
    let data = nt(
        "ex:Dog rdfs:subClassOf ex:Mammal\nex:Mammal rdfs:subClassOf ex:Animal\nex:rex a ex:Dog",
    );
    let target = nt("ex:rex a ex:Animal");
    let found = reasoner()?.explain(NativeOwlRl::PROFILE, &data, &[], &target)?.expect("supported");
    let derivation = &found[0];
    assert_eq!(derivation["rule"], "cax-sco");
    assert_eq!(derivation["incomplete"], false);
    let premises = derivation["premises"].as_array().expect("premises");
    // One premise is asserted; the other (rex a Mammal) was deduced first and is explained in turn.
    let nested = premises.iter().find(|p| p.is_object()).expect("a deduced premise");
    assert_eq!(nested["triple"], nt("ex:rex a ex:Mammal"));
    assert_eq!(nested["rule"], "cax-sco");
    assert!(
        nested["premises"].as_array().is_some_and(|p| p.contains(&json!(nt("ex:rex a ex:Dog"))))
    );
    // An asserted triple has no derivation.
    assert!(
        reasoner()?
            .explain(NativeOwlRl::PROFILE, &data, &[], &nt("ex:rex a ex:Dog"))?
            .expect("supported")
            .is_empty()
    );
    Ok(())
}

#[test]
fn limits_and_configuration_are_enforced() -> Result<()> {
    assert!(NativeFactory.create(&json!({"max_triples": 0})).is_err());
    assert!(NativeFactory.create(&json!({"unknown": 1})).is_err());
    let small = NativeFactory.create(&json!({"max_triples": 3}))?;
    let chain: String =
        (0..20).map(|i| format!("<urn:ex:n{i}> <urn:ex:next> <urn:ex:n{}> .\n", i + 1)).collect();
    let data = format!("<urn:ex:next> <{RDF}type> <{OWL}TransitiveProperty> .\n{chain}");
    let error = small.infer(NativeOwlRl::PROFILE, &data).expect_err("too many derived triples");
    assert!(format!("{error:#}").contains("derived output exceeds"));
    assert!(reasoner()?.infer("owlrl-core-v1", &data).is_err(), "only its own profile");
    Ok(())
}

mod in_a_job {
    use super::*;
    use rdf_engine::Database;
    use rdf_inference_providers::registry;
    use rdf_plugin_inference::{Config, Inference};

    #[test]
    fn a_job_explains_its_deduction_down_to_the_asserted_facts() -> Result<()> {
        let config: Config = serde_json::from_value(json!({
            "providers": {"native": {"kind": "owlrl-native", "config": {}}},
            "jobs": [{"name": "taxonomy", "provider": "native", "profile": "owl2rl-v1", "source_graphs": ["urn:data"]}]
        }))?;
        let mut db = Database::memory()?;
        db.register(Inference::with_registry(config, &registry()?)?)?;
        db.update(&format!("INSERT DATA {{ GRAPH <urn:data> {{ <urn:ex:Dog> <{RDFS}subClassOf> <urn:ex:Mammal> . <urn:ex:Mammal> <{RDFS}subClassOf> <urn:ex:Animal> . <urn:ex:rex> a <urn:ex:Dog> }} }}"))?;
        assert_eq!(
            db.invoke("inference", "rebuild", json!({"job": "taxonomy"}))?["state"],
            "ready"
        );
        let result =
            db.invoke("inference", "explain", json!({"triple": nt("ex:rex a ex:Animal")}))?;
        let explanation = &result["explanations"][0];
        assert_eq!(explanation["supported"], true);
        let premises =
            explanation["derivations"][0]["premises"].as_array().expect("premises").clone();
        let asserted =
            premises.iter().find(|p| p["asserted_in"].is_array()).expect("an asserted premise");
        assert_eq!(asserted["asserted_in"], json!(["urn:data"]));
        let deduced =
            premises.iter().find(|p| p.get("deduced_in_job").is_some()).expect("a deduced premise");
        assert_eq!(deduced["deduced_in_job"]["rule"], "cax-sco");
        let inner = deduced["deduced_in_job"]["premises"].as_array().expect("traced");
        assert!(inner.iter().all(|p| p["asserted_in"] == json!(["urn:data"])), "{inner:?}");
        Ok(())
    }
}
