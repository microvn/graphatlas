//! Story B — Rust-specific seed extraction fixes.
//!
//! Driver: M3 minimal_context audit on tokio (3/10 tasks "symbol not
//! found" because seed_symbol was a Rust idiom artifact, not a real
//! symbol). See /mf-voices Round 4 consensus.
//!
//! 3 patterns the regex picker handles wrong:
//!
//! (1) `impl <Trait> for <Target>` → picker captures `<Trait>` (e.g.
//!     `Default`) instead of `<Target>` (the type the impl extends).
//!     Real example: tokio-96f64f4e (sym=`Default` from
//!     `impl Default for Framed`).
//!
//! (2) `macro_rules! <name> { ... }` → first non-stopword identifier
//!     is `<name>`, but engine doesn't index macro_rules definitions
//!     as Symbol nodes. Real example: tokio-a0d5b8ab (sym=`cfg_rt`).
//!
//! (3) Lines starting with `#[<attr>(...)]` → first identifier is
//!     `<attr>` (e.g. `doc`, `cfg`, `derive`, `inline`). Not a real
//!     symbol — these are attributes that decorate items. Real
//!     example: tokio-eb99e476 (sym=`doc`).

import { describe, expect, test } from "bun:test";
import { extractSymbol } from "./extract-seeds";

describe("Story B — `impl Trait for Target` picks Target", () => {
  test("impl Default for Framed → Framed", () => {
    expect(
      extractSymbol("impl Default for Framed<T, U> {", "rust"),
    ).toBe("Framed");
  });

  test("impl<T> Display for MyType<T> → MyType", () => {
    expect(
      extractSymbol("impl<T> Display for MyType<T> {", "rust"),
    ).toBe("MyType");
  });

  test("impl Future for Notified<'a> → Notified", () => {
    expect(extractSymbol("impl Future for Notified<'a> {", "rust")).toBe(
      "Notified",
    );
  });

  test("regression: plain `impl Foo` still picks Foo (no `for` clause)", () => {
    expect(extractSymbol("impl Foo {", "rust")).toBe("Foo");
    expect(extractSymbol("impl<T> Foo<T> {", "rust")).toBe("Foo");
  });
});

describe("Story B — `macro_rules!` returns null", () => {
  test("macro_rules! cfg_rt → null (engine doesn't index macros)", () => {
    expect(extractSymbol("macro_rules! cfg_rt {", "rust")).toBe(null);
  });

  test("macro_rules! my_macro { ($x:expr) => { ... } } → null", () => {
    expect(
      extractSymbol("macro_rules! my_macro { ($x:expr) => { ... } }", "rust"),
    ).toBe(null);
  });
});

describe("Story B — `#[attr(...)]` lines skip the attribute name", () => {
  test("#[doc = \"...\"] does not pick `doc`", () => {
    expect(extractSymbol("#[doc = \"some doc\"]", "rust")).not.toBe("doc");
  });

  test("#[cfg(feature = \"x\")] does not pick `cfg`", () => {
    expect(extractSymbol("#[cfg(feature = \"x\")]", "rust")).not.toBe("cfg");
  });

  test("#[derive(Debug, Clone)] does not pick `derive`", () => {
    expect(extractSymbol("#[derive(Debug, Clone)]", "rust")).not.toBe("derive");
  });

  test("#[inline] does not pick `inline`", () => {
    expect(extractSymbol("#[inline]", "rust")).not.toBe("inline");
  });

  test("attribute followed by `pub fn foo` picks `foo`", () => {
    // Hunk context line that includes an attribute then the next item:
    // `#[inline] pub fn calculate(x: i32) -> i32 {`
    expect(
      extractSymbol("#[inline] pub fn calculate(x: i32) -> i32 {", "rust"),
    ).toBe("calculate");
  });
});

describe("Story B — regression: real Rust seeds still work", () => {
  test("plain fn", () => {
    expect(extractSymbol("pub fn parse(s: &str) -> Result<()> {", "rust")).toBe(
      "parse",
    );
  });

  test("struct definition", () => {
    expect(extractSymbol("pub struct CancellationToken {", "rust")).toBe(
      "CancellationToken",
    );
  });

  test("trait definition", () => {
    expect(extractSymbol("pub trait FromRequest {", "rust")).toBe(
      "FromRequest",
    );
  });

  test("enum definition", () => {
    expect(extractSymbol("pub enum Color { Red, Green }", "rust")).toBe(
      "Color",
    );
  });
});
