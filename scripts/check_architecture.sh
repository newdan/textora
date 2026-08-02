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
  textora-ui winit wgpu textora-render textora-shaping textora-markdown textora-sync notora-core notora-app
check_forbidden_dependency textora-appkit-shell \
  textora-markdown textora-sync textora-app notora-core notora-app
check_forbidden_dependency textora-ui \
  notora-core notora-app
check_forbidden_dependency notora-core \
  textora-ui winit wgpu textora-render textora-shaping textora-appkit-core textora-appkit-shell \
  textora-markdown textora-sync textora-app
check_forbidden_dependency notora-app \
  textora-app textora-sync

check_forbidden_source_tokens() {
  local source_dir="$1"
  shift

  [[ -d "$source_dir" ]] || return 0

  local forbidden_token
  for forbidden_token in "$@"; do
    if rg -n --glob '*.rs' --fixed-strings "$forbidden_token" "$source_dir"; then
      echo "forbidden product or escape-hatch token '$forbidden_token' found in $source_dir" >&2
      exit 1
    fi
  done
}

local_product_markdown="textora"_"markdown"
local_product_sync="textora"_"sync"
local_textora_product="Textora""Product"
local_notora_product="Notora""Product"
local_note_id="Note""Id"
local_navigation_scope="Navigation""Scope"
local_edit_snapshot=".edit""+"
local_notora_snapshot=".notora"
check_forbidden_source_tokens crates/appkit-shell \
  "$local_product_markdown" \
  "$local_product_sync" \
  "$local_textora_product" \
  "$local_notora_product" \
  "$local_note_id" \
  "$local_navigation_scope" \
  "$local_edit_snapshot" \
  "$local_notora_snapshot"

check_forbidden_source_tokens crates/appkit-core \
  "$local_textora_product" \
  "$local_notora_product" \
  "$local_note_id" \
  "$local_navigation_scope" \
  "$local_edit_snapshot" \
  "$local_notora_snapshot"

check_forbidden_source_tokens crates/ui \
  "$local_notora_product" \
  "$local_note_id" \
  "$local_navigation_scope" \
  "$local_notora_snapshot"

check_forbidden_source_tokens crates/notora-core \
  "$local_edit_snapshot"
check_forbidden_source_tokens crates/notora-app \
  "$local_edit_snapshot"

local_runtime_dir="crates/appkit-shell/src/editor_runtime"
local_workspace_mut="workspace"_"_mut"
local_document_mut="document"_"_mut"
local_gpu_mut="gpu"_"_mut"
local_runtime_store_mut="tab_runtime_store"_"_mut"
check_forbidden_source_tokens "$local_runtime_dir" \
  "$local_workspace_mut" \
  "$local_document_mut" \
  "$local_gpu_mut" \
  "$local_runtime_store_mut"

if rg -n 'SyncSettings|textora_sync' crates/ui/src; then
  echo "ui must not contain textora sync product types" >&2
  exit 1
fi
