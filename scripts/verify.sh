#!/usr/bin/env bash
set -e
set -x

if [[ -z "${TEXTORA_ARCHITECTURE_MIGRATION:-}" ]]; then
  echo "Running architecture boundary checks..."
  bash scripts/check_architecture.sh
fi

echo "Running formatting checks..."
cargo fmt --all -- --check

echo "Running clippy (warnings as errors)..."
cargo clippy --workspace --all-targets -- -D warnings

echo "Running notora-app tests with deterministic resource usage..."
cargo test -p notora-app -- --test-threads=1

echo "Running workspace tests except notora-app..."
cargo test --workspace --exclude notora-app

echo "All checks passed! Baseline is trusted."
