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

echo "Running tests..."
cargo test --workspace

echo "All checks passed! Baseline is trusted."
