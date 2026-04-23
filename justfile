set shell := ["zsh", "-cu"]

default: ci

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

ci: fmt-check check clippy test deny
