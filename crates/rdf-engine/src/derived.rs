//! Generic derived-graph lifecycle; no reasoner-specific code.
use super::*;
use anyhow::Context;
use rdf_plugin_api::GraphSnapshot;
use rdf_store::oxigraph::model::{Literal, Quad, Term};
use serde_json::json;
use sha2::{Digest, Sha256};

const PREFIX: &str = "urn:rdftool:derived:";
const META: &str = "urn:rdftool:metadata";
const ATTEMPT: &str = "urn:rdftool:attempt";
const MAX_BYTES: usize = 16 * 1024 * 1024;
fn hash(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}
fn lines(mut values: Vec<String>) -> String {
    values.sort();
    values.dedup();
    values.join("\n")
}
pub(super) struct PluginContext<'a> {
    pub database: &'a Database,
    pub owner: &'a str,
}
impl PluginContext<'_> {
    fn publication_token_unlocked(&self) -> Result<String> {
        let mut entries = self
            .database
            .dataset
            .store()
            .iter()
            .map(|q| q.map(|q| q.to_string()))
            .collect::<Result<Vec<_>, _>>()?;
        entries.extend(
            self.database
                .dataset
                .store()
                .named_graphs()
                .map(|g| g.map(|g| format!("graph {g}")))
                .collect::<Result<Vec<_>, _>>()?,
        );
        Ok(hash(&format!("{}\n{}", self.database.dataset.identity(), lines(entries))))
    }

    fn target(&self, name: &str) -> Result<NamedNode> {
        ensure!(
            !name.is_empty()
                && name.len() <= 64
                && name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_'),
            "invalid derived graph name"
        );
        Ok(NamedNode::new(format!("{PREFIX}{}:{name}", self.owner))?)
    }
    fn snapshot_unlocked(&self, graphs: &[String]) -> Result<GraphSnapshot> {
        ensure!(!graphs.is_empty() && graphs.len() <= 64, "select 1 to 64 source graphs");
        for graph in graphs {
            // A derived graph may feed another derived graph (chained jobs); its bookkeeping graph may not.
            ensure!(
                !(graph.starts_with(PREFIX) && graph.ends_with(":metadata")),
                "derived-graph metadata cannot be a source"
            );
            if graph != "default" {
                NamedNode::new(graph.clone())?;
            }
        }
        let mut selected = Vec::new();
        let mut triples = Vec::new();
        let mut bytes = 0;
        // Oxigraph iterator owns a consistent snapshot across all selected graphs.
        for quad in self.database.dataset.store().iter() {
            let quad = quad?;
            let name = match &quad.graph_name {
                GraphName::DefaultGraph => "default",
                GraphName::NamedNode(n) => n.as_str(),
                _ => continue,
            };
            if graphs.iter().any(|g| g == name) {
                let line = quad.to_string();
                bytes += line.len();
                ensure!(bytes <= MAX_BYTES, "source snapshot exceeds 16 MiB");
                selected.push(line);
                triples.push(format!("{} {} {} .", quad.subject, quad.predicate, quad.object));
            }
        }
        let mut names = graphs.to_vec();
        names.sort();
        names.dedup();
        let token = hash(&format!(
            "{}\n{}\n{}",
            self.database.dataset.identity(),
            serde_json::to_string(&names)?,
            lines(selected)
        ));
        Ok(GraphSnapshot { graphs: names, token, ntriples: lines(triples) })
    }
}
impl ReadContext for PluginContext<'_> {
    fn query_snapshot_json(&self, snapshot: &GraphSnapshot, query: &str) -> Result<Value> {
        use rdf_store::oxigraph::sparql::{CancellationToken, SparqlEvaluator};
        let data = Dataset::memory()?;
        data.store().extend(
            RdfParser::from_format(RdfFormat::NTriples)
                .for_reader(snapshot.ntriples.as_bytes())
                .collect::<Result<Vec<_>, _>>()?,
        )?;
        let query = Dataset::prepare_query(query)?;
        ensure!(
            matches!(
                &query,
                spargebra::Query::Select { dataset: None, .. }
                    | spargebra::Query::Ask { dataset: None, .. }
            ),
            "view accepts SELECT/ASK without FROM; the selected union is the default graph"
        );
        let pattern = match &query {
            spargebra::Query::Select { pattern, .. } | spargebra::Query::Ask { pattern, .. } => {
                pattern
            }
            _ => unreachable!("validated query"),
        };
        rdf_store::work_budget::check(
            data.store(),
            pattern,
            &std::collections::HashMap::new(),
            1_000_000,
        )?;
        let cancel = CancellationToken::new();
        let timer = cancel.clone();
        let (_done, receiver) = std::sync::mpsc::channel::<()>();
        std::thread::spawn(move || {
            if receiver
                .recv_timeout(std::time::Duration::from_secs(30))
                .is_err_and(|e| e == std::sync::mpsc::RecvTimeoutError::Timeout)
            {
                timer.cancel();
            }
        });
        let results = SparqlEvaluator::new()
            .with_cancellation_token(cancel)
            .for_query(query)
            .on_store(data.store())
            .execute()?;
        struct Limited(Vec<u8>);
        impl std::io::Write for Limited {
            fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
                if self.0.len() + data.len() > 16 * 1024 * 1024 {
                    return Err(std::io::Error::other("view results exceed 16 MiB"));
                }
                self.0.extend_from_slice(data);
                Ok(data.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let mut bytes = Limited(Vec::new());
        write_results(results, QueryResultsFormat::Json, RdfFormat::Turtle, &mut bytes)?;
        Ok(serde_json::from_slice(&bytes.0)?)
    }

    fn publication_token(&self) -> Result<String> {
        let _guard = self.database.dataset.lock_mutation()?;
        self.publication_token_unlocked()
    }

    fn query_json(&self, query: &str) -> Result<Value> {
        self.database.query_json(query)
    }
    fn snapshot(&self, graphs: &[String]) -> Result<GraphSnapshot> {
        let _guard = self.database.dataset.lock_mutation()?;
        self.snapshot_unlocked(graphs)
    }
    fn publish(
        &self,
        name: &str,
        source: &GraphSnapshot,
        ntriples: &str,
        metadata: Value,
    ) -> Result<Value> {
        self.publish_checked(name, source, ntriples, metadata, None)
    }
    fn publish_checked(
        &self,
        name: &str,
        source: &GraphSnapshot,
        ntriples: &str,
        metadata: Value,
        token: Option<&str>,
    ) -> Result<Value> {
        ensure!(
            ntriples.len() <= MAX_BYTES && serde_json::to_vec(&metadata)?.len() <= 65536,
            "derived output or metadata exceeds limit"
        );
        let target = self.target(name)?;
        let meta_graph = NamedNode::new(format!("{}:metadata", target.as_str()))?;
        // Preserve source blank-node identifiers: no rename_blank_nodes here.
        let mut quads = RdfParser::from_format(RdfFormat::NTriples)
            .with_default_graph(target.clone())
            .for_reader(ntriples.as_bytes())
            .collect::<Result<Vec<_>, _>>()?;
        quads.sort_by_key(ToString::to_string);
        quads.dedup();
        let output_digest = hash(&lines(quads.iter().map(|q| q.to_string()).collect()));
        let _guard = self.database.dataset.lock_mutation()?;
        if let Some(token) = token {
            ensure!(
                self.publication_token_unlocked()? == token,
                "dataset changed during dependency evaluation; retry rebuild"
            );
        }

        ensure!(
            self.snapshot_unlocked(&source.graphs)?.token == source.token,
            "source changed during computation; rebuild required"
        );
        let generation = uuid::Uuid::new_v4().to_string();
        let record = json!({"generation":generation,"source_token":source.token,"graphs":source.graphs,"output_digest":output_digest,"metadata":metadata,"triples":quads.len(),"graph":target.as_str()});
        let mut tx = self.database.dataset.store().start_transaction()?;
        tx.clear_graph(&target)?;
        tx.insert_named_graph(&target);
        tx.clear_graph(&meta_graph)?;
        tx.insert_named_graph(&meta_graph);
        for quad in &quads {
            tx.insert(quad);
        }
        tx.insert(&Quad::new(
            target.clone(),
            NamedNode::new(META)?,
            Literal::new_simple_literal(serde_json::to_string(&record)?),
            meta_graph,
        ));
        tx.commit()?;
        Ok(record)
    }
    fn record_attempt(&self, name: &str, attempt: Option<Value>) -> Result<()> {
        let target = self.target(name)?;
        let meta_graph = NamedNode::new(format!("{}:metadata", target.as_str()))?;
        let predicate = NamedNode::new(ATTEMPT)?;
        let text = attempt.as_ref().map(serde_json::to_string).transpose()?;
        ensure!(text.as_ref().is_none_or(|t| t.len() <= 65536), "attempt record exceeds 64 KiB");
        let _guard = self.database.dataset.lock_mutation()?;
        let store = self.database.dataset.store();
        let old = store
            .quads_for_pattern(
                Some(target.as_ref().into()),
                Some(predicate.as_ref()),
                None,
                Some(meta_graph.as_ref().into()),
            )
            .collect::<Result<Vec<_>, _>>()?;
        let mut tx = store.start_transaction()?;
        for quad in &old {
            tx.remove(quad);
        }
        if let Some(text) = text {
            tx.insert_named_graph(&meta_graph);
            tx.insert(&Quad::new(
                target.clone(),
                predicate,
                Literal::new_simple_literal(text),
                meta_graph,
            ));
        }
        tx.commit()?;
        Ok(())
    }
    fn derived_status(&self, name: &str) -> Result<Value> {
        let target = self.target(name)?;
        let meta_graph = NamedNode::new(format!("{}:metadata", target.as_str()))?;
        let _guard = self.database.dataset.lock_mutation()?;
        let attempt = match self
            .database
            .dataset
            .store()
            .quads_for_pattern(
                Some(target.as_ref().into()),
                Some(NamedNode::new(ATTEMPT)?.as_ref()),
                None,
                Some(meta_graph.as_ref().into()),
            )
            .next()
        {
            Some(quad) => match quad?.object {
                Term::Literal(literal) => Some(serde_json::from_str::<Value>(literal.value())?),
                _ => anyhow::bail!("invalid attempt record"),
            },
            None => None,
        };
        let mut records = self.database.dataset.store().quads_for_pattern(
            Some(target.as_ref().into()),
            Some(NamedNode::new(META)?.as_ref()),
            None,
            Some(meta_graph.as_ref().into()),
        );
        let Some(record) = records.next() else {
            let mut missing = json!({"state":"missing","graph":target.as_str()});
            if let Some(attempt) = attempt {
                missing["attempt"] = attempt;
            }
            return Ok(missing);
        };
        let Term::Literal(literal) = record?.object else {
            anyhow::bail!("invalid derived metadata");
        };
        let mut record: Value = serde_json::from_str(literal.value())?;
        let graphs: Vec<String> = serde_json::from_value(record["graphs"].clone())?;
        let source = self.snapshot_unlocked(&graphs)?;
        let quads = self
            .database
            .dataset
            .store()
            .quads_for_pattern(None, None, None, Some(target.as_ref().into()))
            .map(|q| q.map(|q| q.to_string()))
            .collect::<Result<Vec<_>, _>>()?;
        let current = self.database.dataset.store().contains_named_graph(&target)?
            && record["source_token"].as_str().context("source token missing")? == source.token
            && record["output_digest"] == hash(&lines(quads));
        record["state"] = json!(if current { "ready" } else { "stale" });
        if let Some(attempt) = attempt {
            record["attempt"] = attempt;
        }
        Ok(record)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn snapshot_publication_is_atomic_scoped_and_detects_source_changes() -> Result<()> {
        let db = Database::memory()?;
        db.update("INSERT DATA { GRAPH <urn:source> { _:person <urn:name> 'Alice' . } GRAPH <urn:other> { <urn:private> <urn:p> 1 } }")?;
        let context = PluginContext { database: &db, owner: "test" };
        let source = context.snapshot(&["urn:source".into()])?;
        assert!(!source.ntriples.contains("private"));
        let published = context.publish("job", &source, &source.ntriples, json!({"version":1}))?;
        assert_eq!(published["triples"], 1);
        assert_eq!(db.query_json("ASK { GRAPH <urn:source> { ?s <urn:name> 'Alice' } GRAPH <urn:rdftool:derived:test:job> { ?s <urn:name> 'Alice' } }")?["boolean"],true);
        assert_eq!(context.derived_status("job")?["state"], "ready");
        assert!(
            context
                .publish("job", &source, "<urn:s> <urn:p> <urn:o> . invalid", Value::Null)
                .is_err()
        );
        assert_eq!(context.derived_status("job")?["state"], "ready");
        db.update("INSERT DATA { GRAPH <urn:source> { <urn:new> <urn:p> 2 } }")?;
        assert!(context.publish("job", &source, "", Value::Null).is_err());
        assert_eq!(context.derived_status("job")?["state"], "stale");
        assert_eq!(
            db.query_json("ASK { GRAPH <urn:rdftool:derived:test:job> { ?s ?p ?o } }")?["boolean"],
            true
        );
        let other = Database::memory()?;
        let other_context = PluginContext { database: &other, owner: "test" };
        assert!(other_context.publish("job", &source, "", Value::Null).is_err());
        assert!(context.publish("../escape", &source, "", Value::Null).is_err());
        // A derived graph is a valid source, so jobs can chain; its metadata graph is not.
        let chained = context.snapshot(&["urn:rdftool:derived:test:job".into()])?;
        assert_eq!(chained.graphs, ["urn:rdftool:derived:test:job"]);
        assert!(context.snapshot(&["urn:rdftool:derived:test:job:metadata".into()]).is_err());
        let current = context.snapshot(&["urn:source".into()])?;
        context.publish("job", &current, "", json!({"version":2}))?;
        assert_eq!(context.derived_status("job")?["state"], "ready");
        db.update(
            "INSERT DATA { GRAPH <urn:rdftool:derived:test:job> { <urn:tamper> <urn:p> 1 } }",
        )?;
        assert_eq!(context.derived_status("job")?["state"], "stale");
        Ok(())
    }
}
