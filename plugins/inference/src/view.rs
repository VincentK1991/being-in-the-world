//! Explicit materialized union view. No implicit entailment on ordinary queries.
use super::*;
impl Inference {
    pub(crate) fn view_query(&self, context: &dyn ReadContext, input: Value) -> Result<Value> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Input {
            job: String,
            query: String,
            #[serde(default)]
            allow_stale: bool,
        }
        let input: Input = serde_json::from_value(input)?;
        ensure!(input.query.len() <= 65536, "view query exceeds 64 KiB");
        let job = self.job(&input.job).context("unknown inference job")?;
        let guard = context.publication_token()?;
        let mut jobs = self.upstream(&job.name);
        jobs.push(job.name.clone());
        let mut graphs = std::collections::BTreeSet::new();
        let mut generations = Vec::new();
        let mut memo = std::collections::BTreeMap::new();
        for name in jobs {
            let item = self.job(&name).context("unknown inference job")?;
            let status = self.status(context, item, &mut memo, true)?;
            ensure!(
                status["state"] == "ready" || (input.allow_stale && status["state"] == "stale"),
                "inference view unavailable: {name} is {}",
                status["state"]
            );
            graphs.extend(item.source_graphs.iter().cloned());
            graphs.insert(format!("{DERIVED}{name}"));
            generations.push(json!({"job":name,"generation":status["generation"],"state":status["state"],"inputs":status["metadata"]["inputs"]}));
        }
        ensure!(graphs.len() <= 64, "view exceeds 64 graphs");
        let snapshot = context.snapshot(&graphs.into_iter().collect::<Vec<_>>())?;
        ensure!(
            context.publication_token()? == guard,
            "dataset changed while preparing view; retry"
        );
        let results = context.query_snapshot_json(&snapshot, &input.query)?;
        Ok(
            json!({"results":results,"generations":generations,"graphs":snapshot.graphs,"snapshot_token":snapshot.token,
            "view":"set-union-of-local-assertions-and-deductions","remote_input_coherence":"best-effort-observations",
            "remote_assertions_included":false,"allow_stale":input.allow_stale}),
        )
    }
}
