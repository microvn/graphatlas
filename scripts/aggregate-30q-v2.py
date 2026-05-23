#!/usr/bin/env python3
"""Aggregate per-fixture leaderboards into one cross-tool summary.

Reads `bench-results/<uc>-<fixture>-leaderboard.md` (already produced by
`cargo run -p graphatlas -- bench --uc ... --fixture ...`), parses the
markdown table, and emits a stratified summary across 6 fixtures × 3 UCs.

Output: `bench-results/30q-v2-summary.md` with per-tool average F1, recall,
precision, plus per-fixture breakdown for sanity-checking imbalance.
"""
import re
import sys
from pathlib import Path

FIXTURES = ["preact", "gin", "tokio", "django", "php-symfony-console", "kotlinx-coroutines"]
UCS = ["callers", "callees", "symbols"]
TOOLS = ["ga", "ripgrep", "codegraphcontext", "codebase-memory", "code-review-graph", "gitnexus"]

ROW_RE = re.compile(
    r"^\|\s*(?P<tool>[\w-]+)\s*\|\s*"
    r"(?P<f1>[\d.]+)\s*\|\s*"
    r"(?P<recall>[\d.]+)\s*\|\s*"
    r"(?P<precision>[\d.]+)\s*\|\s*"
    r"(?P<mrr>[\d.]+)\s*\|\s*"
    r"(?P<lat>\d+)\s*ms\s*\|\s*"
    r"(?P<pass>[\d.]+)\s*%\s*\|"
)


def parse_leaderboard(path: Path):
    """Return dict tool -> {f1, recall, precision, mrr, lat, pass}."""
    out = {}
    if not path.exists():
        return out
    for line in path.read_text().splitlines():
        m = ROW_RE.match(line)
        if not m:
            continue
        d = m.groupdict()
        out[d["tool"]] = {
            "f1": float(d["f1"]),
            "recall": float(d["recall"]),
            "precision": float(d["precision"]),
            "mrr": float(d["mrr"]),
            "lat": int(d["lat"]),
            "pass": float(d["pass"]),
        }
    return out


def main():
    root = Path(__file__).resolve().parent.parent
    results_dir = root / "bench-results"

    # Per (uc, fixture, tool) cell
    cells = {}
    for uc in UCS:
        for fix in FIXTURES:
            p = results_dir / f"{uc}-{fix}-leaderboard.md"
            cells[(uc, fix)] = parse_leaderboard(p)

    # Tool summary: average across all (uc, fixture) cells where tool returned data
    summary = {t: {"f1": [], "recall": [], "precision": [], "pass": []} for t in TOOLS}
    for (uc, fix), per_tool in cells.items():
        for tool, scores in per_tool.items():
            if tool not in summary:
                continue
            summary[tool]["f1"].append(scores["f1"])
            summary[tool]["recall"].append(scores["recall"])
            summary[tool]["precision"].append(scores["precision"])
            summary[tool]["pass"].append(scores["pass"])

    def avg(xs):
        return sum(xs) / len(xs) if xs else 0.0

    # Output
    lines = []
    lines.append("# 30-Query Cross-Tool Bench v2 — Summary")
    lines.append("")
    lines.append(f"**Fixtures**: {', '.join(FIXTURES)}")
    lines.append(f"**UCs**: {', '.join(UCS)}")
    lines.append(f"**Tools**: {', '.join(TOOLS)}")
    lines.append("")
    lines.append("## Cross-fixture average (per tool)")
    lines.append("")
    lines.append("| Tool | Avg F1 | Avg Recall | Avg Precision | Avg Pass % | Coverage |")
    lines.append("|------|------:|----------:|-------------:|----------:|----------|")
    for t in TOOLS:
        s = summary[t]
        cov = f"{len(s['f1'])}/{len(UCS) * len(FIXTURES)}"
        lines.append(
            f"| {t} | {avg(s['f1']):.3f} | {avg(s['recall']):.3f} | "
            f"{avg(s['precision']):.3f} | {avg(s['pass']):.1f}% | {cov} |"
        )
    lines.append("")

    # Per-fixture detail
    for uc in UCS:
        lines.append(f"## {uc.upper()} per fixture")
        lines.append("")
        header_cols = ["Fixture"] + TOOLS
        lines.append("| " + " | ".join(header_cols) + " |")
        lines.append("|" + "|".join(["---"] * len(header_cols)) + "|")
        for fix in FIXTURES:
            row = [fix]
            for t in TOOLS:
                d = cells.get((uc, fix), {}).get(t)
                if d:
                    row.append(f"{d['f1']:.2f}")
                else:
                    row.append("—")
            lines.append("| " + " | ".join(row) + " |")
        lines.append("")

    # Coverage gap log — fixture/uc cells where tool returned 0 score
    lines.append("## Coverage gaps (tool × fixture × uc with F1=0)")
    lines.append("")
    lines.append("| Tool | Fixture | UC | Note |")
    lines.append("|------|---------|----|----|")
    for t in TOOLS:
        for uc in UCS:
            for fix in FIXTURES:
                d = cells.get((uc, fix), {}).get(t)
                if d is None:
                    lines.append(f"| {t} | {fix} | {uc} | leaderboard missing |")
                elif d["f1"] == 0.0:
                    lines.append(f"| {t} | {fix} | {uc} | F1=0 (not indexed or unsupported) |")
    lines.append("")
    lines.append(f"**Generated**: aggregator script `{__file__.rsplit('/', 1)[-1]}` over {results_dir}/")

    out_path = results_dir / "30q-v2-summary.md"
    out_path.write_text("\n".join(lines))
    print(f"Wrote {out_path} ({len(lines)} lines)")


if __name__ == "__main__":
    sys.exit(main())
