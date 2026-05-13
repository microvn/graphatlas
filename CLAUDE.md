
Default to using Bun instead of Node.js.

- Use `bun <file>` instead of `node <file>` or `ts-node <file>`
- Use `bun test` instead of `jest` or `vitest`
- Use `bun build <file.html|file.ts|file.css>` instead of `webpack` or `esbuild`
- Use `bun install` instead of `npm install` or `yarn install` or `pnpm install`
- Use `bun run <script>` instead of `npm run <script>` or `yarn run <script>` or `pnpm run <script>`
- Use `bunx <package> <command>` instead of `npx <package> <command>`
- Bun automatically loads .env, so don't use dotenv.

## APIs

- `Bun.serve()` supports WebSockets, HTTPS, and routes. Don't use `express`.
- `bun:sqlite` for SQLite. Don't use `better-sqlite3`.
- `Bun.redis` for Redis. Don't use `ioredis`.
- `Bun.sql` for Postgres. Don't use `pg` or `postgres.js`.
- `WebSocket` is built-in. Don't use `ws`.
- Prefer `Bun.file` over `node:fs`'s readFile/writeFile
- Bun.$`ls` instead of execa.

## Testing

Use `bun test` to run tests.

```ts#index.test.ts
import { test, expect } from "bun:test";

test("hello world", () => {
  expect(1).toBe(1);
});
```

## Frontend

Use HTML imports with `Bun.serve()`. Don't use `vite`. HTML imports fully support React, CSS, Tailwind.

Server:

```ts#index.ts
import index from "./index.html"

Bun.serve({
  routes: {
    "/": index,
    "/api/users/:id": {
      GET: (req) => {
        return new Response(JSON.stringify({ id: req.params.id }));
      },
    },
  },
  // optional websocket support
  websocket: {
    open: (ws) => {
      ws.send("Hello, world!");
    },
    message: (ws, message) => {
      ws.send(message);
    },
    close: (ws) => {
      // handle close
    }
  },
  development: {
    hmr: true,
    console: true,
  }
})
```

HTML files can import .tsx, .jsx or .js files directly and Bun's bundler will transpile & bundle automatically. `<link>` tags can point to stylesheets and Bun's CSS bundler will bundle.

```html#index.html
<html>
  <body>
    <h1>Hello, world!</h1>
    <script type="module" src="./frontend.tsx"></script>
  </body>
</html>
```

With the following `frontend.tsx`:

```tsx#frontend.tsx
import React from "react";
import { createRoot } from "react-dom/client";

// import .css files directly and it works
import './index.css';

const root = createRoot(document.body);

export default function Frontend() {
  return <h1>Hello, world!</h1>;
}

root.render(<Frontend />);
```

Then, run index.ts

```sh
bun --hot ./index.ts
```

For more information, read the Bun API docs in `node_modules/bun-types/docs/**.mdx`.

## GraphAtlas Eval Config (as of 2026-04-30)

**Live dataset:** `benches/uc-impact/ground-truth.json` — single source of truth, git-mined. Schema v3 flat: `seed_symbol`, `seed_file`, `expected_files`, `expected_tests`, `should_touch_files`. dev/test split inline per task. Per-fixture `<repo>.json` files (legacy LLM-era) deleted 2026-04-30.

**Build:** `bun run scripts/build-m2.ts` — orchestrator runs the 3 stages (`mine-fix-commits.ts` → `extract-seeds.ts` → `consolidate-gt.ts`). M1 is hand-curated (no generator script). M3 mines runtime via `HmcGitmine` (no static dataset to build).

**Rust types** (no shared `GroundTruth` — pick the right one per gate):
- M1 (callers/importers) → `ga_bench::M1GroundTruth` + `M1Task` (`crates/ga-bench/src/m1_ground_truth.rs`, schema v1).
- M2 (impact) → `ga_bench::M2GroundTruth` + `M2Task` + `Split` (`m2_ground_truth.rs`, schema v3, sha256-verified).
- M3 → no static struct; runtime mining via `gt_gen::hmc_gitmine::HmcGitmine`.

**Test convention:** tooling-time tests (smoke, audit, validate, harness, composite) filter `Split::Dev`. Gate test (`m2_gate_impact`) defaults to `Split::Test` — env `GA_M2_SPLIT` overrides. Test fold stays blind to tooling.

**Fixture isolation per gate:** M2 + M3 runners checkout per-task `base_commit` to drive the indexer. Each gate clones its scratch from the canonical submodule on first use → `<cache_root>/fixtures-{m2,m3}/<repo>` (hardlinked via `git clone --local`, ~1 GB scratch on hot M2). Submodules at `benches/fixtures/<repo>` stay immutable; cross-gate fixture pollution impossible by construction. See `crates/ga-bench/src/fixture_workspace.rs`.

**Live pipeline:** Rust workspace gates only — `crates/ga-{index,query,bench,mcp}/`. Run via `cargo run -p graphatlas -- bench --gate {m1|m2|m3} --uc <name> --fixture <name> --retrievers ga`.

### Archived — DO NOT extend

| Path | Why archived |
|---|---|
| `archive/rust-poc/` (2026-04-28) | Old standalone `graphatlas-poc` binary; superseded by workspace crates |
| `archive/datasets/tasks-v6-*.jsonl` (2026-04-28) | LLM-generated (`contextbench-symbol-trace`) — accuracy too low for honest bench. Replaced by `benches/uc-impact/ground-truth.json` (git-mined). M3 minimal_context UC migrated to `Hmc-gitmine` rule reading the new dataset. |
| `archive/ts-pipeline/` (2026-05-08) | TS eval/benchmark framework — superseded by `crates/ga-{core,parser,index,query,bench,mcp}/`. |
