//! clap command-line surface and subcommand dispatch.

use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand, ValueEnum};

use crate::github::GitHubClient;
use crate::{Result, ScopeError, contributors, star_history, svg};

#[derive(Parser)]
#[command(
    name = "reposcope",
    version,
    about = "Render repo insight visuals as self-contained SVGs for your README"
)]
pub struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Generate a star-history chart SVG for a repository.
    StarHistory {
        /// Repository as owner/name; falls back to GITHUB_REPOSITORY.
        #[arg(long)]
        repo: Option<String>,
        /// Output SVG file path (parent directories are created).
        #[arg(long)]
        out: PathBuf,
        /// Color theme.
        #[arg(long, value_enum, default_value_t = ThemeArg::Light)]
        theme: ThemeArg,
        /// GitHub token; falls back to GITHUB_TOKEN then GH_TOKEN env vars.
        #[arg(long)]
        token: Option<String>,
    },
    /// Generate a contributors avatar-wall SVG for a repository.
    Contributors {
        /// Repository as owner/name; falls back to GITHUB_REPOSITORY.
        #[arg(long)]
        repo: Option<String>,
        /// Output SVG file path (parent directories are created).
        #[arg(long)]
        out: PathBuf,
        /// Maximum number of contributors (clamped to 500 by the API).
        #[arg(long, default_value_t = 100)]
        max: usize,
        /// Grid columns.
        #[arg(long, default_value_t = 12)]
        cols: u32,
        /// Avatar size in pixels.
        #[arg(long, default_value_t = 64)]
        avatar_size: u32,
        /// Include bot accounts (excluded by default).
        #[arg(long)]
        include_bots: bool,
        /// GitHub token; falls back to GITHUB_TOKEN then GH_TOKEN env vars.
        #[arg(long)]
        token: Option<String>,
    },
    /// Print the README snippet referencing the generated SVGs (paste once).
    Snippet {
        /// Directory the SVGs live in, relative to the README (used in paths).
        #[arg(long, default_value = "assets/reposcope")]
        output_dir: String,
        /// Include the dark-theme <source> for the star-history chart.
        #[arg(long)]
        dark: bool,
        /// Repository as owner/name — required with --branch (builds raw URLs).
        #[arg(long)]
        repo: Option<String>,
        /// Publish branch: emit raw.githubusercontent.com URLs (branch mode)
        /// instead of README-relative paths.
        #[arg(long)]
        branch: Option<String>,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum ThemeArg {
    Light,
    Dark,
    /// One fetch, two renders: `<out>` (light) + `<stem>-dark.svg`.
    Both,
}

/// Entry point used by `main`; maps errors to [`ScopeError`].
pub async fn run() -> Result<()> {
    let cli = Cli::try_parse().map_err(|e| ScopeError::Usage(e.to_string()))?;
    match cli.command {
        Command::StarHistory {
            repo,
            out,
            theme,
            token,
        } => star_history_cmd(&repo, &out, theme, &token).await,
        Command::Contributors {
            repo,
            out,
            max,
            cols,
            avatar_size,
            include_bots,
            token,
        } => contributors_cmd(&repo, &out, max, cols, avatar_size, include_bots, &token).await,
        Command::Snippet {
            output_dir,
            dark,
            repo,
            branch,
        } => {
            print!(
                "{}",
                snippet(&output_dir, dark, repo.as_deref(), branch.as_deref())?
            );
            Ok(())
        }
    }
}

/// The paste-once README snippet (spec 00 §8). reposcope never writes README.
fn snippet(
    output_dir: &str,
    dark: bool,
    repo: Option<&str>,
    branch: Option<&str>,
) -> Result<String> {
    let dir = output_dir.trim_end_matches('/');
    let base = match branch {
        Some(b) => {
            let repo = repo
                .ok_or_else(|| ScopeError::Usage("--branch requires --repo owner/name".into()))?;
            // Validate the repo part while we're at it.
            resolve_repo(&Some(repo.to_string()))?;
            format!(
                "https://raw.githubusercontent.com/{repo}/{}/{dir}",
                b.trim_matches('/')
            )
        }
        None => dir.to_string(),
    };
    let star = if dark {
        format!(
            "<picture>\n  <source media=\"(prefers-color-scheme: dark)\" srcset=\"{base}/star-history-dark.svg\">\n  <img alt=\"Star History\" src=\"{base}/star-history.svg\">\n</picture>"
        )
    } else {
        format!("![Star History]({base}/star-history.svg)")
    };
    Ok(format!(
        "{star}\n\n![Contributors]({base}/contributors.svg)\n"
    ))
}

async fn contributors_cmd(
    repo: &Option<String>,
    out: &Path,
    max: usize,
    cols: u32,
    avatar_size: u32,
    include_bots: bool,
    token: &Option<String>,
) -> Result<()> {
    let repo = resolve_repo(repo)?;
    let client = GitHubClient::new(&resolve_token(token)?)?;
    let entries = contributors::fetch_wall(&client, &repo, max, include_bots, avatar_size).await?;
    let svg_text = svg::render_contributor_wall(&entries, &svg::WallOptions { cols, avatar_size });
    write_svg(out, &svg_text)?;
    println!("wrote {}", out.display());
    Ok(())
}

async fn star_history_cmd(
    repo: &Option<String>,
    out: &Path,
    theme: ThemeArg,
    token: &Option<String>,
) -> Result<()> {
    let repo = resolve_repo(repo)?;
    let client = GitHubClient::new(&resolve_token(token)?)?;
    let points = star_history::fetch_star_series(&client, &repo).await?;
    let write = |out: &Path, theme: &svg::Theme| -> Result<()> {
        write_svg(out, &svg::render_star_chart(&points, &repo, theme))?;
        println!("wrote {}", out.display());
        Ok(())
    };
    match theme {
        ThemeArg::Light => write(out, &svg::Theme::light())?,
        ThemeArg::Dark => write(out, &svg::Theme::dark())?,
        ThemeArg::Both => {
            write(out, &svg::Theme::light())?;
            write(&dark_sibling(out), &svg::Theme::dark())?;
        }
    }
    Ok(())
}

/// `foo.svg` → `foo-dark.svg` (spec 00 §3).
fn dark_sibling(out: &Path) -> PathBuf {
    let stem = out
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    out.with_file_name(format!("{stem}-dark.svg"))
}

/// Resolve owner/name from the flag or `GITHUB_REPOSITORY`.
fn resolve_repo(repo: &Option<String>) -> Result<String> {
    let repo = repo
        .clone()
        .or_else(|| std::env::var("GITHUB_REPOSITORY").ok())
        .ok_or_else(|| {
            ScopeError::Usage(
                "repo required: pass --repo owner/name or set GITHUB_REPOSITORY".into(),
            )
        })?;
    let valid = repo.split('/').count() == 2
        && repo.split('/').all(|p| {
            !p.is_empty()
                && p.chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        });
    if !valid {
        return Err(ScopeError::Usage(format!(
            "invalid repo {repo:?}: expected owner/name"
        )));
    }
    Ok(repo)
}

/// Token resolution order: flag > GITHUB_TOKEN > GH_TOKEN. A token is
/// required — the GraphQL API has no anonymous tier (spec 00 §3).
fn resolve_token(token: &Option<String>) -> Result<String> {
    token
        .clone()
        .or_else(|| std::env::var("GITHUB_TOKEN").ok())
        .or_else(|| std::env::var("GH_TOKEN").ok())
        .filter(|t| !t.is_empty())
        .ok_or_else(|| {
            ScopeError::Usage("token required: pass --token or set GITHUB_TOKEN / GH_TOKEN".into())
        })
}

/// Validate the SVG parses, then write atomically (temp file + rename).
fn write_svg(path: &Path, content: &str) -> Result<()> {
    roxmltree::Document::parse(content)
        .map_err(|e| ScopeError::Render(format!("generated SVG failed XML validation: {e}")))?;
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .map_err(|e| ScopeError::Usage(format!("cannot create {}: {e}", parent.display())))?;
    }
    let tmp = path.with_extension("svg.tmp");
    std::fs::write(&tmp, content)
        .map_err(|e| ScopeError::Usage(format!("cannot write {}: {e}", tmp.display())))?;
    std::fs::rename(&tmp, path)
        .map_err(|e| ScopeError::Usage(format!("cannot rename to {}: {e}", path.display())))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dark_sibling_path() {
        assert_eq!(
            dark_sibling(&PathBuf::from("a/b/star-history.svg")),
            PathBuf::from("a/b/star-history-dark.svg")
        );
        assert_eq!(
            dark_sibling(&PathBuf::from("x.svg")),
            PathBuf::from("x-dark.svg")
        );
    }

    #[test]
    fn snippet_variants() {
        let s = snippet("assets/reposcope", true, None, None).unwrap();
        assert!(
            s.contains("<picture>")
                && s.contains("star-history-dark.svg")
                && s.contains("contributors.svg")
        );
        let plain = snippet("assets/reposcope/", false, None, None).unwrap();
        assert!(
            !plain.contains("<picture>")
                && plain.contains("![Star History](assets/reposcope/star-history.svg)")
        );
    }

    #[test]
    fn snippet_branch_mode() {
        let s = snippet(
            "assets/reposcope",
            true,
            Some("owner/repo"),
            Some("reposcope"),
        )
        .unwrap();
        assert!(s.contains("https://raw.githubusercontent.com/owner/repo/reposcope/assets/reposcope/star-history.svg"));
        // --branch without --repo is a usage error.
        assert!(snippet("assets/reposcope", false, None, Some("reposcope")).is_err());
        assert!(snippet("assets/reposcope", false, Some("bad"), Some("reposcope")).is_err());
    }

    #[test]
    fn repo_validation() {
        assert!(resolve_repo(&Some("owner/name".into())).is_ok());
        assert!(resolve_repo(&Some("a.b-c_d/e".into())).is_ok());
        assert!(resolve_repo(&Some("name".into())).is_err());
        assert!(resolve_repo(&Some("a/b/c".into())).is_err());
        assert!(resolve_repo(&Some("/name".into())).is_err());
        assert!(resolve_repo(&Some("owner/".into())).is_err());
        assert!(resolve_repo(&Some("owner/na me".into())).is_err());
    }
}
