//! Render helpers for UI consumption — Spec A S-003 AS-028 / AS-029.
//!
//! The lbug Symbol NODE table stores `qualified_name`, `return_type`,
//! and `params STRUCT(name, type, default_value)[]` separately. The UI
//! detail panel needs ONE rendered string (`Router::new(self: &mut Self,
//! path: &str) -> Router`). We render at query time rather than store a
//! denormalized `signature` column, so:
//!   1. Render rule changes don't need a schema migration.
//!   2. We never drift between stored signature and source params.
//!   3. Lbug schema stays unchanged (Spec A AS-031 invariant).

/// A param as stored in lbug — name + type + default (any may be empty).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParamSlot {
    pub name: String,
    pub type_: String,
    pub default_value: String,
}

impl ParamSlot {
    pub fn new(name: &str, type_: &str) -> Self {
        Self {
            name: name.into(),
            type_: type_.into(),
            default_value: String::new(),
        }
    }

    pub fn with_default(name: &str, type_: &str, default_value: &str) -> Self {
        Self {
            name: name.into(),
            type_: type_.into(),
            default_value: default_value.into(),
        }
    }
}

/// Render a function-like signature for the UI detail panel.
///
/// Format rules:
/// - Each param: `name: type` (no type → just `name`; no name → just `type`)
/// - Default value → ` = <default>` suffix on the param
/// - Empty params → `name()` (still emit the parens — it's a function)
/// - Return type: empty/whitespace → omit `-> X` entirely (AS-029)
/// - Whitespace in inputs is trimmed; never two consecutive spaces in output
///
/// The helper is intentionally allocation-light — single `String` with
/// pre-estimated capacity. Detail-panel callers may render hundreds per
/// page (file_summary endpoint), so we keep the per-call cost flat.
pub fn render_signature(name: &str, return_type: &str, params: &[ParamSlot]) -> String {
    let name = name.trim();
    let return_type = return_type.trim();
    let cap = name.len()
        + 4
        + params
            .iter()
            .map(|p| p.name.len() + p.type_.len() + p.default_value.len() + 5)
            .sum::<usize>()
        + return_type.len()
        + 4;
    let mut out = String::with_capacity(cap);
    out.push_str(name);
    out.push('(');
    for (i, p) in params.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        let pname = p.name.trim();
        let ptype = p.type_.trim();
        match (pname.is_empty(), ptype.is_empty()) {
            (true, true) => { /* nothing — skip silently */ }
            (false, true) => out.push_str(pname),
            (true, false) => out.push_str(ptype),
            (false, false) => {
                out.push_str(pname);
                out.push_str(": ");
                out.push_str(ptype);
            }
        }
        let dflt = p.default_value.trim();
        if !dflt.is_empty() {
            out.push_str(" = ");
            out.push_str(dflt);
        }
    }
    out.push(')');
    if !return_type.is_empty() {
        out.push_str(" -> ");
        out.push_str(return_type);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------- AS-028 happy path ----------

    #[test]
    fn as028_named_params_with_return_type() {
        let sig = render_signature(
            "new",
            "Router",
            &[
                ParamSlot::new("self", "&mut Self"),
                ParamSlot::new("path", "&str"),
            ],
        );
        assert_eq!(sig, "new(self: &mut Self, path: &str) -> Router");
    }

    #[test]
    fn as028_qualified_name() {
        let sig = render_signature("Router::new", "Self", &[ParamSlot::new("path", "&str")]);
        assert_eq!(sig, "Router::new(path: &str) -> Self");
    }

    // ---------- AS-029 return_type default ----------

    #[test]
    fn as029_empty_return_type_omits_arrow() {
        let sig = render_signature("log", "", &[ParamSlot::new("msg", "&str")]);
        assert_eq!(sig, "log(msg: &str)");
    }

    #[test]
    fn as029_whitespace_return_type_omits_arrow() {
        let sig = render_signature("foo", "   ", &[]);
        assert_eq!(sig, "foo()");
    }

    // ---------- edge: zero params ----------

    #[test]
    fn empty_params_still_render_parens() {
        let sig = render_signature("now", "u64", &[]);
        assert_eq!(sig, "now() -> u64");
    }

    // ---------- edge: name-only param (no type) — Python style ----------

    #[test]
    fn name_only_param() {
        let sig = render_signature("greet", "", &[ParamSlot::new("name", "")]);
        assert_eq!(sig, "greet(name)");
    }

    // ---------- edge: type-only param (anonymous) — Rust style ----------

    #[test]
    fn type_only_param() {
        let sig = render_signature("consume", "", &[ParamSlot::new("", "Box<dyn Trait>")]);
        assert_eq!(sig, "consume(Box<dyn Trait>)");
    }

    // ---------- edge: default value ----------

    #[test]
    fn param_with_default() {
        let sig = render_signature(
            "build",
            "Server",
            &[
                ParamSlot::new("host", "&str"),
                ParamSlot::with_default("port", "u16", "8080"),
            ],
        );
        assert_eq!(sig, "build(host: &str, port: u16 = 8080) -> Server");
    }

    // ---------- edge: whitespace cleanup ----------

    #[test]
    fn whitespace_in_inputs_is_trimmed() {
        let sig = render_signature(
            "  trimmed  ",
            "  Result<()>  ",
            &[ParamSlot::new("  x  ", "  i32  ")],
        );
        assert_eq!(sig, "trimmed(x: i32) -> Result<()>");
    }

    // ---------- edge: fully empty param slot silently skipped ----------

    #[test]
    fn fully_empty_param_skipped_silently() {
        let sig = render_signature(
            "weird",
            "",
            &[
                ParamSlot::new("a", "i32"),
                ParamSlot::new("", ""),
                ParamSlot::new("b", "i32"),
            ],
        );
        // The empty slot still occupies its position with a comma — the
        // caller filtered nothing. This is intentional: empty slots are
        // a parser oddity we surface, not silently elide.
        assert_eq!(sig, "weird(a: i32, , b: i32)");
    }
}
