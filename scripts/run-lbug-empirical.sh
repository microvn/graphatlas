#!/usr/bin/env bash
# v1.5 PR9 — Run lbug lifecycle empirical tests and emit per-test JSON
# artifacts at $OUT_DIR/<test_name>.json (defaults to target/lbug_lifecycle).
#
# Contract (graphatlas-v1.5-reindex-empirical.md AS-003):
#   {"result": "PASS" | "FAIL", "test": "<name>", "failure_reason": null | "<msg>"}
#
# Used by CI (.github/workflows/ci.yml) to upload as an artifact. Bench
# gate readers (PR9 incremental decision) inspect these files to decide
# whether dependent-expansion pipeline ships or falls back to full
# rebuild.
set -euo pipefail

OUT_DIR="${LBUG_EMPIRICAL_OUT:-${CARGO_TARGET_DIR:-target}/lbug_lifecycle}"
mkdir -p "$OUT_DIR"

# Test name → integration test binary mapping.
TESTS=(
  "as_001_outgoing_cross_file_edge_survives_delete_insert_cycle:lbug_lifecycle_cross_file_edge_integrity"
  "as_002_incoming_cross_file_edges_to_changed_file_preserved:lbug_lifecycle_cross_file_edge_integrity"
  "posix_wait_for_handle_release_is_immediate_noop:lbug_lifecycle_close_rm_reopen"
)

OVERALL_RESULT=0
for entry in "${TESTS[@]}"; do
  test_name="${entry%%:*}"
  bin_name="${entry##*:}"
  out_file="$OUT_DIR/${test_name}.json"
  if cargo test -p ga-index --test "$bin_name" "$test_name" -- --exact --nocapture >/tmp/lbug_test_out 2>&1; then
    printf '{"result":"PASS","test":"%s","binary":"%s","failure_reason":null}\n' \
      "$test_name" "$bin_name" > "$out_file"
    echo "PASS  $test_name → $out_file"
  else
    reason=$(head -c 500 /tmp/lbug_test_out | tr '\n' ' ' | sed 's/"/\\"/g')
    printf '{"result":"FAIL","test":"%s","binary":"%s","failure_reason":"%s"}\n' \
      "$test_name" "$bin_name" "$reason" > "$out_file"
    echo "FAIL  $test_name → $out_file"
    OVERALL_RESULT=1
  fi
done

exit $OVERALL_RESULT
