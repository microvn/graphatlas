//! P1.5 / N1 (2026-05-22) — compact Markdown renderers for ga_callers /
//! ga_callees / ga_impact / ga_symbols.
//!
//! Same data the JSON path emits, but a ~2× cheaper token shape: bullet
//! list instead of JSON envelope with repeating key names. Opt-in via the
//! tool's `format: "markdown"` argument; default stays JSON for backward
//! compat with the 14 existing test files that assert on parsed payloads.
//!
//! Background: docs/investigate/ga-vs-codegraph-head-to-head-2026-05-21.md
//! — same Markdown shape codegraph emits by default (~50% of GA JSON cost).

use ga_query::{
    AffectedConfig, AffectedRoute, AffectedTest, BreakPoint, CalleeEntry, CallerEntry,
    Disambiguation, ImpactedFile, SymbolEntry,
};

/// Decide which output format the caller requested. `args["format"]` may be
/// `"markdown"` or `"json"` (or absent — defaults to JSON for backward compat).
/// Any unknown value silently falls back to JSON.
pub(crate) fn wants_markdown(args: &serde_json::Value) -> bool {
    args.get("format")
        .and_then(|v| v.as_str())
        .map(|s| s.eq_ignore_ascii_case("markdown") || s.eq_ignore_ascii_case("md"))
        .unwrap_or(false)
}

fn push_disambiguation(out: &mut String, dis: &Disambiguation) {
    out.push_str("\n> **");
    out.push_str(&dis.reason.to_ascii_uppercase());
    out.push_str("** — ");
    out.push_str(&dis.hint);
    out.push('\n');
    for c in &dis.candidates {
        out.push_str("> - ");
        out.push_str(&c.qualified_name);
        out.push_str(" (line ");
        out.push_str(&c.line.to_string());
        out.push_str(", ");
        out.push_str(&c.kind);
        out.push_str(")\n");
    }
}

pub(crate) fn render_callers(
    symbol: &str,
    callers: &[CallerEntry],
    disambiguation: Option<&Disambiguation>,
) -> String {
    if let Some(dis) = disambiguation {
        let mut s = format!("## Callers of {symbol} (ambiguous)\n");
        push_disambiguation(&mut s, dis);
        return s;
    }
    let mut s = format!("## Callers of {symbol} ({} found)\n", callers.len());
    for c in callers {
        // `- <symbol> (<kind>) - <file>:<call_site_line>`
        let kind = match c.kind {
            ga_query::CallKind::Call => "call",
            ga_query::CallKind::Reference => "reference",
        };
        s.push_str("- ");
        s.push_str(&c.symbol);
        s.push_str(" (");
        s.push_str(kind);
        s.push_str(") - ");
        s.push_str(&c.file);
        s.push(':');
        s.push_str(&c.call_site_line.to_string());
        // Low confidence — surface inline so the agent can drop / weight.
        if (c.confidence - 1.0).abs() > 1e-6 {
            s.push_str(" [conf=");
            s.push_str(&format!("{:.1}", c.confidence));
            s.push(']');
        }
        s.push('\n');
    }
    s
}

pub(crate) fn render_callees(
    symbol: &str,
    callees: &[CalleeEntry],
    disambiguation: Option<&Disambiguation>,
) -> String {
    if let Some(dis) = disambiguation {
        let mut s = format!("## Callees of {symbol} (ambiguous)\n");
        push_disambiguation(&mut s, dis);
        return s;
    }
    let mut s = format!("## Callees of {symbol} ({} found)\n", callees.len());
    for c in callees {
        s.push_str("- ");
        s.push_str(&c.symbol);
        s.push_str(" (");
        s.push_str(&c.symbol_kind);
        if c.external {
            s.push_str(", external");
        }
        s.push_str(") - ");
        s.push_str(&c.file);
        s.push(':');
        s.push_str(&c.call_site_line.to_string());
        s.push('\n');
    }
    s
}

pub(crate) fn render_impact(
    seed_label: &str,
    impacted_files: &[ImpactedFile],
    affected_tests: &[AffectedTest],
    affected_routes: &[AffectedRoute],
    affected_configs: &[AffectedConfig],
    break_points: &[BreakPoint],
    disambiguation: Option<&Disambiguation>,
) -> String {
    if let Some(dis) = disambiguation {
        let mut s = format!("## Impact: {seed_label} (ambiguous)\n");
        push_disambiguation(&mut s, dis);
        return s;
    }
    let mut s = format!("## Impact: {seed_label}\n");
    if !break_points.is_empty() {
        s.push_str(&format!("\n### Break points ({})\n", break_points.len()));
        for bp in break_points {
            // `- <file>:<line> via <caller_symbols>`
            s.push_str("- ");
            s.push_str(&bp.file);
            s.push(':');
            s.push_str(&bp.line.to_string());
            if !bp.caller_symbols.is_empty() {
                s.push_str(" via ");
                s.push_str(&bp.caller_symbols.join(", "));
            }
            s.push('\n');
        }
    }
    if !impacted_files.is_empty() {
        s.push_str(&format!(
            "\n### Impacted files ({})\n",
            impacted_files.len()
        ));
        for f in impacted_files {
            s.push_str("- ");
            s.push_str(&f.path);
            s.push_str(" (depth ");
            s.push_str(&f.depth.to_string());
            s.push_str(", ");
            s.push_str(format!("{:?}", f.reason).to_lowercase().as_str());
            s.push(')');
            if (f.confidence - 1.0).abs() > 1e-6 {
                s.push_str(" [conf=");
                s.push_str(&format!("{:.1}", f.confidence));
                s.push(']');
            }
            s.push('\n');
        }
    }
    if !affected_tests.is_empty() {
        s.push_str(&format!(
            "\n### Affected tests ({})\n",
            affected_tests.len()
        ));
        for t in affected_tests {
            s.push_str("- ");
            s.push_str(&t.path);
            s.push_str(" (");
            s.push_str(format!("{:?}", t.reason).to_lowercase().as_str());
            s.push_str(")\n");
        }
    }
    if !affected_routes.is_empty() {
        s.push_str(&format!(
            "\n### Affected routes ({})\n",
            affected_routes.len()
        ));
        for r in affected_routes {
            s.push_str("- ");
            if !r.method.is_empty() {
                s.push_str(&r.method);
                s.push(' ');
            }
            s.push_str(&r.path);
            if !r.source_file.is_empty() {
                s.push_str(" (");
                s.push_str(&r.source_file);
                s.push(')');
            }
            s.push('\n');
        }
    }
    if !affected_configs.is_empty() {
        s.push_str(&format!(
            "\n### Affected configs ({})\n",
            affected_configs.len()
        ));
        for c in affected_configs {
            s.push_str("- ");
            s.push_str(&c.path);
            s.push(':');
            s.push_str(&c.line.to_string());
            s.push('\n');
        }
    }
    s
}

pub(crate) fn render_symbols(pattern: &str, symbols: &[SymbolEntry]) -> String {
    let mut s = format!(
        "## Search results for \"{pattern}\" ({} found)\n",
        symbols.len()
    );
    for sym in symbols {
        s.push_str("- ");
        s.push_str(&sym.name);
        s.push_str(" (");
        s.push_str(&sym.kind);
        s.push_str(") - ");
        s.push_str(&sym.file);
        s.push(':');
        s.push_str(&sym.line.to_string());
        s.push('\n');
    }
    s
}
