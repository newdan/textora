#!/usr/bin/env bash
set -euo pipefail

check_forbidden_dependency() {
  local package_name="$1"
  shift
  local dependency_tree
  dependency_tree="$(cargo tree -p "$package_name" --prefix none)"
  for forbidden_name in "$@"; do
    if printf '%s\n' "$dependency_tree" | grep -Eq "^${forbidden_name}( |$)"; then
      echo "${package_name} must not depend on ${forbidden_name}" >&2
      exit 1
    fi
  done
}

check_forbidden_dependency textora-appkit-core \
  textora-ui winit wgpu textora-render textora-shaping textora-markdown textora-sync
check_forbidden_dependency textora-appkit-shell \
  textora-markdown textora-sync textora-app

if rg -n '\.edit\+' crates/appkit-core crates/appkit-shell; then
  echo "shared crates must not hardcode .edit+" >&2
  exit 1
fi

if rg -n 'SyncSettings|textora_sync' crates/ui/src; then
  echo "ui must not contain textora sync product types" >&2
  exit 1
fi
