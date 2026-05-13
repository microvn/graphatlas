//! S-003-bench follow-up — extract-seeds.ts mining bug regression tests.
//!
//! Bug (verified by Codex 2026-04-26): MQTTnet had 13/19 tasks with
//! `seed_symbol = "MQTTnet"` (the namespace name itself), trivially
//! winnable by ripgrep, unwinnable by graph traversal (no Symbol node
//! named "MQTTnet"). Three root causes in scripts/extract-seeds.ts:
//!
//!   (A) NON_FN_HUNK_PREFIX (line 227) does NOT skip C# `namespace` /
//!       `using`. Java `package` IS skipped — C# regression.
//!   (B) extractAllHunkContexts (line 277) only reads `@@ ... @@ <ctx>`
//!       header tail. C# diff hunks often have header `namespace Foo` only;
//!       real method change lives in hunk BODY. Need to scan body too.
//!   (C) Java/C#/Kotlin method-sig regex (line 113) requires lowercase
//!       method first char — wrong for C# PascalCase (AcceptAsync,
//!       GetSocketOption).
//!
//! Plus pre-emptive Ruby fixes (S-004 lookahead per Codex insight #2):
//!   (D) NON_FN_HUNK_PREFIX += `require\s` / `require_relative\s` /
//!       `module\s` so Ruby module declarations don't pollute seeds.

import { describe, expect, test } from "bun:test";
import {
  extractSymbol,
  extractAllHunkContexts,
  isLikelyBadSymbol,
  NON_FN_HUNK_PREFIX,
} from "./extract-seeds";

describe("NON_FN_HUNK_PREFIX — namespace/using exclusion (Patch A)", () => {
  test("skips C# namespace declarations", () => {
    expect(NON_FN_HUNK_PREFIX.test("namespace MQTTnet.Implementations")).toBe(
      true,
    );
    expect(NON_FN_HUNK_PREFIX.test("namespace App.Models {")).toBe(true);
  });

  test("skips C# using directives", () => {
    expect(NON_FN_HUNK_PREFIX.test("using System.Threading.Tasks;")).toBe(true);
    expect(NON_FN_HUNK_PREFIX.test("using static System.Math;")).toBe(true);
    expect(NON_FN_HUNK_PREFIX.test("using F = System.Foo;")).toBe(true);
  });

  test("skips Ruby require / require_relative / module (Patch D — pre-emptive S-004)", () => {
    expect(NON_FN_HUNK_PREFIX.test("require 'json'")).toBe(true);
    expect(NON_FN_HUNK_PREFIX.test("require_relative '../foo'")).toBe(true);
    expect(NON_FN_HUNK_PREFIX.test("module Foo")).toBe(true);
  });

  test("preserves Java package skip (regression guard)", () => {
    expect(NON_FN_HUNK_PREFIX.test("package org.mockito")).toBe(true);
  });

  test("does NOT skip real method/class signatures", () => {
    expect(
      NON_FN_HUNK_PREFIX.test(
        "public async Task<CrossPlatformSocket> AcceptAsync(CancellationToken ct)",
      ),
    ).toBe(false);
    expect(NON_FN_HUNK_PREFIX.test("public class Foo")).toBe(false);
    expect(NON_FN_HUNK_PREFIX.test("def authenticate(password)")).toBe(false);
  });
});

describe("extractSymbol — PascalCase method support (Patch C)", () => {
  test("extracts C# PascalCase method name", () => {
    // Pre-fix: regex `[a-z_][\w]*` rejected `AcceptAsync` → fell back to
    // first non-stopword identifier → returned `Task` or `CrossPlatformSocket`.
    // Post-fix: `[A-Za-z_][\w]*` captures `AcceptAsync`.
    const ctx =
      "public async Task<CrossPlatformSocket> AcceptAsync(CancellationToken cancellationToken)";
    expect(extractSymbol(ctx)).toBe("AcceptAsync");
  });

  test("extracts C# void PascalCase method", () => {
    expect(extractSymbol("public void Process()")).toBe("Process");
  });

  test("extracts C# private async method", () => {
    expect(extractSymbol("private async Task DoWorkAsync(int x)")).toBe(
      "DoWorkAsync",
    );
  });

  test("preserves Java camelCase method extraction (regression guard)", () => {
    expect(
      extractSymbol("public boolean isAuthenticated(String token)"),
    ).toBe("isAuthenticated");
  });

  test("preserves class declaration extraction over method (class regex matches first)", () => {
    expect(extractSymbol("public class UserRepository")).toBe(
      "UserRepository",
    );
  });
});

describe("extractAllHunkContexts — body-line scanning (Patch B)", () => {
  test("extracts symbol from hunk BODY when header is only a namespace", () => {
    // Real-world MQTTnet pattern: hunk header says only `namespace MQTTnet.X`
    // because the diff context wraps to the outermost class scope. The
    // actual change is a method INSIDE the body lines (` ` / `+` / `-`
    // prefixed). Pre-fix returns ["namespace MQTTnet.X"] only; extractSymbol
    // picks "MQTTnet". Post-fix scans body lines + filters NON_FN_HUNK_PREFIX,
    // returning the actual method signature.
    const diff = `\
diff --git a/Source/MQTTnet/CrossPlatformSocket.cs b/Source/MQTTnet/CrossPlatformSocket.cs
index abc..def 100644
--- a/Source/MQTTnet/CrossPlatformSocket.cs
+++ b/Source/MQTTnet/CrossPlatformSocket.cs
@@ -10,5 +10,7 @@ namespace MQTTnet.Implementations
     public sealed class CrossPlatformSocket {
         readonly Socket _socket;
+
+        public async Task<CrossPlatformSocket> AcceptAsync(CancellationToken ct) {
+            var s = await _socket.AcceptAsync(ct);
+            return new CrossPlatformSocket(s);
+        }
     }
 }
`;
    const contexts = extractAllHunkContexts(diff);
    // Body should contribute method signature; header (namespace) should be
    // filtered by NON_FN_HUNK_PREFIX.
    const hasMethodSig = contexts.some((c) =>
      c.includes("AcceptAsync"),
    );
    expect(hasMethodSig).toBe(true);
    // Negative: namespace header should NOT appear (it matches NON_FN_HUNK_PREFIX).
    const hasNamespace = contexts.some((c) =>
      c.startsWith("namespace MQTTnet"),
    );
    expect(hasNamespace).toBe(false);
  });

  test("preserves header-only extraction when body has no signature (existing behavior)", () => {
    // If hunk body has no method/class signature (just data changes), the
    // header context should still be returned (when it's a real signature).
    const diff = `\
diff --git a/foo.java b/foo.java
@@ -5,3 +5,3 @@ public class Foo {
     int x = 1;
-    int y = 2;
+    int y = 3;
 }
`;
    const contexts = extractAllHunkContexts(diff);
    expect(contexts.some((c) => c.includes("class Foo"))).toBe(true);
  });

  test("body lines starting with namespace/using are filtered (Patch A+B together)", () => {
    // Edge case: if a body line happens to be `namespace Foo` (rare — only
    // at file start), it must NOT become a candidate context.
    const diff = `\
diff --git a/x.cs b/x.cs
@@ -1,5 +1,7 @@
+namespace App.NewModule
+{
+    public void Helper() { }
+}
 namespace App.OldModule { }
`;
    const contexts = extractAllHunkContexts(diff);
    // Method signature `public void Helper()` should appear from body.
    expect(contexts.some((c) => c.includes("Helper"))).toBe(true);
    // Namespace lines must NOT appear.
    expect(contexts.some((c) => c.startsWith("namespace"))).toBe(false);
  });
});

describe("isLikelyBadSymbol — lang-aware filter (Step A: Codex insight #1)", () => {
  // Real bad seeds observed in MQTTnet-tasks.json after Patches A+B+C:
  //   _options, _port, _tcpOptions     → C# private fields
  //   Substring, FromResult            → .NET stdlib methods
  //   ArgumentException, InvalidOperationException → .NET stdlib exceptions
  // None of these are graph-traversable concepts being TESTED — they're
  // implementation noise (private storage) or stdlib refs (every C# file
  // calls them). Filter them out as bad seeds at extraction time.

  describe("C# private-field convention (^_[a-z])", () => {
    test("rejects C# _options private field", () => {
      expect(isLikelyBadSymbol("_options", "csharp")).toBe(true);
    });
    test("rejects C# _tcpOptions private field", () => {
      expect(isLikelyBadSymbol("_tcpOptions", "csharp")).toBe(true);
    });
    test("rejects C# _port private field", () => {
      expect(isLikelyBadSymbol("_port", "csharp")).toBe(true);
    });
    test("preserves C# _Foo (uppercase after underscore — not the field convention)", () => {
      // Rare but legitimate (e.g. underscore-prefixed nested type).
      expect(isLikelyBadSymbol("_Helper", "csharp")).toBe(false);
    });
    test("preserves JS _foo (legitimate convention-private function/symbol)", () => {
      // JS/TS use leading underscore as a soft "internal" marker but the
      // symbol itself is still a real function/class users reference.
      expect(isLikelyBadSymbol("_foo", "javascript")).toBe(false);
      expect(isLikelyBadSymbol("_handler", "typescript")).toBe(false);
    });
    test("preserves Python __all__ (real module-level export)", () => {
      expect(isLikelyBadSymbol("__all__", "python")).toBe(false);
    });
  });

  describe("STDLIB symbol rejection (lang-scoped)", () => {
    test("rejects .NET stdlib Substring as csharp seed", () => {
      expect(isLikelyBadSymbol("Substring", "csharp")).toBe(true);
    });
    test("rejects .NET stdlib FromResult as csharp seed", () => {
      expect(isLikelyBadSymbol("FromResult", "csharp")).toBe(true);
    });
    test("rejects .NET stdlib ArgumentException as csharp seed", () => {
      expect(isLikelyBadSymbol("ArgumentException", "csharp")).toBe(true);
    });
    test("rejects .NET stdlib InvalidOperationException as csharp seed", () => {
      expect(isLikelyBadSymbol("InvalidOperationException", "csharp")).toBe(true);
    });
    test("preserves user-defined symbol that happens to share a stdlib name when lang differs", () => {
      // "Substring" in a Java repo is fine — .NET stdlib list shouldn't
      // bleed across languages.
      expect(isLikelyBadSymbol("Substring", "java")).toBe(false);
    });
  });

  describe("backward compat — no lang specified", () => {
    test("existing call sites without lang still work for stopwords", () => {
      expect(isLikelyBadSymbol("void")).toBe(true);
      expect(isLikelyBadSymbol("MQTTnet")).toBe(false); // not in stopwords, not stdlib
    });
    test("private-field rule does NOT apply when lang omitted", () => {
      // Without lang context, _options could be legitimate JS — don't reject.
      expect(isLikelyBadSymbol("_options")).toBe(false);
    });
    test("stdlib rule does NOT apply when lang omitted", () => {
      expect(isLikelyBadSymbol("Substring")).toBe(false);
    });
  });
});

describe("integration — end-to-end seed extraction (Patches A+B+C)", () => {
  test("MQTTnet-style hunk produces real method seed, not 'MQTTnet' namespace", () => {
    // Faithful reproducer of the real MQTTnet-9177b6ae bug pattern Codex
    // identified. Pre-fix: this returned "MQTTnet". Post-fix: returns
    // a real method or class symbol.
    const diff = `\
diff --git a/Source/MQTTnet/Implementations/CrossPlatformSocket.cs b/Source/MQTTnet/Implementations/CrossPlatformSocket.cs
@@ -50,7 +50,15 @@ namespace MQTTnet.Implementations
     public sealed class CrossPlatformSocket : IDisposable
     {
         readonly Socket _socket;
+
+        public async Task<CrossPlatformSocket> AcceptAsync(CancellationToken cancellationToken)
+        {
+            var clientSocket = await _socket.AcceptAsync(cancellationToken).ConfigureAwait(false);
+            return new CrossPlatformSocket(clientSocket);
+        }
     }
 }
`;
    const contexts = extractAllHunkContexts(diff);
    // First valid context should yield a real symbol, not "MQTTnet".
    let firstSymbol: string | null = null;
    for (const ctx of contexts) {
      const sym = extractSymbol(ctx);
      if (sym) {
        firstSymbol = sym;
        break;
      }
    }
    expect(firstSymbol).not.toBeNull();
    expect(firstSymbol).not.toBe("MQTTnet");
    expect(firstSymbol).not.toBe("System");
    // Should be one of: AcceptAsync (method), CrossPlatformSocket (class).
    expect(["AcceptAsync", "CrossPlatformSocket"]).toContain(firstSymbol!);
  });
});
