#!/usr/bin/env bash
# Bring the whole local stack up from nothing, seed it, and run every live suite against it:
# rule tests, the federation/inference end-to-end checks, the Studio inference panel and documentation in a browser,
# and W3C conformance with the reasoner sidecars. Used by .github/workflows/live-stack.yml.
#
# Ports are published on RDF_PUBLISH_HOST, which CI sets to 0.0.0.0: on plain Linux, containers
# reach the host through host-gateway and cannot see ports bound to the host's loopback. Run it
# only on a throwaway machine; locally the stack keeps its 127.0.0.1 default.
#
#   RDF_PUBLISH_HOST=0.0.0.0 bash scripts/ci/live-stack.sh            # everything
#   bash scripts/ci/live-stack.sh up | seed | test | logs              # one phase
set -euo pipefail
cd "$(dirname "$0")/../.."
compose() { bash scripts/docker-compose.sh "$@"; }
STACKS=(sidecars/federation/compose.yml sidecars/inference/compose.yml sidecars/jena-rules/compose.yml
        sidecars/semantic/compose.yml deploy/local/compose.yml)
step() { printf '\n== %s (%s)\n' "$1" "$(date +%T)"; }

up() {
    step 'build rdftool'
    cargo build --locked -p rdf-cli
    step 'federation sidecars: Postgres, ACME seed, Ontop, Comunica'
    python3 scripts/federation/sync_allowlist.py --check
    test -f sidecars/federation/ontop/postgresql.jar || curl -fsSL -o sidecars/federation/ontop/postgresql.jar \
        https://repo1.maven.org/maven2/org/postgresql/postgresql/42.7.4/postgresql-42.7.4.jar
    compose -f sidecars/federation/compose.yml up -d --wait --wait-timeout 300 postgres
    python3 scripts/seed/acme_postgres.py
    compose -f sidecars/federation/compose.yml up -d --build --wait --wait-timeout 300
    step 'reasoner sidecars'
    compose -f sidecars/inference/compose.yml up -d --build --wait
    compose -f sidecars/jena-rules/compose.yml up -d --build --wait
    step 'semantic search (Qdrant, CPU embeddings; the first start downloads the model)'
    compose -f sidecars/semantic/compose.yml up -d --build --wait --wait-timeout 600
    step 'backend and Studio'
    compose -f deploy/local/compose.yml up -d --build --wait backend ui
}

seed() {
    step 'example graphs, rules and catalog'
    python3 scripts/local-demo.py
    step 'ACME insurance graph'
    python3 scripts/seed/acme_insurance.py
    step 'claims and finance datasets, indexes, ACME jobs'
    python3 scripts/seed/federation_demo.py
}

test_all() {
    local failed=0
    run() { step "$1"; shift; "$@" || { echo "FAILED: $*"; failed=1; }; }
    run 'rule tests' python3 scripts/test/rule_tests.py
    run 'federation and inference end to end' python3 scripts/test/federation_e2e.py
    run 'Studio inference panel (browser)' node scripts/ui/inference-deployed.mjs
    run 'Documentation site (browser)' node scripts/ui/docs-deployed.mjs
    run 'W3C conformance with reasoners' env CONFORMANCE_REASONERS=1 bash scripts/conformance.sh
    return "$failed"
}

logs() {
    mkdir -p target/ci-logs
    for file in "${STACKS[@]}"; do
        name=$(basename "$(dirname "$file")")
        compose -f "$file" ps -a > "target/ci-logs/$name-ps.txt" 2>&1 || true
        compose -f "$file" logs --no-color --timestamps > "target/ci-logs/$name.log" 2>&1 || true
    done
    echo "container logs in target/ci-logs"
}

case "${1:-all}" in
    up) up ;;
    seed) seed ;;
    test) test_all ;;
    logs) logs ;;
    # Plain sequence so `set -e` stops at the first failing step; CI collects `logs` either way.
    all) up; seed; test_all ;;
    *) echo "usage: $0 [all|up|seed|test|logs]" >&2; exit 2 ;;
esac
