//! Helpers shared across query modules (`callers`, `callees`, …).

use ga_core::{Error, Result};

pub(crate) fn is_safe_ident(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '$' | '.'))
}

/// `true` when `path` matches a per-language test-file convention.
///
/// Shared by `impact::affected_tests` (convention-match signal) and the
/// indexer's TESTED_BY edge derivation (KG-1). Conventions covered:
/// - Python: `test_*.py`, `*_test.py`, any segment in `{tests, test, __tests__}`
/// - Go: `*_test.go`
/// - Rust: `*_test.rs` suffix or any segment in `{tests, test}`
/// - TS/JS family: `*.test.{ts,tsx,js,jsx,mjs,cjs}`, `*.spec.*`, `__tests__/`
/// - Java (v1.1-M4): `*Test.java`, `*Tests.java`, `*Spec.java`, `*IT.java`, `src/test/`
/// - Kotlin (v1.1-M4): `*Test.kt`, `*Tests.kt`, `*Spec.kt`
/// - C# (v1.1-M4): `*Test.cs`, `*Tests.cs`
/// - Ruby (v1.1-M4): `*_spec.rb`, `*_test.rb`, `spec/`
///
/// Single source of truth — both the query-time convention matcher and the
/// index-time TESTED_BY filter MUST agree on what "test path" means.
/// Canonical test-path detection. Single source of truth per
/// `docs/guide/dataset-for-new-language.md` §4.2.6 (medium-term refactor
/// landed S-002-bench, replacing 8 duplicated copies across `ga-bench`).
///
/// All bench retrievers, GT-generation rules (h1/h2/h4), and `m2_runner`
/// import THIS function. Drift between sites is now impossible by
/// construction — bias-asymmetry per §4.2.5 cannot recur unless this
/// canonical is itself wrong.
///
/// Coverage:
/// - Suffix-based: Python `_test.py` / `test_*.py`, Go `_test.go`,
///   Rust `_test.rs`, JS/TS `*.test.*` / `*.spec.*`, Java
///   `*Test.java` / `*Tests.java` / `*Spec.java` / `*IT.java`,
///   Kotlin `*Test.kt` / `*Tests.kt` / `*Spec.kt` / `*IT.kt`,
///   C# `*Test.cs` / `*Tests.cs`, Ruby `*_spec.rb` / `*_test.rb`.
/// - Path-segment: `tests` / `test` / `__tests__` / `spec` / `specs`.
/// - KMP multi-target: `<setName>Test/kotlin/` (commonTest / jvmTest /
///   androidTest / androidUnitTest / iosTest / nativeTest / jsTest /
///   wasmTest / linuxX64Test / mingwX64Test ...) — ends-with-Test
///   segment immediately preceding `kotlin` segment.
/// - Maven explicit `src/test/`.
pub fn is_test_path(path: &str) -> bool {
    let Some(name) = path.rsplit('/').next() else {
        return false;
    };
    let segments: Vec<&str> = path.split('/').collect();
    let in_tests_dir = segments
        .iter()
        .any(|seg| matches!(*seg, "tests" | "test" | "__tests__" | "spec" | "specs"));
    // Java/Kotlin Maven-style `src/test/<lang>/...` layout — segments contain
    // `test` already covered above, but also recognise the explicit pair.
    let in_src_test = path.contains("/src/test/") || path.starts_with("src/test/");

    // Kotlin Multiplatform multi-target: `<setName>Test/kotlin/` segment
    // pair (commonTest, jvmTest, androidTest, androidUnitTest, iosTest,
    // nativeTest, jsTest, wasmTest, linuxX64Test, mingwX64Test, ...).
    // Detection: any segment ending with "Test" (length > 4 to exclude
    // bare "Test") immediately followed by a "kotlin" segment.
    let in_kmp_test_set = segments
        .windows(2)
        .any(|w| w[0].len() > 4 && w[0].ends_with("Test") && w[1] == "kotlin");

    if let Some(stem) = name.strip_suffix(".py") {
        if stem.starts_with("test_") || stem.ends_with("_test") {
            return true;
        }
        if in_tests_dir {
            return true;
        }
    }

    if let Some(stem) = name.strip_suffix("_test.go") {
        if !stem.is_empty() {
            return true;
        }
    }

    if let Some(stem) = name.strip_suffix(".rs") {
        if stem.ends_with("_test") {
            return true;
        }
        if in_tests_dir {
            return true;
        }
    }

    for ext in [".ts", ".tsx", ".js", ".jsx", ".mjs", ".cjs"] {
        if let Some(stem) = name.strip_suffix(ext) {
            if stem.ends_with(".test") || stem.ends_with(".spec") {
                return true;
            }
            if in_tests_dir {
                return true;
            }
        }
    }

    // v1.1-M4 — Java / Kotlin / C# suffix-based + Maven src/test/ layout.
    if let Some(stem) = name.strip_suffix(".java") {
        if stem.ends_with("Test")
            || stem.ends_with("Tests")
            || stem.ends_with("Spec")
            || stem.ends_with("IT")
        {
            return true;
        }
        if in_src_test || in_tests_dir {
            return true;
        }
    }

    if let Some(stem) = name.strip_suffix(".kt") {
        if stem.ends_with("Test")
            || stem.ends_with("Tests")
            || stem.ends_with("Spec")
            || stem.ends_with("IT")
        {
            return true;
        }
        if in_src_test || in_tests_dir || in_kmp_test_set {
            return true;
        }
    }

    if let Some(stem) = name.strip_suffix(".cs") {
        if stem.ends_with("Test") || stem.ends_with("Tests") {
            return true;
        }
        if in_tests_dir {
            return true;
        }
    }

    if let Some(stem) = name.strip_suffix(".rb") {
        if stem.ends_with("_spec") || stem.ends_with("_test") {
            return true;
        }
        if in_tests_dir {
            return true;
        }
    }

    // v1.2 — PHP / PHPUnit suffix `*Test.php` / `*Tests.php` + tests/ dir.
    if let Some(stem) = name.strip_suffix(".php") {
        if stem.ends_with("Test") || stem.ends_with("Tests") {
            return true;
        }
        if in_tests_dir {
            return true;
        }
    }

    false
}

pub(crate) fn count_defs(conn: &lbug::Connection<'_>, name: &str) -> Result<i64> {
    // Exclude synthetic external Symbol nodes from the polymorphic-confidence
    // calculation — they aren't real source defs, so they must not inflate
    // def_count and flip confidence to 0.6 under the Tools-C11 rule.
    let rs = conn
        .query(&format!(
            "MATCH (s:Symbol) WHERE s.name = '{}' AND s.kind <> 'external' \
             RETURN COUNT(DISTINCT s.file)",
            name
        ))
        .map_err(|e| Error::Other(anyhow::anyhow!("def-count query: {e}")))?;
    let mut n = 0i64;
    for row in rs {
        if let Some(lbug::Value::Int64(v)) = row.into_iter().next() {
            n = v;
        }
    }
    Ok(n)
}

pub(crate) fn symbol_exists(conn: &lbug::Connection<'_>, name: &str) -> Result<bool> {
    let rs = conn
        .query(&format!(
            "MATCH (s:Symbol) WHERE s.name = '{}' RETURN count(s)",
            name
        ))
        .map_err(|e| Error::Other(anyhow::anyhow!("symbol-exists query: {e}")))?;
    for row in rs {
        if let Some(lbug::Value::Int64(n)) = row.into_iter().next() {
            return Ok(n > 0);
        }
    }
    Ok(false)
}

/// Top-3 Levenshtein-nearest symbol names from the graph. Full scan is fine
/// at M1 scale (≤50k symbols per AS-008); FTS-backed suggestion lands with
/// S-004 ga_symbols.
pub(crate) fn suggest_similar(conn: &lbug::Connection<'_>, query: &str) -> Vec<String> {
    let Ok(rs) = conn.query("MATCH (s:Symbol) RETURN DISTINCT s.name") else {
        return Vec::new();
    };
    let mut scored: Vec<(u32, String)> = Vec::new();
    for row in rs {
        if let Some(lbug::Value::String(name)) = row.into_iter().next() {
            let d = levenshtein(query, &name);
            scored.push((d, name));
        }
    }
    scored.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    scored.into_iter().take(3).map(|(_, n)| n).collect()
}

/// Classic DP Levenshtein. u32 result — symbol names cap at 512 chars (Tools-C8).
pub(crate) fn levenshtein(a: &str, b: &str) -> u32 {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let n = a.len();
    let m = b.len();
    if n == 0 {
        return m as u32;
    }
    if m == 0 {
        return n as u32;
    }
    let mut prev: Vec<u32> = (0..=m as u32).collect();
    let mut curr: Vec<u32> = vec![0; m + 1];
    for i in 1..=n {
        curr[0] = i as u32;
        for j in 1..=m {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            curr[j] = (curr[j - 1] + 1).min(prev[j] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[m]
}

#[cfg(test)]
mod tests {
    use super::is_test_path;
    use ga_core::Lang;

    /// Per-lang test fixtures for `is_test_path`. The `match lang` body
    /// is **exhaustive**: adding a new variant to `Lang` fails compile
    /// here until the maintainer adds an arm — making it impossible to
    /// ship a new language without declaring its test/production path
    /// conventions for both the indexer (KG-1 TESTED_BY filter) and
    /// the impact::affected_tests convention scan.
    ///
    /// Returns `(positives, negatives)`:
    /// - `positives`: paths the new lang considers a test → must
    ///   classify `is_test_path == true`.
    /// - `negatives`: production paths → must classify `false`.
    ///
    /// If a new lang has no convention support yet (e.g., variant
    /// shipped but the function arms not added), pass empty slices —
    /// the comprehensive test below still iterates the variant via
    /// `Lang::ALL`, so the gap is logged rather than silently missed.
    fn paths_for(lang: Lang) -> (&'static [&'static str], &'static [&'static str]) {
        match lang {
            Lang::Python => (
                &[
                    "tests/test_models.py",
                    "test_foo.py",
                    "auth_test.py",
                    "pkg/__tests__/foo.py",
                ],
                &["src/models.py", "auth.py"],
            ),
            Lang::TypeScript => (
                &[
                    "src/foo.test.ts",
                    "src/foo.spec.ts",
                    "__tests__/users.ts",
                    "src/foo.test.tsx",
                ],
                &["src/foo.ts", "src/users.tsx"],
            ),
            Lang::JavaScript => (
                &["src/foo.test.js", "src/foo.spec.js", "__tests__/users.js"],
                &["src/foo.js", "src/users.mjs"],
            ),
            Lang::Go => (
                &["foo_test.go", "pkg/svc_test.go"],
                &["foo.go", "pkg/svc.go"],
            ),
            Lang::Rust => (
                &["tests/foo.rs", "tests/integration.rs", "pkg_test.rs"],
                &["src/lib.rs", "src/main.rs"],
            ),
            Lang::Java => (
                &[
                    // Suffix conventions.
                    "src/test/java/foo/UserRepositoryTest.java",
                    "src/test/java/foo/UserRepositoryTests.java",
                    "src/test/java/foo/UserRepositorySpec.java",
                    "src/test/java/foo/UserRepositoryIT.java",
                    // Maven `src/test/` layout — fixtures / helpers
                    // without a Test suffix still count as test.
                    "src/test/java/foo/Helper.java",
                    "mockito-core/src/test/java/foo/Fixture.java",
                ],
                &[
                    "src/main/java/foo/UserRepository.java",
                    "mockito-core/src/main/java/org/mockito/Mockito.java",
                ],
            ),
            Lang::Kotlin => (
                &[
                    "src/test/kotlin/FooTest.kt",
                    "src/test/kotlin/FooTests.kt",
                    "src/test/kotlin/FooSpec.kt",
                ],
                &["src/main/kotlin/Foo.kt"],
            ),
            Lang::CSharp => (
                &["src/Foo.Tests/UserTest.cs", "tests/UserTests.cs"],
                &["src/Foo/User.cs"],
            ),
            Lang::Ruby => (
                &[
                    "spec/foo_spec.rb",
                    "test/foo_test.rb",
                    "spec/models/user_spec.rb",
                ],
                &["lib/foo.rb", "app/models/user.rb"],
            ),
            // v1.2 — PHPUnit + PSR-4 convention. Pairs with `is_test_path`
            // PHP arm above so the exhaustive-match compile-time guard holds.
            Lang::Php => (
                &[
                    "tests/FooTest.php",
                    "tests/Unit/UserTest.php",
                    "tests/Feature/UserTest.php",
                ],
                &["src/Foo.php", "app/Models/User.php"],
            ),
        }
    }

    /// One-shot comprehensive coverage for `is_test_path`. Iterates
    /// `ga_core::Lang::ALL` so adding a new variant + updating ALL is
    /// enough — no need to add a per-lang `#[test]`. The exhaustive
    /// match in `paths_for` is the second compile-time guard.
    #[test]
    fn classifies_every_shipped_lang() {
        let mut failures: Vec<String> = Vec::new();
        for &lang in Lang::ALL {
            let (positives, negatives) = paths_for(lang);
            for &p in positives {
                if !is_test_path(p) {
                    failures.push(format!("[{lang:?}] expected TEST classification: {p}"));
                }
            }
            for &p in negatives {
                if is_test_path(p) {
                    failures.push(format!(
                        "[{lang:?}] expected PRODUCTION classification: {p}"
                    ));
                }
            }
        }
        assert!(
            failures.is_empty(),
            "{} is_test_path classification failure(s):\n{}",
            failures.len(),
            failures.join("\n")
        );
    }
}
