//! "Why was this deduced?" for SPARQL rules.
use anyhow::Result;
use rdf_inference_providers::SparqlFactory;
use rdf_plugin_inference::{Reasoner, ReasonerFactory};
use serde_json::{Value, json};

const PREFIX: &str = "PREFIX ex: <urn:ex:>\n";
const DATA: &str = r#"<urn:ex:c1> <urn:ex:onPolicy> <urn:ex:p1> .
<urn:ex:c1> <urn:ex:severity> "high" .
<urn:ex:c2> <urn:ex:onPolicy> <urn:ex:p1> .
<urn:ex:c2> <urn:ex:severity> "high" .
<urn:ex:c3> <urn:ex:onPolicy> <urn:ex:p1> .
<urn:ex:c3> <urn:ex:severity> "low" .
<urn:ex:c4> <urn:ex:onPolicy> <urn:ex:p2> .
<urn:ex:c4> <urn:ex:severity> "low" .
<urn:ex:p1> <urn:ex:number> "111" ."#;

fn reasoner() -> Result<std::sync::Arc<dyn Reasoner>> {
    SparqlFactory.create(&json!({}))
}
fn rules() -> Vec<String> {
    vec![
        format!(
            "{PREFIX}CONSTRUCT {{ ?p ex:atRisk true }} WHERE {{ ?c ex:onPolicy ?p ; ex:severity \"high\" }}"
        ),
        format!(
            "{PREFIX}CONSTRUCT {{ ?p ex:highClaims ?n }} WHERE {{ SELECT ?p (COUNT(?c) AS ?n) WHERE {{ ?c ex:onPolicy ?p ; ex:severity \"high\" }} GROUP BY ?p }}"
        ),
        format!(
            "{PREFIX}CONSTRUCT {{ ?p ex:label ?l }} WHERE {{ ?p ex:number ?n . BIND(CONCAT(\"Policy \", ?n) AS ?l) }}"
        ),
    ]
}
fn explain(target: &str) -> Result<Vec<Value>> {
    Ok(reasoner()?.explain("sparql-construct-once-v1", DATA, &rules(), target)?.expect("supported"))
}
fn premises(derivation: &Value) -> Vec<&str> {
    derivation["premises"].as_array().expect("premises").iter().filter_map(Value::as_str).collect()
}

#[test]
fn a_derived_triple_is_explained_by_the_rule_and_the_facts_it_used() -> Result<()> {
    let found = explain(
        "<urn:ex:p1> <urn:ex:atRisk> \"true\"^^<http://www.w3.org/2001/XMLSchema#boolean> .",
    )?;
    // Two high-severity claims on p1, so two different ways to derive the same triple.
    assert_eq!(found.len(), 2);
    assert!(found.iter().all(|d| d["rule"] == 0 && d["aggregate"] == false));
    let claims: Vec<String> =
        found.iter().map(|d| d["bindings"]["c"].as_str().expect("claim").to_owned()).collect();
    assert_eq!(
        claims.iter().cloned().collect::<std::collections::BTreeSet<_>>(),
        ["<urn:ex:c1>", "<urn:ex:c2>"].map(String::from).into()
    );
    for derivation in &found {
        let claim = derivation["bindings"]["c"].as_str().expect("claim");
        assert!(
            premises(derivation)
                .contains(&format!("{claim} <urn:ex:severity> \"high\" .").as_str())
        );
        assert!(
            premises(derivation)
                .contains(&format!("{claim} <urn:ex:onPolicy> <urn:ex:p1> .").as_str())
        );
        assert_eq!(premises(derivation).len(), 2, "only the facts the rule matched");
    }
    Ok(())
}

#[test]
fn an_aggregate_lists_every_member_it_counted_and_says_so() -> Result<()> {
    let found = explain(
        "<urn:ex:p1> <urn:ex:highClaims> \"2\"^^<http://www.w3.org/2001/XMLSchema#integer> .",
    )?;
    assert_eq!(found.len(), 1);
    assert_eq!(found[0]["rule"], 1);
    assert_eq!(found[0]["aggregate"], true);
    let used = premises(&found[0]);
    for claim in ["c1", "c2"] {
        assert!(
            used.contains(&format!("<urn:ex:{claim}> <urn:ex:severity> \"high\" .").as_str()),
            "{used:?}"
        );
    }
    assert!(
        !used.iter().any(|line| line.contains("<urn:ex:c3>")),
        "the low-severity claim was not counted"
    );
    Ok(())
}

#[test]
fn a_computed_value_is_matched_and_its_inputs_are_listed() -> Result<()> {
    let found = explain("<urn:ex:p1> <urn:ex:label> \"Policy 111\" .")?;
    assert_eq!(found.len(), 1);
    assert_eq!(found[0]["bindings"]["n"], "\"111\"");
    assert_eq!(premises(&found[0]), ["<urn:ex:p1> <urn:ex:number> \"111\" ."]);
    Ok(())
}

#[test]
fn a_triple_no_rule_produces_has_no_explanation() -> Result<()> {
    assert!(
        explain(
            "<urn:ex:p2> <urn:ex:atRisk> \"true\"^^<http://www.w3.org/2001/XMLSchema#boolean> ."
        )?
        .is_empty()
    );
    assert!(
        explain(
            "<urn:ex:p1> <urn:ex:highClaims> \"7\"^^<http://www.w3.org/2001/XMLSchema#integer> ."
        )?
        .is_empty()
    );
    Ok(())
}

mod across_jobs {
    use super::*;
    use rdf_engine::Database;
    use rdf_inference_providers::registry;
    use rdf_plugin_inference::{Config, Inference};

    const FLAGGED: &str =
        "<urn:ex:c1> <urn:ex:flagged> \"true\"^^<http://www.w3.org/2001/XMLSchema#boolean> .";
    const AT_RISK: &str =
        "<urn:ex:p1> <urn:ex:atRisk> \"true\"^^<http://www.w3.org/2001/XMLSchema#boolean> .";

    /// flags: severity high -> flagged. risk: a flagged claim on a policy -> the policy is at risk, reading flags' output.
    fn database() -> Result<Database> {
        let config: Config = serde_json::from_value(json!({
            "providers": {"local": {"kind": "sparql-rules", "config": {}}},
            "jobs": [
                {"name": "flags", "provider": "local", "profile": "sparql-construct-once-v1", "source_graphs": ["urn:data"],
                 "rules": [format!("{PREFIX}CONSTRUCT {{ ?c ex:flagged true }} WHERE {{ ?c ex:severity \"high\" }}")]},
                {"name": "risk", "provider": "local", "profile": "sparql-construct-once-v1",
                 "source_graphs": ["urn:data", "urn:rdftool:derived:inference:flags"],
                 "rules": [format!("{PREFIX}CONSTRUCT {{ ?p ex:atRisk true }} WHERE {{ ?c ex:onPolicy ?p ; ex:flagged true }}")]},
            ]
        }))?;
        let mut db = Database::memory()?;
        db.register(Inference::with_registry(config, &registry()?)?)?;
        db.update("INSERT DATA { GRAPH <urn:data> { <urn:ex:c1> <urn:ex:severity> \"high\" . <urn:ex:c1> <urn:ex:onPolicy> <urn:ex:p1> . <urn:ex:c2> <urn:ex:severity> \"low\" . <urn:ex:c2> <urn:ex:onPolicy> <urn:ex:p1> } }")?;
        db.invoke("inference", "rebuild", json!({"job": "risk", "cascade": "upstream"}))?;
        Ok(db)
    }
    fn explain(db: &Database, input: Value) -> Result<Value> {
        db.invoke("inference", "explain", input)
    }

    #[test]
    fn a_conclusion_is_traced_through_the_earlier_job_down_to_the_asserted_fact() -> Result<()> {
        let db = database()?;
        let result = explain(&db, json!({"triple": AT_RISK}))?;
        assert_eq!(result["derived"], true);
        let risk = &result["explanations"][0];
        assert_eq!(risk["job"], "risk");
        assert_eq!(risk["stale"], false);
        let derivation = &risk["derivations"][0];
        assert_eq!(derivation["bindings"]["c"], "<urn:ex:c1>");
        let premises = derivation["premises"].as_array().expect("premises");
        // Asserted data.
        let on_policy = premises
            .iter()
            .find(|p| p["triple"].as_str().is_some_and(|t| t.contains("onPolicy")))
            .expect("onPolicy");
        assert_eq!(on_policy["asserted_in"], json!(["urn:data"]));
        // A deduction of the earlier job, itself explained down to an asserted fact.
        let flagged = premises.iter().find(|p| p["triple"] == FLAGGED).expect("flagged");
        assert!(flagged.get("asserted_in").is_none());
        let earlier = &flagged["deduced_by"][0];
        assert_eq!(earlier["job"], "flags");
        let severity = &earlier["derivations"][0]["premises"][0];
        assert_eq!(severity["triple"], "<urn:ex:c1> <urn:ex:severity> \"high\" .");
        assert_eq!(severity["asserted_in"], json!(["urn:data"]));
        Ok(())
    }

    #[test]
    fn depth_limits_how_far_back_the_trail_goes() -> Result<()> {
        let db = database()?;
        let result = explain(&db, json!({"triple": AT_RISK, "depth": 1}))?;
        let premises = result["explanations"][0]["derivations"][0]["premises"]
            .as_array()
            .expect("premises")
            .clone();
        let flagged = premises.iter().find(|p| p["triple"] == FLAGGED).expect("flagged");
        assert_eq!(flagged["deduced_by"][0]["explained"], false);
        Ok(())
    }

    #[test]
    fn an_asserted_triple_is_reported_as_not_derived_and_bad_requests_are_refused() -> Result<()> {
        let db = database()?;
        let asserted = explain(&db, json!({"triple": "<urn:ex:c1> <urn:ex:severity> \"high\" ."}))?;
        assert_eq!(asserted["derived"], false);
        assert_eq!(asserted["asserted_in"], json!(["urn:data"]));
        assert!(
            explain(&db, json!({"triple": AT_RISK, "job": "flags"})).is_err(),
            "flags did not derive it"
        );
        assert!(explain(&db, json!({"triple": "_:b <urn:p> <urn:o> ."})).is_err());
        assert!(explain(&db, json!({"triple": AT_RISK, "depth": 9})).is_err());
        assert!(explain(&db, json!({})).is_err());
        Ok(())
    }

    #[test]
    fn a_job_that_is_out_of_date_says_so() -> Result<()> {
        let db = database()?;
        db.update("INSERT DATA { GRAPH <urn:data> { <urn:ex:c3> <urn:ex:severity> \"high\" } }")?;
        let result = explain(&db, json!({"triple": AT_RISK}))?;
        assert_eq!(result["explanations"][0]["stale"], true);
        Ok(())
    }
}
