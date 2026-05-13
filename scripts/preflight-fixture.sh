#!/usr/bin/env bash
#
# Pre-flight check for a candidate bench fixture per
# docs/guide/dataset-for-new-language.md §1.5.
#
# Runs WITHOUT git submodule add — uses a shallow clone in $TMPDIR.
# Outputs PASS/FAIL/SOFT-FAIL per criterion + paradigm matrix detection
# (§0.6-G multi-target, §0.6-H generated dirs, §0.6-K test conventions).
#
# Usage:
#   scripts/preflight-fixture.sh <repo-url> <lang> [<expected-fixture-name>]
#
# Example:
#   scripts/preflight-fixture.sh \
#     https://github.com/Kotlin/kotlinx.coroutines.git \
#     kotlin
#
# Exit codes:
#   0 = all hard requirements pass (eligible)
#   1 = at least one hard requirement fails (ineligible)
#   2 = soft-fail (H2 below 20 but other hard reqs OK; document yield expectation)

set -euo pipefail

URL="${1:?missing arg 1: repo URL}"
LANG="${2:?missing arg 2: language (kotlin|java|csharp|ruby|swift|php|cpp|...)}"
NAME="${3:-$(basename "$URL" .git | tr '.' '-')}"

# Per-language extension mapping (primary file extension for LoC count).
case "$LANG" in
  kotlin) EXT_MAIN="kt"; EXT_ALL="kt|kts" ;;
  java) EXT_MAIN="java"; EXT_ALL="java" ;;
  csharp|cs) EXT_MAIN="cs"; EXT_ALL="cs" ;;
  ruby|rb) EXT_MAIN="rb"; EXT_ALL="rb|rake" ;;
  swift) EXT_MAIN="swift"; EXT_ALL="swift" ;;
  php) EXT_MAIN="php"; EXT_ALL="php" ;;
  cpp|cxx) EXT_MAIN="cpp"; EXT_ALL="cpp|cc|cxx|h|hpp" ;;
  python|py) EXT_MAIN="py"; EXT_ALL="py|pyi" ;;
  typescript|ts) EXT_MAIN="ts"; EXT_ALL="ts|tsx|cts|mts" ;;
  javascript|js) EXT_MAIN="js"; EXT_ALL="js|jsx|mjs|cjs" ;;
  go) EXT_MAIN="go"; EXT_ALL="go" ;;
  rust|rs) EXT_MAIN="rs"; EXT_ALL="rs" ;;
  *) echo "preflight: unknown lang '$LANG' — extend EXT_MAIN map"; exit 1 ;;
esac

TMP="$(mktemp -d -t "preflight-$NAME-XXXXXX")"
trap 'rm -rf "$TMP"' EXIT
cd "$TMP"

echo "================================================================"
echo "Pre-flight: $NAME ($LANG)"
echo "URL:        $URL"
echo "Workdir:    $TMP"
echo "================================================================"

# Clone shallow (depth 500 — enough for fix-commit history; --filter blob:none
# avoids downloading file contents we don't need for log mining).
echo
echo "[setup] Shallow-cloning depth=500 (this may take 30-60s for large repos)..."
git clone --depth=500 --filter=blob:none --no-tags --quiet "$URL" "$NAME" 2>&1 | tail -5
cd "$NAME"

PASS=0; FAIL=0; SOFT_FAIL=0

# ──────────────────────────────────────────────────────────────────────
# H1 — LoC range (5K-150K, sweet 20K-50K)
# ──────────────────────────────────────────────────────────────────────
echo
echo "── §1.1 H1: LoC range ────────────────────────────────────────────"
# Build a `-name "*.ext1" -o -name "*.ext2" ...` clause from EXT_ALL.
NAME_CLAUSE=$(echo "$EXT_ALL" | tr '|' '\n' | awk 'BEGIN{first=1}
  {
    if (first) { printf "-name \"*.%s\"", $0; first=0 }
    else { printf " -o -name \"*.%s\"", $0 }
  }')
LINES=$(eval "find . -type f \\( $NAME_CLAUSE \\) \
  -not -path '*/build/*' -not -path '*/target/*' \
  -not -path '*/node_modules/*' -not -path '*/vendor/*' \
  -not -path '*/.git/*' -not -path '*/generated/*' \
  -exec wc -l {} +" 2>/dev/null | tail -1 | awk '{print $1}')
LINES=${LINES:-0}
echo "  $LANG main LoC (excl. build/target/vendored/generated): $LINES"
if [ "$LINES" -ge 5000 ] && [ "$LINES" -le 150000 ]; then
  if [ "$LINES" -ge 20000 ] && [ "$LINES" -le 50000 ]; then
    echo "  → H1 PASS (in sweet spot 20K-50K)"
  else
    echo "  → H1 PASS (in range 5K-150K, outside sweet spot)"
  fi
  PASS=$((PASS + 1))
else
  echo "  → H1 FAIL (need 5K-150K)"
  FAIL=$((FAIL + 1))
fi

# ──────────────────────────────────────────────────────────────────────
# H2 — Fix-commit count after filter (≥20)
# ──────────────────────────────────────────────────────────────────────
echo
echo "── §1.1 H2: Fix-commit volume ───────────────────────────────────"
# Raw count — case-insensitive grep for fix patterns.
FIX_RAW=$(git log --since="2 years ago" \
  --grep='fix\|fixes\|bug\|issue\|patch\|defect' -i \
  --pretty=format:"%h" 2>/dev/null | wc -l | xargs)
echo "  Raw fix commits (last 2y, case-insensitive): $FIX_RAW"

# Filtered count — apply mining filter heuristic: 1≤src_files≤15 AND ≥1 test file.
# Use git log --name-only and AWK to count per-commit file shapes.
FIX_FILTERED=$(git log --since="2 years ago" \
  --grep='fix\|fixes\|bug\|issue\|patch\|defect' -i \
  --name-only --pretty=format:"@@%h" 2>/dev/null | \
  awk -v ext="$EXT_MAIN" '
    BEGIN { n=0; src=0; tst=0 }
    /^@@/ {
      if (src >= 1 && src <= 15 && tst >= 1) n++
      src = 0; tst = 0
      next
    }
    /^$/ { next }
    /(Test|Tests|Spec|IT|_test|_spec)\.|\/(test|tests|__tests__|spec|specs)\// {
      tst++; next
    }
    /\./ {
      if (match($0, "\\.("ext")$")) src++
    }
    END {
      if (src >= 1 && src <= 15 && tst >= 1) n++
      print n
    }')
FIX_FILTERED=${FIX_FILTERED:-0}
echo "  Filtered (1≤src≤15 + ≥1 test): ~$FIX_FILTERED  | gate: ≥20 (mockito precedent: 15)"
if [ "$FIX_FILTERED" -ge 20 ]; then
  echo "  → H2 PASS"
  PASS=$((PASS + 1))
elif [ "$FIX_FILTERED" -ge 10 ]; then
  echo "  → H2 SOFT-FAIL (≥10 acceptable per §5.7; expect ≤6 dev / N-6 test split)"
  SOFT_FAIL=$((SOFT_FAIL + 1))
else
  echo "  → H2 FAIL (<10; investigate fixture choice)"
  FAIL=$((FAIL + 1))
fi

# ──────────────────────────────────────────────────────────────────────
# H3 — Conventional test layout
# ──────────────────────────────────────────────────────────────────────
echo
echo "── §1.1 H3: Test layout convention ──────────────────────────────"
case "$LANG" in
  kotlin)
    TEST_FILES=$(find . -type f \
      \( -name "*Test.kt" -o -name "*Tests.kt" -o -name "*Spec.kt" -o -name "*IT.kt" \) \
      -not -path "*/build/*" -not -path "*/.git/*" 2>/dev/null | wc -l | xargs)
    ;;
  java)
    TEST_FILES=$(find . -type f \
      \( -name "*Test.java" -o -name "*Tests.java" -o -name "*Spec.java" -o -name "*IT.java" \) \
      -not -path "*/target/*" -not -path "*/.git/*" 2>/dev/null | wc -l | xargs)
    ;;
  csharp|cs)
    TEST_FILES=$(find . -type f \
      \( -name "*Test.cs" -o -name "*Tests.cs" \) \
      -not -path "*/bin/*" -not -path "*/obj/*" -not -path "*/.git/*" 2>/dev/null | wc -l | xargs)
    ;;
  ruby|rb)
    # RSpec suffix `_spec.rb`, Minitest suffix `_test.rb`, Minitest prefix
    # `test_*.rb` (jekyll uses this), Test::Unit `tc_*.rb`. Also accept any
    # `.rb` under `test/` or `spec/` dirs.
    TEST_FILES=$(find . -type f \
      \( -name "*_spec.rb" -o -name "*_test.rb" -o -name "test_*.rb" -o -name "tc_*.rb" \) \
      -not -path "*/.git/*" 2>/dev/null | wc -l | xargs)
    DIR_TESTS=$(find . -type f -name "*.rb" \( -path "*/test/*" -o -path "*/spec/*" \) \
      -not -path "*/.git/*" 2>/dev/null | wc -l | xargs)
    DIR_TESTS=${DIR_TESTS:-0}
    if [ "$DIR_TESTS" -gt "$TEST_FILES" ]; then
      TEST_FILES=$DIR_TESTS
    fi
    ;;
  swift)
    TEST_FILES=$(find . -type f -name "*Tests.swift" \
      -not -path "*/.build/*" -not -path "*/.git/*" 2>/dev/null | wc -l | xargs)
    ;;
  php)
    TEST_FILES=$(find . -type f -name "*Test.php" \
      -not -path "*/vendor/*" -not -path "*/.git/*" 2>/dev/null | wc -l | xargs)
    ;;
  *)
    # Generic: fall back to path-segment detection
    TEST_FILES=$(find . -type f -path "*/test*/*" -o -path "*/spec*/*" \
      -not -path "*/build/*" -not -path "*/.git/*" 2>/dev/null | wc -l | xargs)
    ;;
esac
TEST_FILES=${TEST_FILES:-0}
echo "  Test files (suffix-detected for $LANG): $TEST_FILES  | gate: ≥5"
if [ "$TEST_FILES" -ge 5 ]; then
  echo "  → H3 PASS"
  PASS=$((PASS + 1))
else
  echo "  → H3 FAIL (no recognizable test convention)"
  FAIL=$((FAIL + 1))
fi

# ──────────────────────────────────────────────────────────────────────
# H4 — License (permissive)
# ──────────────────────────────────────────────────────────────────────
echo
echo "── §1.1 H4: License ─────────────────────────────────────────────"
if [ -f LICENSE ] || [ -f LICENSE.txt ] || [ -f LICENSE.md ]; then
  LIC_FILE=$(ls LICENSE* 2>/dev/null | head -1)
  LIC=$(head -10 "$LIC_FILE" | grep -oE "Apache License|MIT License|BSD|GNU General Public|GNU Lesser|Mozilla Public|Eclipse Public|Unlicense|ISC" | head -1)
  echo "  License (from $LIC_FILE): ${LIC:-UNKNOWN}"
  case "$LIC" in
    "Apache License"|"MIT License"|BSD|"Mozilla Public"|Unlicense|ISC)
      echo "  → H4 PASS"
      PASS=$((PASS + 1)) ;;
    "GNU General Public"|"GNU Lesser"|"Eclipse Public")
      echo "  → H4 FAIL — copyleft, cannot derive bench artifact"
      FAIL=$((FAIL + 1)) ;;
    *)
      echo "  → H4 INVESTIGATE — license unrecognized; review manually"
      SOFT_FAIL=$((SOFT_FAIL + 1)) ;;
  esac
else
  echo "  → H4 FAIL — no LICENSE file at repo root"
  FAIL=$((FAIL + 1))
fi

# ──────────────────────────────────────────────────────────────────────
# H5 — Active maintenance
# ──────────────────────────────────────────────────────────────────────
echo
echo "── §1.1 H5: Active maintenance ──────────────────────────────────"
LAST_CI=$(git log -1 --format="%ci" 2>/dev/null)
LAST_REL=$(git log -1 --format="%cr" 2>/dev/null)
echo "  Last commit: $LAST_REL ($LAST_CI)"
# Cross-platform date arithmetic — use git's seconds-since-epoch
LAST_SEC=$(git log -1 --format="%ct" 2>/dev/null)
NOW_SEC=$(date +%s)
DAYS_AGO=$(( (NOW_SEC - LAST_SEC) / 86400 ))
echo "  Days since last commit: $DAYS_AGO  | gate: ≤180 (6 months)"
if [ "$DAYS_AGO" -le 180 ]; then
  echo "  → H5 PASS"
  PASS=$((PASS + 1))
else
  echo "  → H5 FAIL (stale repo — grammar/lang drift won't surface)"
  FAIL=$((FAIL + 1))
fi

# ──────────────────────────────────────────────────────────────────────
# §0.6-G — Multi-target source set detection
# ──────────────────────────────────────────────────────────────────────
echo
echo "── §0.6-G: Multi-target source sets ─────────────────────────────"
case "$LANG" in
  kotlin)
    COMMON=$(find . -path "*/commonMain*" -type d 2>/dev/null | head -1)
    JVM=$(find . -path "*/jvmMain*" -type d 2>/dev/null | head -1)
    ANDROID=$(find . -path "*/androidMain*" -o -path "*/androidTest*" -type d 2>/dev/null | head -1)
    NATIVE=$(find . -path "*/nativeMain*" -o -path "*/iosMain*" -type d 2>/dev/null | head -1)
    JS=$(find . -path "*/jsMain*" -type d 2>/dev/null | head -1)
    [ -n "$COMMON" ] && echo "  commonMain: yes"
    [ -n "$JVM" ] && echo "  jvmMain: yes"
    [ -n "$ANDROID" ] && echo "  androidMain/androidTest: yes"
    [ -n "$NATIVE" ] && echo "  nativeMain/iosMain: yes"
    [ -n "$JS" ] && echo "  jsMain: yes"
    if [ -n "$COMMON$JVM$ANDROID$NATIVE$JS" ]; then
      echo "  → §0.6-G YES — KMP detected. Multi-target lock-step §4.2 extension required BEFORE submodule lands."
    else
      echo "  → §0.6-G NO — flat src/ layout"
    fi
    ;;
  swift)
    PLATFORMS=$(find Sources/* -mindepth 1 -maxdepth 1 -type d 2>/dev/null | head -3)
    if [ -n "$PLATFORMS" ]; then
      echo "  Per-platform Sources/<Target>/ detected:"
      echo "$PLATFORMS" | sed 's|^|    |'
      echo "  → §0.6-G YES — extend lock-step for SwiftPM platforms"
    else
      echo "  → §0.6-G NO"
    fi
    ;;
  *)
    echo "  → §0.6-G N/A for $LANG (no per-language multi-target check implemented)"
    ;;
esac

# ──────────────────────────────────────────────────────────────────────
# §0.6-H — Generated code dir detection
# ──────────────────────────────────────────────────────────────────────
echo
echo "── §0.6-H: Generated code dirs ──────────────────────────────────"
GEN_KSP=$(find . -path "*/build/generated/ksp/*" -type d 2>/dev/null | head -1)
GEN_KAPT=$(find . -path "*/build/generated/source/kapt/*" -type d 2>/dev/null | head -1)
GEN_GENERIC=$(find . -name "generated" -type d -not -path "*/.git/*" -not -path "*/build/*" 2>/dev/null | head -3)
DESIGNER=$(find . -name "*.designer.cs" -type f 2>/dev/null | head -1)
PB=$(find . \( -name "*.pb.go" -o -name "*.pb.cc" -o -name "*_pb.py" \) -type f 2>/dev/null | head -1)
GEN_FOUND=0
[ -n "$GEN_KSP" ] && { echo "  ksp generated: $GEN_KSP"; GEN_FOUND=1; }
[ -n "$GEN_KAPT" ] && { echo "  kapt generated: $GEN_KAPT"; GEN_FOUND=1; }
[ -n "$GEN_GENERIC" ] && { echo "  generic 'generated' dirs:"; echo "$GEN_GENERIC" | sed 's|^|    |'; GEN_FOUND=1; }
[ -n "$DESIGNER" ] && { echo "  WinForms designer: $DESIGNER"; GEN_FOUND=1; }
[ -n "$PB" ] && { echo "  protoc output: $PB"; GEN_FOUND=1; }
if [ "$GEN_FOUND" -eq 1 ]; then
  echo "  → §0.6-H YES — extend VENDORED_PATTERN before mining"
else
  echo "  → §0.6-H NO (no generated dirs detected)"
fi

# ──────────────────────────────────────────────────────────────────────
# §0.6-D — DI annotation detection (sample probe)
# ──────────────────────────────────────────────────────────────────────
echo
echo "── §0.6-D: DI annotation patterns (sample) ──────────────────────"
# Probe is best-effort; relax pipefail so empty grep matches don't kill us.
set +e
case "$LANG" in
  kotlin|java)
    SAMPLE_FILES=$(find . -type f \( -name "*.kt" -o -name "*.java" \) \
      -not -path "*/build/*" -not -path "*/target/*" \
      -not -path "*/.git/*" -not -path "*/generated/*" 2>/dev/null | head -200)
    DI_HITS=0
    if [ -n "$SAMPLE_FILES" ]; then
      DI_HITS=$(printf '%s\n' "$SAMPLE_FILES" | tr '\n' '\0' | \
        xargs -0 grep -lE "@(Inject|Autowired|Composable|Serializable|Component|Service|Repository|SerialName|Contextual)" 2>/dev/null | wc -l | xargs)
    fi
    echo "  Files with DI/annotation hits (sample of first 200 files): ${DI_HITS:-0}"
    if [ "${DI_HITS:-0}" -ge 1 ]; then
      echo "  → §0.6-D YES/PARTIAL (extend Lang-C7 emit hook)"
    else
      echo "  → §0.6-D NO/light (Lang-C7 may be N/A — confirm in spec)"
    fi
    ;;
  csharp|cs)
    SAMPLE_FILES=$(find . -type f -name "*.cs" \
      -not -path "*/bin/*" -not -path "*/obj/*" -not -path "*/.git/*" 2>/dev/null | head -200)
    DI_HITS=0
    if [ -n "$SAMPLE_FILES" ]; then
      DI_HITS=$(printf '%s\n' "$SAMPLE_FILES" | tr '\n' '\0' | \
        xargs -0 grep -lE "\[(Inject|Required|Bind)\]" 2>/dev/null | wc -l | xargs)
    fi
    echo "  Files with DI/annotation hits: ${DI_HITS:-0}"
    if [ "${DI_HITS:-0}" -ge 1 ]; then
      echo "  → §0.6-D YES"
    else
      echo "  → §0.6-D NO/light"
    fi
    ;;
  *)
    echo "  → §0.6-D — no probe implemented for $LANG; check manually"
    ;;
esac
set -e

# ──────────────────────────────────────────────────────────────────────
# Summary
# ──────────────────────────────────────────────────────────────────────
echo
echo "================================================================"
echo "Verdict: PASS=$PASS  FAIL=$FAIL  SOFT-FAIL=$SOFT_FAIL"
if [ "$FAIL" -eq 0 ] && [ "$SOFT_FAIL" -eq 0 ]; then
  echo "  → ELIGIBLE — proceed to §1.4 fixture-ranking on §1.2 soft criteria"
  echo "================================================================"
  exit 0
elif [ "$FAIL" -eq 0 ]; then
  echo "  → ELIGIBLE WITH CAVEATS — soft-fails noted; document expected yield"
  echo "================================================================"
  exit 2
else
  echo "  → INELIGIBLE — at least one hard requirement failed"
  echo "================================================================"
  exit 1
fi
