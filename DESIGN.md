# Design System — GraphAtlas UI

**Created:** 2026-05-15
**Status:** Draft (Phase 1 prototype)
**Prototype:** `prototype/index.html`

## Product Context

- **What this is:** Local web UI cho GraphAtlas — graph-based code-context engine cho AI coding agents. 2 page: Projects list + Project detail (Graph + Index Control tabs).
- **Who it's for:** Developer dogfood (single user). Người dùng GraphAtlas qua MCP từ Claude Code / Cursor / Codex.
- **Space/industry:** Developer tools, code-analysis, MCP ecosystem. Peers: Linear (data density), Vercel (Geist), Codebase Map (graph viz), GitNexus (WebGL graph), Sourcegraph (code-search dev tool).
- **Project type:** Local web app (Bun.serve frontend + axum backend, 127.0.0.1 only). Không phải marketing, không phải hosted SaaS.

## Memorable Thing

*"Đây là graph data hiển thị theo cách developer nghĩ — dense, exact, terminal-native. Không phải dashboard SaaS thứ N."*

Mỗi design decision phải phục vụ "terminal-native" cảm giác: tight density, mono cho code-y values, restraint accent dùng đúng chỗ, no decoration noise.

## Aesthetic Direction

- **Direction:** **Industrial / Technical Editorial**
- **Decoration level:** **Minimal** — typography + 4px grid + 1 accent color gánh toàn bộ hierarchy. Không icon-circles, không gradients, không card shadows, không decorative blobs.
- **Mood:** Linear's data density × Vercel's restraint × terminal aesthetic. Reading a `git log --graph --oneline` rendered as a web app.
- **Mode:** **Dark-primary** (developer default). Light mode = Phase 2 backlog.
- **Reference sites:** Linear, Vercel dashboard, Catppuccin theme demo, Tokyo Night editor screenshots.

## Typography

**Stack:**
- **Display/Hero:** **General Sans** (Fontshare) — weights 500/600/700. Geometric, technical, không boring như Inter. Used for: page H1, brand wordmark.
- **Body / UI:** **Geist** (Vercel) — weights 400/500/600. Tabular-nums built-in via `font-feature-settings: "tnum"`. Used for: everything else (labels, buttons, table cells when content is human-readable).
- **Mono:** **Geist Mono** — for symbol IDs, file paths, slugs, hex hashes, line numbers, durations, counts in data tables, log output, signatures.

**Loading (Phase 1):**
- General Sans: Fontshare CDN `https://api.fontshare.com/v2/css?f[]=general-sans@500,600,700&display=swap`
- Geist + Geist Mono: jsDelivr `https://cdn.jsdelivr.net/npm/geist@1.3.1/dist/css/geist.css` + `geist-mono.css`
- Phase 2 (production): self-host woff2 files trong `ui-bundle/fonts/`.

**Scale (px):**
| Token | Size | Usage |
|---|---|---|
| `xs` | 10 | uppercase labels, badges, micro-meta |
| `sm` | 11 | mono data, log lines, secondary text |
| `md` | 12 | UI controls (buttons, inputs), table cells |
| `body` | 13 | default body text |
| `lg` | 14 | brand wordmark, emphasized inline |
| `xl` | 16 | section labels, expanded |
| `2xl` | 20 | sub-headings |
| `h2` | 22 | page H1 in detail view |
| `h1` | 28 | page H1 in list view |
| `display` | 40 | hero (reserved — Phase 2) |

**Anti-list (never use as primary):**
Inter, Roboto, Open Sans, Lato, Montserrat, Poppins, Arial, Helvetica, **Space Grotesk** (every AI design tool converges on it — convergence trap), Comic Sans, Papyrus, system-ui as display.

## Color

**Approach:** Restrained. 1 accent + neutrals + 4 semantic. Accent dùng đúng chỗ: focus ring, primary CTA, selected node, brand mark.

**Palette:**

```css
/* Neutrals (dark-primary) */
--bg:        #0A0A0A;   /* page background — warm near-black, không pure black */
--surface:   #141414;   /* card/panel surface */
--surface-2: #1A1A1A;   /* selected row, nested surface */
--border:    #262626;   /* default border */
--border-2:  #333333;   /* control border (button, input) */
--text:      #FAFAFA;   /* primary text */
--subtle:    #A3A3A3;   /* secondary text */
--muted:     #737373;   /* labels, meta, captions */

/* Accent — Tokyo Night blue. Coding-fatigue friendly, editor-tribe signal. */
--accent:        #7AA2F7;   /* primary accent */
--accent-soft:   #7AA2F71A; /* 10% alpha — chip bg, soft highlight */
--accent-strong: #9CB7F9;   /* hover state */
--accent-cool:   #7DD3FC;   /* secondary accent (struct nodes, syntax types) */

/* Semantic */
--ok:    #22C55E;   /* fresh/running/success */
--warn:  #F59E0B;   /* stale/warning */
--err:   #EF4444;   /* corrupt/error */
--info:  #38BDF8;   /* building/info */
```

**Usage rules:**
- Background depth 4 levels: `--bg` (page) → `--surface` (card) → `--surface-2` (selected) → optional inset `--bg`.
- Text 3 levels: `--text` (primary) → `--subtle` (secondary) → `--muted` (meta/labels).
- Accent: ONLY for focus, primary CTA, selected graph node, hover edge on interactive controls. Không dùng cho text body, không dùng cho lớn diện tích background.
- Semantic colors: cho status badges (fresh/stale/corrupt/building/orphan) và dot indicators (watcher running/stopped/errored). Không dùng làm accent thay thế.

**Light mode:** Phase 2 — defer. Khi build sẽ redesign surfaces (không invert), giảm saturation accent 10-20%.

## Spacing

**Base unit:** 4px.

**Scale:**
| Token | px | Usage |
|---|---|---|
| `s1` | 4 | inline tight gap |
| `s2` | 8 | row gap, button padding-y |
| `s3` | 12 | cell padding, sm padding |
| `s4` | 16 | default section gap, md padding |
| `s5` | 24 | major section gap |
| `s6` | 32 | page section, list-header padding-y |
| `s7` | 48 | breath at page edge |
| `s8` | 64 | hero spacing |

**Density:** Compact. Table row 32px (10px padding + 12px content). Form input 32px. Button 32px. Topbar 44px. Tab strip 40px.

## Layout

- **Approach:** Grid-disciplined. App grid, not editorial.
- **Top nav:** 44px sticky topbar với brand + meta info (backend addr, token status).
- **Page 1 (Projects list):** Full-width table với 11-column structure: 4 basic + 4 index size + 1 health + 2 watcher. Column-group dividers via `border-left` 1px. Hover row → `--surface`. Sort default `last_indexed desc`.
- **Page 2 (Project detail):**
  - Header: breadcrumb + H1 + meta line, action buttons right.
  - Tabs ngang: `Graph` (default) | `Index control`. Active state = `border-bottom: 2px solid var(--accent)`.
  - **Graph tab:** 2-pane layout — canvas (flex 1) + detail panel (360px right, collapsible). Canvas có floating toolbar top-left + meta top-right + legend bottom-left.
  - **Index control tab:** 2-col grid card layout, max-width 1100px, gap 16px. Cards: Status / Cache / Reindex (span 2) / Watcher / Last run.
- **Max content width:** Bảng full-width, detail tabs cap 1100px.
- **Border radius:** Hierarchical — sm `4px` (inputs/buttons/badges), md `6px` (cards), lg `10px` (modals — Phase 2). Badges dùng `2px` cứng cạnh — terminal-y.

## Motion

- **Approach:** Minimal-functional only. Không entrance animation, không scroll-driven.
- **Easing:** `ease-out` cho hover/enter, `ease-in-out` cho tab/state change.
- **Duration:** 100ms (hover, dot pulse), 150ms (border-color, background change), không có gì >250ms.
- **Glow:** Selected graph node có 1 radial-gradient glow (`--accent` 0.5 → 0) — không animate, chỉ visual depth.

## Decisions — Risks Taken

| # | Risk | Why | Trade-off accepted |
|---|---|---|---|
| R-1 | **Tokyo Night blue accent `#7AA2F7`** thay vì Tailwind blue/purple SaaS default | Editor-tribe signal. Devs nhận ra palette từ Tokyo Night / Catppuccin themes. Low saturation = không fatigue trong session dài. | Ít "corporate professional" hơn — không sao, GA không cần |
| R-2 | **Mono aggressively trên data tables** (slug, file path, count, duration, hex hash) — chỉ tên project + label dùng sans | Signal "data này là code". Khác với Codebase Map dùng sans cho mọi thứ. | Density cao, mono đọc chậm hơn cho non-tech — nhưng users là devs |
| R-3 | **No icons** (hoặc cực ít stroke 1.5px nếu thật cần) | Anti-AI-slop "3-column icon grid". Chữ + màu + size phân biệt kind. | User cần learn type colors. OK vì sample size = 1 user dogfood |
| R-4 | **Dark-only Phase 1** | Dev default, no toggle complexity, focus build | Light mode users phải đợi Phase 2 |
| R-5 | **Compact density (32px row)** thay vì SaaS-comfortable 48-56px | More data per viewport, terminal-native | Yêu cầu pointer precision cao hơn — OK desktop-only |

## Components — Design Patterns

**Badge** (state indicators):
- Mono, uppercase, `font-size: 10px`, `letter-spacing: 0.04em`, `padding: 2px 6px`, `border-radius: 2px`.
- Semantic backgrounds at 10% alpha: `--ok` for fresh/running, `--warn` for stale, `--err` for corrupt/errored, `--info` for building, `--muted` for orphan/stopped.

**Button:**
- Primary: `background: var(--accent)`, `color: var(--bg)`, font-weight 600. Hover → `--accent-strong`.
- Ghost: `background: transparent`, `border: 1px solid var(--border)`, hover border → `--accent`.
- Height 32px, padding 8/14, radius 4px, font 12px/1.

**Input:**
- Height 32px, padding 0/12, radius 4px, bg `--surface`, border 1px `--border`, focus border `--accent`. Mono font nếu chứa code/path.

**Card (Index control tab):**
- bg `--surface`, border 1px `--border`, radius 6px, padding 16px.
- Section header: uppercase 10px label, `letter-spacing: 0.06em`, color `--muted`, border-bottom 1px.
- Card row: flex space-between, padding 6px 0, border-top divider.

**Graph node (canvas):**
- Selected: radius 8px filled `--accent` + radial glow ~36px.
- Function: radius 4px `--ok` (#22C55E).
- Struct: radius 4px `--accent-cool` (#7DD3FC).
- Class: radius 4px soft purple (#A78BFA) — OO container.
- Interface: radius 4px teal (#5EEAD4) — contract.
- Enum: radius 4px amber (#FBBF24) — discriminated values.
- Trait: radius 4px pink (#F472B6) — capability (Rust trait / Haskell typeclass).
- Method: radius 4px `--subtle` (#A3A3A3).
- Handler/special: radius 4px `--warn`.
- Other/connector: radius 3px `--muted`.
- **Note:** earlier rev collapsed struct/class/interface/enum onto one `--accent-cool`; dogfood proved the legend chips were unreadable, so each type-kind now owns a distinct hue within the cool Tokyo Night band.
- Labels: Geist Mono 10px, color `--subtle` (default) / `--text` (focused).

**Graph edge:**
- Calls: solid 1px `#333` (border-2).
- References: dashed 3px-3px gap `--border`.
- Hover: stroke → `--accent`.

## Anti-patterns — Never Ship

- ❌ Purple/violet gradient hero
- ❌ 3-column feature grid with icons in colored circles
- ❌ Centered everything with uniform comfort spacing
- ❌ Bubble border-radius on every element
- ❌ Gradient buttons as primary CTA
- ❌ Stock-photo hero, lifestyle imagery
- ❌ system-ui as display font
- ❌ "Built for developers" / "Powered by AI" marketing copy
- ❌ Inter, Space Grotesk, Roboto as primary fonts (overused — convergence trap)
- ❌ Icons-everywhere (Lucide grid of muted-gray icons in every action)
- ❌ Soft pastel chips with `border-radius: 999px`
- ❌ Card shadows / elevation surfaces (we're flat, terminal-native)

## CSS Variable Reference

Copy-paste-able root for any new component:

```css
:root {
  --bg: #0A0A0A; --surface: #141414; --surface-2: #1A1A1A;
  --border: #262626; --border-2: #333333;
  --text: #FAFAFA; --subtle: #A3A3A3; --muted: #737373;
  --accent: #7AA2F7; --accent-soft: #7AA2F71A;
  --accent-strong: #9CB7F9; --accent-cool: #7DD3FC;
  --ok: #22C55E; --warn: #F59E0B; --err: #EF4444; --info: #38BDF8;
  --font-display: 'General Sans', system-ui, sans-serif;
  --font-body: 'Geist', system-ui, sans-serif;
  --font-mono: 'Geist Mono', ui-monospace, monospace;
  --s1:4px; --s2:8px; --s3:12px; --s4:16px; --s5:24px; --s6:32px; --s7:48px; --s8:64px;
  --r-sm:4px; --r-md:6px; --r-lg:10px;
}
```

## Decisions Log

| Date | Decision | Rationale |
|------|----------|-----------|
| 2026-05-15 | Initial design system + prototype | `/design-consultation`. Industrial/Technical Editorial direction chốt từ product context (dev tool, MCP, terminal-spawned). |
| 2026-05-15 | Accent = Tokyo Night blue `#7AA2F7` (đổi từ amber `#FFB020`) | User feedback: cần màu dễ nhìn khi coding session dài. Editor-tribe palette familiar to devs. |
| 2026-05-15 | Prototype `prototype/index.html` ship | Full-screen Page 1 + Page 2 (Graph + Index Control tabs) cùng 1 file để dogfood. |
