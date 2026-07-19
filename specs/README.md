# RepoScope Specs

RepoScope is a Rust CLI + GitHub Action that renders repository insight visuals
(star history chart, contributors wall, ...) as **self-contained SVG files**
committed to the repo, referenced by README via static relative paths.

Spec index:

| Spec | Title | Status |
|------|-------|--------|
| [00-mvp.md](00-mvp.md) | MVP: star-history chart + contributors wall, CLI, Action | Draft |

## Invariants (all specs)

1. **Never mutate README.md (or any hand-written file).** RepoScope only writes
   generated asset files. README references them once, by hand, via relative path.
2. **Deterministic output.** Given identical upstream data, output bytes are
   identical. No wall-clock, no randomness, no unstable ordering in outputs.
   This is what makes "commit only when changed" work via `git diff --quiet`.
3. **Spec-first.** Code must follow spec; if reality disagrees, fix the spec
   first, then the code.
4. **Commit discipline.** Conventional Commits format; generated-file commits
   default to `chore(reposcope): update repo visuals [skip ci]`.
