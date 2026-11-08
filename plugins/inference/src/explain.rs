//! `explain`: why a deduced triple holds. The job that holds it is re-run in explanation mode over the
//! data it reads now, which names the rule and the premises; each premise is then traced to the graph it
//! is asserted in, the outside input it came from, or the earlier job that deduced it.
use super::{DERIVED, Inference, Job, fetch_inputs, inputs, rule_tests::blank, upstream_name};
use anyhow::{Context, Result, ensure};
use rdf_plugin_api::ReadContext;
use serde_json::{Value, json};
use std::collections::BTreeSet;

const DEFAULT_DEPTH: u64 = 3;
const MAX_DEPTH: u64 = 5;
/// Premise lookups one call may make, so a wide explanation cannot run away.
const MAX_LOOKUPS: usize = 400;

struct Walk {
    lookups: usize,
    seen: BTreeSet<(String, String)>,
    truncated: bool,
}

/// One canonical N-Triples line.
fn canonical(text: &str) -> Result<String> {
    let mut parsed = oxttl::NTriplesParser::new().for_slice(text.as_bytes());
    let first = parsed.next().context("give one triple as an N-Triples line")?;
    ensure!(parsed.next().is_none(), "explain one triple at a time");
    Ok(format!("{} .", first.context("not a valid N-Triples line")?))
}
/// Graphs holding `triple`, by name ("default" for the default graph).
fn graphs_holding(context: &dyn ReadContext, triple: &str) -> Result<Vec<String>> {
    let pattern = triple.trim_end_matches(" .");
    let result = context.query_json(&format!(
        "SELECT DISTINCT ?g WHERE {{ {{ GRAPH ?g {{ {pattern} }} }} UNION {{ {pattern} BIND(\"default\" AS ?g) }} }}"
    ))?;
    let mut graphs: Vec<String> = result["results"]["bindings"]
        .as_array()
        .map(|rows| {
            rows.iter().filter_map(|r| r["g"]["value"].as_str().map(str::to_owned)).collect()
        })
        .unwrap_or_default();
    graphs.sort();
    Ok(graphs)
}

impl Inference {
    /// `{triple, job?, depth?}`: every derivation of the triple, with premises traced back.
    pub(crate) fn explain(&self, context: &dyn ReadContext, input: &Value) -> Result<Value> {
        let object = input.as_object().context("expected object")?;
        ensure!(
            object.contains_key("triple")
                && object.keys().all(|key| matches!(key.as_str(), "triple" | "job" | "depth")),
            "expected triple, and optionally job and depth"
        );
        let triple = canonical(input["triple"].as_str().context("triple must be a string")?)?;
        ensure!(!blank(&triple), "a triple with a blank node cannot be looked up; use IRIs");
        let depth = match object.get("depth") {
            None => DEFAULT_DEPTH,
            Some(value) => value
                .as_u64()
                .filter(|d| (1..=MAX_DEPTH).contains(d))
                .context("depth must be 1 to 5")?,
        };
        let named = object
            .get("job")
            .map(|job| job.as_str().context("job must be a string"))
            .transpose()?;
        let holders: Vec<&Job> = graphs_holding(context, &triple)?
            .iter()
            .filter_map(|graph| upstream_name(graph))
            .filter_map(|name| self.job(name))
            .filter(|job| named.is_none_or(|wanted| job.name == wanted))
            .collect();
        if holders.is_empty() {
            ensure!(named.is_none(), "the triple is not in that job's derived graph");
            return Ok(json!({
                "triple": triple, "derived": false,
                "asserted_in": graphs_holding(context, &triple)?,
                "explanations": [],
            }));
        }
        let mut walk = Walk { lookups: 0, seen: BTreeSet::new(), truncated: false };
        let explanations = holders
            .into_iter()
            .map(|job| self.explain_in(context, job, &triple, depth as usize, &mut walk))
            .collect::<Result<Vec<_>>>()?;
        Ok(
            json!({"triple": triple, "derived": true, "explanations": explanations, "truncated": walk.truncated}),
        )
    }

    fn explain_in(
        &self,
        context: &dyn ReadContext,
        job: &Job,
        triple: &str,
        depth: usize,
        walk: &mut Walk,
    ) -> Result<Value> {
        let graph = format!("{DERIVED}{}", job.name);
        if !walk.seen.insert((job.name.clone(), triple.to_owned())) {
            return Ok(json!({"job": job.name, "graph": graph, "repeated": true}));
        }
        let (reasoner, _) = self.parts(job)?;
        let state = context.derived_status(&job.name)?["state"].clone();
        let source = context.snapshot(&job.source_graphs)?;
        let fetched = fetch_inputs(job)?;
        let merged = inputs::merge(&source.ntriples, &fetched)?;
        let Some(derivations) = reasoner.explain(&job.profile, &merged, &job.rules, triple)? else {
            return Ok(json!({
                "job": job.name, "graph": graph, "profile": job.profile, "state": state, "supported": false,
                "note": "this reasoner does not record why it deduced a triple; the job and the graphs it read are known",
                "source_graphs": job.source_graphs,
            }));
        };
        let mut described = Vec::new();
        for derivation in derivations {
            let mut entry = derivation.clone();
            entry["premises"] =
                self.trace_all(context, job, &fetched, &derivation["premises"], depth, walk)?;
            described.push(entry);
        }
        Ok(json!({
            "job": job.name, "graph": graph, "profile": job.profile, "state": state, "supported": true,
            "stale": state != "ready", "derivations": described,
        }))
    }

    /// Premises are N-Triples lines, or, from a provider that runs to a fixpoint, objects for
    /// premises the same job deduced first (`triple`, `rule`, nested `premises`). Lines are traced
    /// to the graphs and inputs they come from; objects are kept, with their own premises traced.
    fn trace_all(
        &self,
        context: &dyn ReadContext,
        job: &Job,
        fetched: &[super::Fetched],
        premises: &Value,
        depth: usize,
        walk: &mut Walk,
    ) -> Result<Value> {
        let mut traced = Vec::new();
        for premise in premises.as_array().into_iter().flatten() {
            if let Some(line) = premise.as_str() {
                traced.push(self.trace(context, job, fetched, line, depth, walk)?);
            } else {
                let mut entry = json!({"triple": premise["triple"], "deduced_in_job": {"rule": premise["rule"]}});
                if premise["truncated"] == true {
                    entry["deduced_in_job"]["truncated"] = json!(true);
                } else {
                    entry["deduced_in_job"]["premises"] =
                        self.trace_all(context, job, fetched, &premise["premises"], depth, walk)?;
                }
                traced.push(entry);
            }
        }
        Ok(Value::Array(traced))
    }

    /// Where one premise comes from.
    fn trace(
        &self,
        context: &dyn ReadContext,
        job: &Job,
        fetched: &[super::Fetched],
        premise: &str,
        depth: usize,
        walk: &mut Walk,
    ) -> Result<Value> {
        if blank(premise) {
            return Ok(json!({"triple": premise, "origin": "blank node; not traced"}));
        }
        if walk.lookups >= MAX_LOOKUPS {
            walk.truncated = true;
            return Ok(json!({"triple": premise, "origin": "not traced; lookup limit reached"}));
        }
        walk.lookups += 1;
        let holding = graphs_holding(context, premise)?;
        let mut asserted = Vec::new();
        let mut deductions = Vec::new();
        for graph in holding.iter().filter(|g| job.source_graphs.contains(g)) {
            match upstream_name(graph).and_then(|name| self.job(name)) {
                Some(earlier) if depth > 1 => {
                    deductions.push(self.explain_in(context, earlier, premise, depth - 1, walk)?);
                }
                Some(earlier) => deductions
                    .push(json!({"job": earlier.name, "graph": graph, "explained": false})),
                None => asserted.push(graph.clone()),
            }
        }
        // Outside inputs are not stored. Blank nodes in them are renamed, so only blank-free lines match.
        let from_inputs: Vec<&str> = fetched
            .iter()
            .filter(|input| input.ntriples.lines().any(|line| line == premise))
            .map(|input| input.name.as_str())
            .collect();
        let mut entry = json!({"triple": premise});
        if !asserted.is_empty() {
            entry["asserted_in"] = json!(asserted);
        }
        if !from_inputs.is_empty() {
            entry["outside_input"] = json!(from_inputs);
        }
        if !deductions.is_empty() {
            entry["deduced_by"] = Value::Array(deductions);
        }
        if asserted.is_empty() && from_inputs.is_empty() && entry.get("deduced_by").is_none() {
            entry["origin"] =
                json!("not found in the job's sources now; the data may have changed");
        }
        Ok(entry)
    }
}
