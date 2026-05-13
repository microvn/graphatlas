# Security policy

## Reporting a vulnerability

Please report security issues through GitHub's **Private Vulnerability
Reporting**:

> https://github.com/microvn/GraphAtlas/security/advisories/new

Do not open a public issue for suspected vulnerabilities.

## Scope

GraphAtlas is a code-context engine that reads source repositories and
exposes graph queries via a CLI and an MCP server. In-scope:

- The `graphatlas` binary (CLI + MCP server) — anything that could
  exfiltrate data outside the indexed repo, escape the cache directory,
  or execute attacker-controlled code.
- The Rust workspace crates (`ga-core`, `ga-parser`, `ga-index`,
  `ga-query`, `ga-mcp`, `ga-bench`).
- The release tarball published from this repo.

Out of scope:

- Bugs in upstream dependencies (lbug, tree-sitter, etc.) — please
  report those upstream.
- Issues in benchmark fixtures (`benches/fixtures/*`) — those are
  third-party submodules pinned for evaluation; report to the upstream
  project.
- Pre-1.0 API stability — the API will change between minor versions
  until v1.0.0.

## Response

Solo-maintained project; best-effort response within 7 days. Critical
issues take priority over feature work.
