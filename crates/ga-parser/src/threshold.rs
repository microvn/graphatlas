//! Per-language parse-failure threshold accounting (AS-011 / AS-012).
//!
//! Caller calls `record_ok()` / `record_failure()` as each file is processed,
//! then `evaluate()` at end-of-batch. If failure rate > threshold → return
//! [`ThresholdOutcome::AbortBrokenGrammar`] with the AS-012 spec-literal
//! message shape.

use ga_core::Lang;

/// Default per-language parse-failure threshold (AS-011).
pub const DEFAULT_THRESHOLD: f32 = 0.30;

#[derive(Debug, Clone)]
pub struct LangStats {
    lang: Lang,
    ok: u32,
    failed: u32,
}

impl LangStats {
    pub fn new(lang: Lang) -> Self {
        Self {
            lang,
            ok: 0,
            failed: 0,
        }
    }

    pub fn record_ok(&mut self) {
        self.ok += 1;
    }

    pub fn record_failure(&mut self) {
        self.failed += 1;
    }

    pub fn total(&self) -> u32 {
        self.ok + self.failed
    }

    pub fn failure_rate(&self) -> f32 {
        let total = self.total();
        if total == 0 {
            return 0.0;
        }
        self.failed as f32 / total as f32
    }

    /// AS-011 boundary: failure rate <= threshold → Continue (inclusive).
    /// > threshold → AbortBrokenGrammar.
    pub fn evaluate(&self, threshold: f32) -> ThresholdOutcome {
        let rate = self.failure_rate();
        if rate <= threshold {
            ThresholdOutcome::Continue {
                lang: self.lang,
                failure_rate: rate,
                ok: self.ok,
                failed: self.failed,
            }
        } else {
            ThresholdOutcome::AbortBrokenGrammar {
                lang: self.lang,
                failure_rate: rate,
                threshold,
                message: format_abort_message(self.lang, rate, threshold),
            }
        }
    }
}

fn format_abort_message(lang: Lang, rate: f32, threshold: f32) -> String {
    // AS-012 literal: "TypeScript parsing failed: 40% files >30% threshold,
    // likely broken grammar".
    let lang_display = match lang {
        Lang::Python => "Python",
        Lang::TypeScript => "TypeScript",
        Lang::JavaScript => "JavaScript",
        Lang::Go => "Go",
        Lang::Rust => "Rust",
        Lang::Java => "Java",
        Lang::Kotlin => "Kotlin",
        Lang::CSharp => "C#",
        Lang::Ruby => "Ruby",
        Lang::Php => "PHP",
    };
    let pct = (rate * 100.0).round() as u32;
    let thresh_pct = (threshold * 100.0).round() as u32;
    format!(
        "{lang_display} parsing failed: {pct}% files >{thresh_pct}% threshold, \
         likely broken grammar"
    )
}

#[derive(Debug)]
pub enum ThresholdOutcome {
    Continue {
        lang: Lang,
        failure_rate: f32,
        ok: u32,
        failed: u32,
    },
    AbortBrokenGrammar {
        lang: Lang,
        failure_rate: f32,
        threshold: f32,
        message: String,
    },
}
