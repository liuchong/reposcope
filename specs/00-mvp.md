# Spec 00 — MVP: star-history chart + contributors wall

Status: Draft
Crate: `reposcope` (single crate, lib + bin, edition 2024, MSRV 1.85)
Initial version: 0.1.0. License: 1PL.

## 1. Goal

A Rust CLI that runs locally and inside GitHub Actions, fetching public GitHub
data and rendering **self-contained SVG files**:

- `star-history` — line/area chart of cumulative stargazers over time.
- `contributors` — circular avatar wall of repo contributors.

Plus `snippet`, which prints the README markdown/HTML referencing the generated
files (for humans to paste once).

Future (deferred, §12): more widgets, PNG output, xkcd theme, PR mode.

### Non-goals (MVP)

- No README (or any hand-written file) mutation, ever (invariant 1).
- No external rendering service / API dependency at view time. The SVG files
  are committed assets; GitHub camo serves them.
- No contribution-type classification. The MVP
  wall is contribution-count ordered avatars only.
- No multi-repo comparison charts.

## 2. Architecture

Single crate, lib + thin bin:

```
src/
  main.rs          # exit-code mapping only
  lib.rs
  cli.rs           # clap subcommands
  github.rs        # minimal REST client (repo meta, stargazers, contributors, avatars)
  star_history.rs  # sampling + time-series building
  contributors.rs  # fetch/filter/avatar embedding
  svg.rs           # chart + wall rendering primitives (string-built, no chart lib)
  snippet.rs       # README reference snippet printing
```

Dependencies: `anyhow thiserror serde serde_json reqwest(rustls,json) tokio
clap(derive) base64 futures`. Dev: `httpmock`, `tempfile`. No SVG/chart crate —
SVG is string-built and XML-validated before writing (see §6).

## 3. CLI contract

```
reposcope star-history  --repo owner/name --out <file.svg> [--theme light|dark]
reposcope contributors  --repo owner/name --out <file.svg> [--max 100] [--cols 12] [--avatar-size 64] [--include-bots]
reposcope snippet       --repo owner/name [--output-dir assets/reposcope] [--dark]
```

- `--repo` falls back to `GITHUB_REPOSITORY` env; error if neither.
- Token: `--token` flag, else `GITHUB_TOKEN` then `GH_TOKEN` env. **A token is
  required** — the GraphQL API has no anonymous tier. In GitHub Actions the
  default `github.token` works out of the box (see §4.1).
- `--out`: parent dirs are created. Default theme `light`; `dark` only changes
  the palette (same layout); `--theme both` fetches once and writes both
  `<out>` (light) and `<stem>-dark.svg` (dark) in a single run.
- Exit codes: `0` success; `1` usage/config error; `2` GitHub API failure
  after retries. Errors are human-readable single lines on stderr.
- User-Agent: `reposcope/<version>`.

## 4. Data fetching

### 4.1 Star history (GraphQL, full cursor pagination)

**Why GraphQL-only (verified 2026-07-19):** as of 2026-07-14 GitHub restricts
the REST stargazers endpoint to repo admins/collaborators. Bot identities
(`github-actions[bot]`, GitHub App installation tokens) are denied (403/404)
even on their own repo; user tokens get 404 on repos they don't collaborate
on. The **GraphQL `stargazers` connection remains readable by any
authenticated identity** — including the Actions `GITHUB_TOKEN` and App
installation tokens — for any public repo (verified against repos with 0,
3.7k, and 114k stars). GraphQL-only keeps one code path and needs no PAT
setup, unlike REST-based tools.

1. `POST {base}/graphql` with query:
   ```
   query($owner:String!,$name:String!,$after:String) {
     repository(owner:$owner, name:$name) {
       stargazers(first:100, after:$after) {
         totalCount
         pageInfo { hasNextPage endCursor }
         edges { starredAt }
       }
     }
     rateLimit { remaining resetAt }
   }
   ```
2. Paginate the cursor chain sequentially (`after = endCursor` while
   `hasNextPage`) until all stars are fetched. Cost: `ceil(S/100)` requests
   (~1 rate point each); log progress to stderr every 50 pages.
3. Series: all `starredAt` timestamps → cumulative count bucketed by UTC day
   → if > 200 points, subsample every `ceil(n/200)` days, always keeping
   first and last. `S = 0`: render an empty-state chart ("No stars yet").
4. Endpoint (spec §5.2): the final edge of the last page **is** the most
   recent star → point `(its starredAt, total fetched)`. No wall clock.
5. If stars are added mid-pagination, counts may drift slightly; the chart
   reflects fetched data and self-heals on the next run.

### 4.2 Contributors

The REST contributors endpoint is **not** affected by the 2026-07-14
stargazers restriction (verified 2026-07-19: readable anonymously, by user
tokens, and by App installation tokens, on own and foreign repos).

1. `GET /repos/{owner}/{repo}/contributors?per_page=100&page=N` (no `anon`)
   until `--max` reached or pages exhausted. API hard ceiling: 500
   (`--max` is clamped to 500 with a stderr note).
2. Filter: drop entries with `type == "Bot"` unless `--include-bots`.
3. For each kept contributor: download `avatar_url` with `&s=<avatar-size>`
   appended, ≤ 8 concurrent. On download failure: fallback placeholder (gray
   circle + first char of login, uppercased).
4. **Pixel-normalize every avatar** (invariant 2): the avatars CDN re-encodes
   images per edge node, so identical URLs return *different bytes* across
   requests (observed 2026-07-19: gd-jpeg output drifted between runs,
   causing daily no-op commits). Decode the image and re-encode it as PNG
   with fixed encoder settings — identical pixels → identical bytes; a real
   avatar change → different pixels → a real commit. Undecodable bytes also
   fall back to the placeholder. Embed as `data:image/png;base64,...`.

> Why base64-embedded: an SVG loaded via `<img>` (how GitHub renders README
> images) cannot fetch external subresources. External `href`s would render
> blank. Data URIs are self-contained and work.

### 4.3 HTTP / GraphQL behavior

- GraphQL `errors` with type `RATE_LIMITED`: sleep until `rateLimit.resetAt`
  (capped at 10 min), max 5 retries.
- HTTP 5xx / network errors: exponential backoff (base 1s, ×2, max 5 retries);
  honor `Retry-After` when present.
- `data.repository == null` → exit 2 with "repository not found (or token
  lacks access)". Other GraphQL `errors` → exit 2 with the first message.
- Per-request timeout 15s.

## 5. Determinism contract (invariant 2)

1. No timestamps of generation, no random IDs (SVG IDs are sequential:
   `c0, c1, ...`), floats formatted with fixed precision (2 decimals).
2. **Endpoint rule.** The chart's final point is `(starred_at of the most
   recent star, S)` — *not* `(today, S)`. The full series is therefore a pure
   function of immutable star history; output bytes change **iff** the
   underlying data changed. (hosted charts using wall-clock `today` would shift
   the x-domain daily and cause empty-diff commits — explicitly rejected.)
3. Contributor wall order = API order (contributions desc), stable. Avatar
   fetches use ordered concurrency (`buffered`) and avatars are
   pixel-normalized to PNG (§4.2) — CDN byte drift must not leak into output.
4. Consequence: `git add` + `git diff --cached --quiet` is a correct
   change-detector; no separate state file is needed.

## 6. SVG rendering contract

### 6.1 Star-history chart

- Canvas: `width × height = 800 × 533` viewBox (3:2), `width="100%"
  height="auto"` style so it scales in README. Padding: L90 R35 T80 B70.
- Scales: x = UTC time from first star to most recent star; y = linear
  `0 .. y_max` where `y_max = nice_ceil(S)` (nice-number rounding, ≥ 25).
- Ticks: y 5 ticks, K/M formatting (`12K`, `12.5K`, `1M`); x ~6 ticks,
  adaptive `Jan 2026` / `2024` labels. Light gridlines at y ticks.
- Series: line (3px, round caps/joins) smoothed with Catmull-Rom → cubic
  Bézier; area under line filled with vertical gradient (22% → 2% line color);
  end dot (r=5) + final count label, clamped inside canvas.
- Legend top-left: color swatch + `owner/name`.
- Themes: `light` (bg `#ffffff`, line `#2563eb`, text `#1f2328`) and `dark`
  (bg `#0d1117`, line `#60a5fa`, text `#e6edf3`). Palette switch only.
- Font: system stack (`-apple-system, "Segoe UI", Roboto, Helvetica, Arial,
  sans-serif`). No embedded font in MVP.

### 6.2 Contributors wall

- Grid: `--cols` columns, `--avatar-size` px cells, gap 8px, padding 16px;
  rows = `ceil(n / cols)`. Intrinsic width/height derived from the grid, with
  `viewBox` + `style="max-width:100%;height:auto"` so the wall never upscales
  but shrinks to fit narrow containers.
- Each cell: `<clipPath>`-clipped circular `<image>` with the data URI;
  `<title>@login (N contributions)</title>` tooltip; no names rendered (MVP).
- Background transparent (works on light and dark READMEs).

### 6.3 Output hygiene

- Generated SVG is parsed back (`ElementTree`-equivalent) before writing;
  invalid → exit 1, no partial file (write via temp file + rename).
- Golden-file tests pin both themes of the chart and a wall fixture.

## 7. GitHub Action contract

Composite `action.yml` at repo root (download-binary model):

```yaml
inputs:
  version:        { default: "v0.1.0" }       # pinned, no floating tags
  github_token:   { default: ${{ github.token }} }
  repo:           { default: ${{ github.repository }} }
  output_dir:     { default: "assets/reposcope" }
  star_history:   { default: "true" }          # emits light + -dark variant in one run
  contributors:   { default: "true" }
  max_contributors: { default: "100" }
  cols:           { default: "12" }
  commit_message: { default: "chore(reposcope): update repo visuals [skip ci]" }
  push:           { default: "true" }
  branch:         { default: "" }              # see "Branch publishing" below

outputs:
  changed:        # "true" when files were committed (from the commit step)
  files:          # newline-separated changed paths
```

**Branch publishing.** When `branch` is set (e.g. `reposcope`), generated
files are committed to that branch instead of the checked-out one — created
as an orphan-style branch on first run (fresh root commit, unrelated
history), so the main branch's commit history is never polluted by asset
updates. Mechanism: a setup step clones the branch (or `git init -b` when it
doesn't exist) into `$RUNNER_TEMP` using an `x-access-token` remote (token
never logged); generation steps write into that directory; the commit step
stages **only `output_dir`** and pushes to `branch`. The README then
references the assets via stable `raw.githubusercontent.com/<repo>/<branch>/<path>`
URLs, pasted once (§8). When `branch` is empty, behavior is the direct
commit-if-changed on the checked-out branch.

Steps: platform resolve (linux-x64 only) → `actions/cache@v4` on the binary
dir (keyed by version) → download
`reposcope-<version>-x86_64-unknown-linux-musl.tar.gz` from the release and
`sha256sum -c` on cache miss → run enabled subcommands into `output_dir`
(`GITHUB_TOKEN` via env, never argv) → commit-if-changed (below).

```bash
git add -A "$OUTPUT_DIR"
git diff --cached --quiet && exit 0          # nothing changed → no commit
git -c user.name=github-actions[bot] -c user.email=41898282+github-actions[bot]@users.noreply.github.com \
    commit -m "$COMMIT_MESSAGE"
git push
```

- The action **only ever stages `output_dir`** (invariant 1 enforcement).
- Consumer workflow: `permissions: contents: write`;
  `on: schedule (daily) + workflow_dispatch`; `concurrency` group per repo.
- Branch protection / forks: direct push assumes the default branch accepts
  Actions pushes. PR-based mode is deferred (§12).

## 8. README snippet contract

`reposcope snippet` prints (humans paste once; reposcope never writes it).

Direct mode (assets on the same branch as the README):

```html
<picture>
  <source media="(prefers-color-scheme: dark)" srcset="assets/reposcope/star-history-dark.svg">
  <img alt="Star History" src="assets/reposcope/star-history.svg">
</picture>
<img alt="Contributors" src="assets/reposcope/contributors.svg">
```

Branch mode (`--repo owner/name --branch reposcope`): absolute
`https://raw.githubusercontent.com/<repo>/<branch>/<output_dir>/<file>` URLs,
since relative paths cannot cross branches. Public repos only (raw URLs need
no auth).

Relative paths resolve from the README location; `output_dir` input keeps the
two in sync.

## 9. Packaging & release

- Workspace single crate `reposcope` (0.1.0). CI: fmt + `clippy
  --all-targets -- -D warnings` + tests + musl build smoke.
- Tag `v*` → musl static build → strip → `.tar.gz` + `.sha256` → GitHub
  Release (softprops/action-gh-release). No floating major tags; consumers pin
  exact versions.
- Remote repo creation on GitHub is a separate, later step (local-only for now).

## 10. Testing & acceptance

- Unit: Link-header parsing, sample-page index selection, day bucketing +
  subsampling, nice-number/K-M tick math, Catmull-Rom path, bot filtering,
  base64 mime detection, SVG escaping.
- Integration (httpmock, `POST /graphql`): multi-page pagination flow,
  empty repo, contributors flow incl. avatar fetch + one forced avatar
  failure → placeholder; **double run → byte-identical outputs**
  (determinism proof).
- Gates: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
  `cargo test` — all green before any release.
- Manual acceptance: run against a zero-star repo (empty state), a
  ~10k-star repo (multi-page), and one >100k-star repo (full-pagination
  proof); eyeball SVGs; confirm a second run produces zero git diff.

## 11. Milestones

| M | Scope | Acceptance |
|---|-------|-----------|
| M1 | Scaffold + `star-history` | Chart renders for tiny & huge repos; tests green |
| M2 | `contributors` | Wall renders, bots excluded, placeholder fallback works |
| M3 | `snippet` + determinism audit | Double-run byte equality proven in tests |
| M4 | `action.yml` + CI/release workflows | Tag v0.1.0 builds release; action runs from a test repo |
| M5 | Dogfood | reposcope's own README uses reposcope; then onboard another owned repo |

## 12. Deferred (future specs)

PNG output (resvg), xkcd hand-drawn theme (embedded font + turbulence filter),
PR-mode commits for protected branches, orphan-branch publishing, multi-repo
charts, more widgets (issues/PR pulse, traffic, language breakdown), GitHub
Marketplace listing, `reposcope[bot]` app identity.
