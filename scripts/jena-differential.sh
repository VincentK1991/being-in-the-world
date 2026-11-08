#!/usr/bin/env bash
set -euo pipefail
repo_root=$(cd "$(dirname "$0")/.." && pwd)
cd "$repo_root"
mkdir -p target/oracle
archive=target/oracle/apache-jena-4.10.0.tar.gz
if [[ ! -f "$archive" ]]; then
  curl -fL https://archive.apache.org/dist/jena/binaries/apache-jena-4.10.0.tar.gz -o "$archive"
fi
printf '%s  %s\n' c1097988fda802d157d031ba79007fd949a8ca2556bb383134021f48b44ade511ba6e2690ac4b1548c57df0476267f18ffce94101a87e094c0a161a78766d341 "$archive" | sha512sum -c -
tar -xzf "$archive" -C target/oracle
cargo build --locked -p rdf-testkit --example compare_oracle
for query in tests/oracle/*.rq; do
  result="target/oracle/$(basename "$query").json"
  target/oracle/apache-jena-4.10.0/bin/arq --data=tests/oracle/data.ttl --query="$query" --results=JSON > "$result"
  target/debug/examples/compare_oracle tests/oracle/data.ttl "$query" "$result"
done
