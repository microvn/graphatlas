//! LANG-1 regression suite (2026-05-22) — JSX element usage in `.tsx`
//! must surface as a CALL edge to the component identifier.
//!
//! Real-world evidence: cardshield round 3 — React function component
//! `PlanBanner` (defined apps/admin/src/components/PlanBanner.tsx:14) used
//! via `<PlanBanner />` in Layout.tsx — `ga_callers PlanBanner` returned 0.
//!
//! Background: docs/investigate/ga-vs-codegraph-head-to-head-2026-05-21.md
//! LANG-1.

use ga_core::Lang;
use ga_parser::extract_calls;

#[test]
fn jsx_self_closing_element_counts_as_call() {
    // Regression: LANG-1 — `<PlanBanner />` must produce a CALL with
    // callee_name=PlanBanner. Pre-fix the indexer loaded LANGUAGE_TYPESCRIPT
    // which has no JSX node kinds → empty CALLs.
    let src = b"\
import PlanBanner from './PlanBanner';

export function Layout() {
    return <PlanBanner />;
}
";
    let calls = extract_calls(Lang::TypeScript, src).expect("extract_calls Ok");
    assert!(
        calls.iter().any(|c| c.callee_name == "PlanBanner"),
        "expected PlanBanner call from JSX self-closing: {calls:?}"
    );
}

#[test]
fn jsx_opening_element_with_children_counts_as_call() {
    let src = b"\
import { Card } from './Card';

export function Page() {
    return <Card><span>hi</span></Card>;
}
";
    let calls = extract_calls(Lang::TypeScript, src).expect("extract_calls Ok");
    assert!(
        calls.iter().any(|c| c.callee_name == "Card"),
        "expected Card call from JSX opening element: {calls:?}"
    );
}

#[test]
fn jsx_lowercase_html_tag_does_not_emit_call() {
    // `<div>`, `<span>` etc. are intrinsic HTML — must NOT become CALL edges
    // to fictional `div`/`span` symbols.
    let src = b"\
export function Hello() {
    return <div><span>hi</span></div>;
}
";
    let calls = extract_calls(Lang::TypeScript, src).expect("extract_calls Ok");
    let bad: Vec<_> = calls
        .iter()
        .filter(|c| matches!(c.callee_name.as_str(), "div" | "span"))
        .collect();
    assert!(
        bad.is_empty(),
        "intrinsic HTML tags must not be calls: {bad:?}"
    );
}

#[test]
fn plain_typescript_call_still_works_after_grammar_swap() {
    // After swapping LANGUAGE_TYPESCRIPT → LANGUAGE_TSX, regular .ts
    // call_expression must continue to parse. Sanity check.
    let src = b"\
function add(a: number, b: number): number {
    return a + b;
}

function caller(): number {
    return add(1, 2);
}
";
    let calls = extract_calls(Lang::TypeScript, src).expect("extract_calls Ok");
    assert!(
        calls.iter().any(|c| c.callee_name == "add"),
        "plain TS call must still parse: {calls:?}"
    );
}
