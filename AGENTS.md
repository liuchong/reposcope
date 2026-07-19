# AGENTS.md — reposcope

RepoScope renders repository insight visuals (star history, contributors
wall, ...) as self-contained SVG files that a README references by path.
Rust CLI + GitHub composite action.

## Build & test gates (must all pass before committing)

```bash
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test
```

## Rules

1. **Spec-first.** `specs/` governs behavior. If code and spec disagree,
   fix the spec first, then the code. Never let code silently diverge.
2. **Never mutate hand-written files** (README.md et al.) — reposcope only
   writes generated asset files.
3. **Deterministic output**: identical upstream data → identical bytes.
   No wall clock, no randomness, no unstable ordering in generated files.
4. **Commits**: Conventional Commits, English. Generated-asset commits:
   `chore(reposcope): update repo visuals [skip ci]`.
5. **Secrets**: never log or commit tokens. Token resolution:
   `--token` > `GITHUB_TOKEN` > `GH_TOKEN`.

## Key facts

- GitHub GraphQL-only for stargazers: the REST stargazers endpoint was
  restricted to repo admins/collaborators on 2026-07-14; GraphQL remains
  readable by any authenticated identity (incl. `GITHUB_TOKEN`).
- Full cursor pagination (100 edges/page); series is UTC-day cumulative,
  subsampled to ≤ 200 points; endpoint = most recent star (no "today").
- SVG is string-built, XML-validated before write (temp file + rename).
