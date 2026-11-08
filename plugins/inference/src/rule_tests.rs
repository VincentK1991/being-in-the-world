//! Tests for rules: "given this data, the job must produce exactly this". They run the job's own
//! reasoner and rules over the supplied data. Nothing is read from or written to the store.
use crate::Reasoner;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeSet;

const MAX_TESTS: usize = 50;
const MAX_TEXT: usize = 64 * 1024;
const MAX_LISTED: usize = 20;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuleTest {
    pub name: String,
    /// Turtle shared by `given`, `expect` and `absent`, normally `@prefix` lines.
    #[serde(default)]
    pub prefixes: String,
    /// The data the rules run over.
    pub given: String,
    /// Triples the job must produce.
    #[serde(default)]
    pub expect: String,
    /// Triples the job must not produce.
    #[serde(default)]
    pub absent: String,
    /// Also fail if the job produces anything not listed in `expect`.
    #[serde(default)]
    pub exact: bool,
}

/// Parses Turtle into sorted, de-duplicated N-Triples lines.
fn triples(prefixes: &str, body: &str, what: &str) -> Result<BTreeSet<String>> {
    let text = format!("{prefixes}\n{body}");
    let mut found = BTreeSet::new();
    for triple in oxttl::TurtleParser::new().for_slice(text.as_bytes()) {
        let triple = triple.with_context(|| format!("{what} is not valid Turtle"))?;
        found.insert(format!("{triple} ."));
    }
    Ok(found)
}
fn blank(line: &str) -> bool {
    let body = line.trim_end_matches(" .");
    body.starts_with("_:") || body.rsplit(' ').next().is_some_and(|object| object.starts_with("_:"))
}
/// Reasoner output as the same canonical lines, so the comparison ignores formatting.
fn produced(ntriples: &str) -> Result<BTreeSet<String>> {
    let mut found = BTreeSet::new();
    for triple in oxttl::NTriplesParser::new().for_slice(ntriples.as_bytes()) {
        found.insert(format!("{} .", triple.context("reasoner returned invalid N-Triples")?));
    }
    Ok(found)
}
fn listed(lines: impl IntoIterator<Item = String>) -> Vec<String> {
    lines.into_iter().take(MAX_LISTED).collect()
}

pub fn validate(tests: &[RuleTest]) -> Result<()> {
    ensure!(tests.len() <= MAX_TESTS, "a job accepts at most {MAX_TESTS} tests");
    let mut names = BTreeSet::new();
    for test in tests {
        ensure!(
            !test.name.is_empty() && test.name.len() <= 200 && names.insert(&test.name),
            "test names must be unique, 1 to 200 characters"
        );
        ensure!(
            [&test.prefixes, &test.given, &test.expect, &test.absent]
                .iter()
                .all(|text| text.len() <= MAX_TEXT),
            "test {} exceeds 64 KiB",
            test.name
        );
        ensure!(!test.given.trim().is_empty(), "test {} has no given data", test.name);
        ensure!(
            !(test.expect.trim().is_empty() && test.absent.trim().is_empty() && !test.exact),
            "test {} checks nothing: add expect, absent or exact",
            test.name
        );
        triples(&test.prefixes, &test.given, &format!("given of test {}", test.name))?;
        for (what, body) in [("expect", &test.expect), ("absent", &test.absent)] {
            let lines = triples(&test.prefixes, body, &format!("{what} of test {}", test.name))?;
            ensure!(
                !lines.iter().any(|line| blank(line)),
                "{what} of test {} uses a blank node; name the resource with an IRI",
                test.name
            );
        }
    }
    Ok(())
}

impl RuleTest {
    pub fn run(&self, reasoner: &dyn Reasoner, profile: &str, rules: &[String]) -> Result<Value> {
        let given = triples(&self.prefixes, &self.given, "given")?;
        let input = given.iter().cloned().collect::<Vec<_>>().join("\n");
        let (output, _) = reasoner.infer_rules(profile, &input, rules)?;
        let output = produced(&output)?;
        let expect = triples(&self.prefixes, &self.expect, "expect")?;
        let absent = triples(&self.prefixes, &self.absent, "absent")?;
        let missing: Vec<String> = expect.difference(&output).cloned().collect();
        let forbidden: Vec<String> = absent.intersection(&output).cloned().collect();
        let extra: Vec<String> =
            if self.exact { output.difference(&expect).cloned().collect() } else { Vec::new() };
        let passed = missing.is_empty() && forbidden.is_empty() && extra.is_empty();
        Ok(json!({
            "name": self.name, "passed": passed, "produced": output.len(),
            "missing": listed(missing), "present_but_forbidden": listed(forbidden),
            "unexpected": listed(extra),
        }))
    }
}
