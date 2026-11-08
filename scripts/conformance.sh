#!/usr/bin/env bash
set -euo pipefail
repo_root=$(cd "$(dirname "$0")/.." && pwd)
suite_dir="$repo_root/target/w3c-rdf-tests"
revision=d01b141c65ddf8e5b615256f264111118bdb2134
# OWL 2 conformance test cases in the RL profile (W3C export, 2009-11); premises are embedded RDF/XML.
owl2_sha256=6415369555af022fedd0fe9a7d6b56eb274f89284e4109d7303540ec75c76988
owl2="$repo_root/target/owl2-tests/profile-RL.rdf"
if [[ ! -d "$suite_dir/.git" ]]; then
  git clone https://github.com/w3c/rdf-tests.git "$suite_dir"
fi
git -C "$suite_dir" fetch origin "$revision"
git -C "$suite_dir" checkout --detach "$revision"
if [[ ! -f "$owl2" ]]; then
  mkdir -p "$(dirname "$owl2")"
  curl -fsSL https://www.w3.org/2009/11/owl-test/profile-RL.rdf -o "$owl2"
fi
printf '%s  %s\n' "$owl2_sha256" "$owl2" | sha256sum -c -
cd "$repo_root"
cargo build --locked -p rdf-cli
for suite in rdf-n-triples rdf-n-quads rdf-turtle rdf-trig; do
  target/debug/rdftool test-suite --manifest "$suite_dir/rdf/rdf11/$suite/manifest.ttl" > "target/conformance-$suite.json"
done
for suite in syntax-query syntax-update-1 syntax-update-2; do
  target/debug/rdftool test-suite --manifest "$suite_dir/sparql/sparql11/$suite/manifest.ttl" > "target/conformance-$suite.json"
done
# RDF syntaxes (with RDF/XML) and SPARQL 1.0 + 1.1 evaluation, held to committed baselines;
# SPARQL runs both through Oxigraph and through rdf-engine.
python3 scripts/conformance/baseline.py rdf
python3 scripts/conformance/baseline.py sparql --engine oxigraph
python3 scripts/conformance/baseline.py sparql --engine database
# Entailment (RDF 1.1 Semantics, SPARQL entailment regimes, OWL 2 RL) needs the reasoner sidecars.
if [[ "${CONFORMANCE_REASONERS:-0}" == 1 ]]; then
  python3 scripts/conformance/baseline.py entailment --reasoner owlrl
  python3 scripts/conformance/baseline.py entailment --reasoner jena-rdfs
fi
