#!/usr/bin/env bash
set -euo pipefail

repository_root="$(cd "$(dirname "$0")/.." && pwd)"
audit_directory="${1:-$repository_root/target/editing-audit}"
mkdir -p "$audit_directory"
audit_directory="$(cd "$audit_directory" && pwd)"
cd "$repository_root"
python3 scripts/editing-audit.py prepare "$audit_directory"

run_phase() {
    local phase_name="$1"
    shift
    local phase_status=0
    "$@" > "$audit_directory/$phase_name.log" 2>&1 || phase_status=$?
    printf '%s\n' "$phase_status" > "$audit_directory/$phase_name.status"
    printf '%s: exit %s (%s)\n' "$phase_name" "$phase_status" "$audit_directory/$phase_name.log"
}

run_phase build cargo build -p textora-app --bin textora
if [[ "$(cat "$audit_directory/build.status")" == 0 ]]; then
    python3 scripts/editing-audit.py bundle "$audit_directory"
fi
run_phase formatting cargo fmt --all -- --check
run_phase semantic cargo test -p textora-markdown --test system_boundary_audit -- --include-ignored --nocapture --test-threads=1
run_phase file_roundtrip cargo test -p textora-app --lib editing_boundary_audit_file_open_save -- --include-ignored --test-threads=1
run_phase history cargo test -p textora-app --lib editing_boundary_audit_undo_redo -- --test-threads=1
run_harness() {
    python3 -B -m unittest discover -s scripts/tests -p 'test_editing_audit.py' &&
        node --test scripts/tests/editing-cua.test.mjs scripts/tests/editing-native-cua.test.mjs
}
run_phase harness run_harness
run_phase full_verify ./scripts/verify.sh
audit_status=0
python3 scripts/editing-audit.py summarize "$audit_directory" || audit_status=$?

printf 'UI manifest: %s/manifest.json\n' "$audit_directory"
printf 'Use scripts/editing-cua.mjs in Computer Use for native keyboard and screenshots.\n'
exit "$audit_status"
