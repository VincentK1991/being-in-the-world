#!/usr/bin/env bash
set -euo pipefail
# Run explicitly: bash scripts/setup-linux.sh
sudo apt-get update
sudo apt-get install -y build-essential clang libclang-dev cmake pkg-config
cargo fetch --locked
cargo install cargo-nextest --locked
cargo install cargo-deny --locked
