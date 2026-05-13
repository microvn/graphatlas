#!/usr/bin/env bun
// M2 dataset orchestrator — runs the 3 mining stages in sequence.
// Keeps the original 3 scripts intact as libraries (m2_cochange_validate.rs:7
// and hmc_gitmine.rs:7 reference specific line numbers in extract-seeds.ts;
// inlining would break those comments).
import { $ } from "bun";

console.log("=== M2 Stage 1: mine-fix-commits ===");
await $`bun run scripts/mine-fix-commits.ts`;

console.log("\n=== M2 Stage 2: extract-seeds ===");
await $`bun run scripts/extract-seeds.ts`;

console.log("\n=== M2 Stage 3: consolidate-gt ===");
await $`bun run scripts/consolidate-gt.ts`;

console.log("\n[build-m2] Done. ground-truth.json + ground-truth.sha256 regenerated.");
