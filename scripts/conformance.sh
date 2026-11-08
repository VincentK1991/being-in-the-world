#!/usr/bin/env bash
set -euo pipefail
repo_root=$(cd "$(dirname "$0")/.." && pwd)
suite_dir="$repo_root/target/w3c-rdf-tests"
revision=d01b141c65ddf8e5b615256f264111118bdb2134
if [[ ! -d "$suite_dir/.git" ]]; then
  git clone https://github.com/w3c/rdf-tests.git "$suite_dir"
fi
git -C "$suite_dir" fetch origin "$revision"
git -C "$suite_dir" checkout --detach "$revision"
cd "$repo_root"
cargo build --locked -p rdf-cli
for suite in rdf-n-triples rdf-n-quads rdf-turtle rdf-trig; do
  target/debug/rdftool test-suite --manifest "$suite_dir/rdf/rdf11/$suite/manifest.ttl" > "target/conformance-$suite.json"
done
for suite in syntax-query syntax-update-1 syntax-update-2; do
  target/debug/rdftool test-suite --manifest "$suite_dir/sparql/sparql11/$suite/manifest.ttl" > "target/conformance-$suite.json"
done
