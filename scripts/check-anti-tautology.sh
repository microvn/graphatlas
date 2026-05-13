#!/usr/bin/env bash
# S-002 AS-006 — anti-tautology grep enforcement (Phase 1+2 MVP).
#
# What: scan `crates/ga-bench/src/gt_gen/h*.rs` for forbidden imports of
# graphatlas analysis types. A bench rule importing `ga_query::dead_code`
# (etc.) defeats the rule's purpose: it scores GA against itself.
# Why: per `docs/specs/graphatlas-v1.1/graphatlas-v1.1-bench.md` §C1 +
# `docs/benchmarks/methodology.md` §"Why H1-text replaced H1-polymorphism".
# How to apply: exit 0 if clean, 1 with offending lines + remediation hint
# if any rule imports a forbidden module. Allowed: `ga_parser`, `ga_store`,
# `ga_query::common`, `ga_query::import_resolve`. Build-time lint
# hardening (build.rs) is deferred to Phase 3.
#
# Local: bash scripts/check-anti-tautology.sh
# CI:    invoked from .github/workflows/ci.yml `anti-tautology` job.
#
# Override REPO_ROOT to scan a different tree (used by integration tests).

set -euo pipefail

ROOT="${REPO_ROOT:-$(cd "$(dirname "$0")/.." && pwd)}"
GT_GEN_DIR="$ROOT/crates/ga-bench/src/gt_gen"

# `ga_query::common::*` is whitelisted helper substrate; everything else
# under ga_query is the production analysis surface and tautological for a
# rule generator to reach into.
FORBIDDEN_RE='^use ga_query::(dead_code|callers|rename_safety|architecture|risk|minimal_context)'

if [ ! -d "$GT_GEN_DIR" ]; then
    # No gt_gen tree under this root — nothing to enforce. Exit clean so the
    # script is a no-op for repos that haven't grown the bench yet.
    exit 0
fi

# Collect candidate rule files.
shopt -s nullglob
files=("$GT_GEN_DIR"/h*.rs)
shopt -u nullglob

if [ "${#files[@]}" -eq 0 ]; then
    exit 0
fi

bad_lines=$(grep -EnH "$FORBIDDEN_RE" "${files[@]}" || true)

if [ -n "$bad_lines" ]; then
    cat >&2 <<EOF
ERROR: anti-tautology policy violation.

A bench rule imported a graphatlas analysis module. Rules must build their
expected sets from raw AST signal (ga_parser, ga_store, ga_query::common
helpers) — never from the same tool they are scoring.

Offending lines:
$bad_lines

Fix: replace the import with a raw-AST equivalent (ga_parser::extract_calls,
ga_parser::extract_references, etc.). See:
  - docs/specs/graphatlas-v1.1/graphatlas-v1.1-bench.md §C1
  - docs/benchmarks/methodology.md §"Why H1-text replaced H1-polymorphism"
EOF
    exit 1
fi

echo "anti-tautology: ${#files[@]} rule file(s) clean."
