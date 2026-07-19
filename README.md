# RepoScope

Render repository insight visuals — **star history**, **contributors wall**, …
— as self-contained SVG files your README references by path. Built to run in
GitHub Actions: no external API dependency at view time, no README rewriting,
commits only when the underlying data actually changed.

> Status: early development (MVP in progress). See [specs/](specs/).

## Why self-hosted SVGs?

- **No third-party service in the request path.** The chart is a static file
  committed to your repo; it renders as long as GitHub renders SVGs.
- **Works with the default `GITHUB_TOKEN`.** RepoScope reads stargazers via
  the GitHub GraphQL API — since the 2026-07-14 REST stargazers restriction,
  token-based REST tools need a fine-grained PAT; RepoScope does not.
- **Deterministic output.** Bytes change iff star/contributor data changes,
  so the action commits only when there is something new to show.

## License

[1PL](LICENSE)
