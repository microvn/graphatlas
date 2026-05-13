#!/usr/bin/env bash
#
# Per-fixture seed-quality audit (Codex insight #2 → guide §13 L14).
#
# Reports the share of "weak" seed_symbols in each repo's
# benches/uc-impact/raw/<repo>-tasks.json. A high weak-share invalidates
# graph-traversal benchmarks for that fixture — seeds become trivially
# winnable by ripgrep / unwinnable by any retriever.
#
# Weak categories:
#   stdlib   — language stdlib symbols (.NET Substring, ArgumentException, ...)
#   priv     — C# private-field convention (^_[a-z])
#   short    — < 3 chars
#   varlike  — common loop/local variable names (i, x, msg, channel, ...)
#
# Run after a re-mine to catch silent regressions before they hit M2.
#
# Usage:
#   scripts/audit-seed-quality.sh                  # all repos
#   scripts/audit-seed-quality.sh MQTTnet Polly    # specific fixtures

set -euo pipefail

RAW_DIR="benches/uc-impact/raw"

# Lang-scoped stdlib set — keep in sync with STDLIB_SYMBOLS in extract-seeds.ts.
# Used as a *forensic* check (would isLikelyBadSymbol have rejected this seed?).
CSHARP_STDLIB="Substring|ToString|ToArray|ToList|Equals|GetHashCode|Contains|IndexOf|Compare|CompareTo|FromResult|FromException|WhenAll|WhenAny|ConfigureAwait|ContinueWith|Run|Delay|ArgumentException|ArgumentNullException|ArgumentOutOfRangeException|InvalidOperationException|NotSupportedException|NotImplementedException|ObjectDisposedException|TimeoutException|OperationCanceledException|NullReferenceException|IndexOutOfRangeException"

# Generic local-var names that leak through the regex fallback.
VARLIKE="^(i|j|k|x|y|n|m|s|msg|message|channel|ctx|err|tmp|val|key|item|index|maxIndex|count|result|data)$"

audit_one() {
  local file="$1"
  local repo lang
  repo=$(jq -r '.repo' "$file")
  lang=$(jq -r '.lang' "$file")
  local total
  total=$(jq '.tasks | length' "$file")
  if [ "$total" -eq 0 ]; then
    printf "%-25s lang=%-10s tasks=0 (skip)\n" "$repo" "$lang"
    return
  fi

  local short stdlib priv varlike
  short=$(jq -r '.tasks[].seed_symbol' "$file" | awk 'length($0) < 3' | wc -l | tr -d ' ')
  varlike=$(jq -r '.tasks[].seed_symbol' "$file" | grep -cE "$VARLIKE" || true)

  if [ "$lang" = "csharp" ]; then
    stdlib=$(jq -r '.tasks[].seed_symbol' "$file" | grep -cE "^($CSHARP_STDLIB)$" || true)
    priv=$(jq -r '.tasks[].seed_symbol' "$file" | grep -cE "^_[a-z]" || true)
  else
    stdlib=0
    priv=0
  fi

  local bad weak_pct
  bad=$((short + stdlib + priv + varlike))
  weak_pct=$(awk "BEGIN { printf \"%.1f\", 100 * $bad / $total }")

  local flag=""
  # Threshold per L14: >20% weak seeds → footnote in leaderboard.
  if awk "BEGIN { exit !($weak_pct > 20) }"; then
    flag=" ⚠️"
  fi

  printf "%-25s lang=%-10s tasks=%-3s weak=%-3s (%5s%%)  short=%s stdlib=%s priv=%s varlike=%s%s\n" \
    "$repo" "$lang" "$total" "$bad" "$weak_pct" "$short" "$stdlib" "$priv" "$varlike" "$flag"
}

if [ "$#" -gt 0 ]; then
  files=()
  for repo in "$@"; do
    f="$RAW_DIR/${repo}-tasks.json"
    if [ ! -f "$f" ]; then
      echo "[SKIP] $f not found" >&2
      continue
    fi
    files+=("$f")
  done
else
  shopt -s nullglob
  files=("$RAW_DIR"/*-tasks.json)
fi

if [ "${#files[@]}" -eq 0 ]; then
  echo "No -tasks.json files to audit." >&2
  exit 1
fi

echo "=== Seed-quality audit (threshold ⚠️ = weak >20%) ==="
for f in "${files[@]}"; do
  audit_one "$f"
done
