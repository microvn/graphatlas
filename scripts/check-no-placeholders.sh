#!/usr/bin/env bash
# Foundation-C13 gate (R28): fail CI if unresolved `<placeholder>` literals ship.
#
# What: grep release-artifact paths for literal `<owner>`, `<placeholder>`,
# `TODO_DOMAIN`. Any hit = build fails.
# Why: R28 + AS-014 — install.sh, cosign identity regex, release workflow all
# must resolve the GitHub org + domain BEFORE any tag ships publicly.
# How to apply: exits 0 if clean, 1 with line numbers if dirty. Runs in CI +
# can be run locally (`bash scripts/check-no-placeholders.sh`).

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"

# Paths scanned — release-artifact-adjacent only. Spec and ADR docs deliberately
# excluded: they discuss placeholders as a concept and would always match.
# Note: this script itself would match "<owner>" and "<placeholder>" below —
# excluded via path filter.
PATHS=(
    "$ROOT/Cargo.toml"
    "$ROOT/.github/workflows"
    "$ROOT/crates"
    "$ROOT/src"
    "$ROOT/install.sh"            # not yet created; grep handles missing
)

PATTERNS='(<owner>|<placeholder>|TODO_DOMAIN|YOUR_ORG_HERE)'

FAIL=0
for p in "${PATHS[@]}"; do
    [[ -e "$p" ]] || continue
    if grep -rInE "$PATTERNS" "$p" 2>/dev/null; then
        FAIL=1
    fi
done

if [[ "$FAIL" -ne 0 ]]; then
    echo "ERROR: unresolved placeholder literal(s) found in release-artifact paths." >&2
    echo "Resolve per R28 (register GitHub org + graphatlas.dev domain)." >&2
    exit 1
fi

echo "OK: no placeholder literals in release-artifact paths."
