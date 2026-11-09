set shell := ["zsh", "-cu"]

# List available commands (validation is explicit: `just ci`).
default:
    @just --list

fmt:
    cargo fmt --all

fmt-check:
    cargo fmt --all --check

check:
    cargo check --workspace --all-targets

clippy:
    cargo clippy --workspace --all-targets -- -D warnings

test:
    cargo nextest run --workspace --all-targets

deny:
    cargo deny check

ci: fmt-check check clippy test deny allowlist-check docs-check

# The documentation's references to repository files, symbols and ADRs must exist.
docs-check:
    python3 scripts/docs/check_paths.py

# The Comunica allowlist is generated from the dataset configs; fail if it drifted.
allowlist-check:
    python3 scripts/federation/sync_allowlist.py --check

# Build and start the localhost backend + persistent RDF store in Docker.
backend:
    cargo build --locked -p rdf-cli
    bash scripts/docker-compose.sh -f deploy/local/compose.yml up -d --build --wait backend

# Build and start Astro/React Studio at http://127.0.0.1:4321 (start backend first).
ui:
    bash scripts/docker-compose.sh -f deploy/local/compose.yml up -d --build --wait ui

# Start the existing OWL-RL and Jena inference sidecars.
inference:
    bash scripts/docker-compose.sh -f sidecars/inference/compose.yml up -d --build --wait
    bash scripts/docker-compose.sh -f sidecars/jena-rules/compose.yml up -d --build --wait

# Start the Jena SHACL Core validator (shares the existing Jena rules sidecar).
shacl:
    bash scripts/docker-compose.sh -f sidecars/jena-rules/compose.yml up -d --build --wait

# Validate SHACL with the real Jena provider and the deployed HTTP workflow.
shacl-test:
    cargo test --locked -p rdf-plugin-shacl
    cargo test --locked -p rdf-plugin-shacl --test live -- --ignored
    python3 scripts/test/shacl_e2e.py

# Start Qdrant and local CUDA embeddings (requires NVIDIA GPU support).
semantic-gpu:
    bash scripts/docker-compose.sh -f sidecars/semantic/compose.yml -f sidecars/semantic/compose.gpu.yml up -d --build --wait --wait-timeout 300

# Start Qdrant and local CPU embeddings.
semantic-cpu:
    bash scripts/docker-compose.sh -f sidecars/semantic/compose.yml up -d --build --wait --wait-timeout 300

# Start Postgres, Ontop (virtual graph) and Comunica (SERVICE federation); seeds ACME on first run.
federation:
    python3 scripts/federation/sync_allowlist.py
    test -f sidecars/federation/ontop/postgresql.jar || curl -fsSL -o sidecars/federation/ontop/postgresql.jar https://repo1.maven.org/maven2/org/postgresql/postgresql/42.7.4/postgresql-42.7.4.jar
    bash scripts/docker-compose.sh -f sidecars/federation/compose.yml up -d --wait --wait-timeout 300 postgres
    python3 scripts/seed/acme_postgres.py
    bash scripts/docker-compose.sh -f sidecars/federation/compose.yml up -d --build --wait --wait-timeout 300
    python3 scripts/seed/federation_demo.py

# Run every inference job's rule tests (needs `just dev`, and the OWL-RL and Jena sidecars for those jobs).
# W3C conformance: syntax suites, SPARQL evaluation and entailment, each held to tests/conformance/.
conformance:
    CONFORMANCE_REASONERS=1 bash scripts/conformance.sh

# Rewrite a conformance baseline after a deliberate change, e.g. `just conformance-update sparql --engine database`.
conformance-update *args:
    python3 scripts/conformance/baseline.py {{args}} --update

rule-tests:
    python3 scripts/test/rule_tests.py

# End-to-end virtual graph and cross-dataset federation checks (needs `just dev` and `just federation`).
federation-test:
    python3 scripts/test/federation_e2e.py

# Read-only MCP server for AI agents (stdio); needs the gateway on 127.0.0.1:7878.
mcp:
    cargo run --release -p rdf-mcp

# MCP tools and the cross-plugin query examples, end to end against the running, seeded stack.
mcp-test:
    cargo test -p rdf-mcp --test live --test composition -- --ignored

# Hot-reload dev loop: Studio HMR plus backend rebuild + restart on Rust changes.
dev:
    bash scripts/dev.sh

# Hot-reload running sidecars (Python sync + restart, Jena rebuild). Start them first.
dev-sidecars:
    bash scripts/dev-sidecars.sh

# Show UI/backend container status.
status:
    bash scripts/docker-compose.sh -f deploy/local/compose.yml ps

# Follow UI/backend logs.
logs:
    bash scripts/docker-compose.sh -f deploy/local/compose.yml logs -f --tail=100

# Stop UI/backend; preserve the persistent database volume and sidecars.
down:
    bash scripts/docker-compose.sh -f deploy/local/compose.yml down

# Add sample RDF, rebuild inference/search, and validate through the running UI proxy.
demo:
    python3 scripts/local-demo.py
