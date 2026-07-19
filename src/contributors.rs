//! Contributors wall data (spec 00 §4.2): REST contributors endpoint,
//! bot filtering, concurrent avatar download with base64 embedding.
//!
//! Determinism note: avatar fetches use `buffered` (ordered) concurrency —
//! `buffer_unordered` would make wall order depend on network timing.

use base64::Engine as _;
use futures::stream::{self, StreamExt};

use crate::Result;
use crate::github::GitHubClient;

/// The contributors API never returns beyond the first 500.
const API_MAX: usize = 500;
/// Concurrent avatar downloads.
const AVATAR_CONCURRENCY: usize = 8;

/// One wall cell: contributor identity + avatar payload (`None` → placeholder).
#[derive(Debug)]
pub struct WallEntry {
    pub login: String,
    pub contributions: u64,
    /// `data:<mime>;base64,...` URI, or `None` for the placeholder.
    pub avatar: Option<String>,
}

/// Fetch contributors (API order = contributions desc), filter bots, embed avatars.
pub async fn fetch_wall(
    client: &GitHubClient,
    repo: &str,
    max: usize,
    include_bots: bool,
    avatar_size: u32,
) -> Result<Vec<WallEntry>> {
    let max = if max > API_MAX {
        eprintln!("reposcope: --max clamped to {API_MAX} (contributors API ceiling)");
        API_MAX
    } else {
        max
    };
    let mut contributors = Vec::new();
    let mut page = 1u32;
    while contributors.len() < max {
        let batch = client.contributors_page(repo, page).await?;
        if batch.is_empty() {
            break;
        }
        let exhausted = batch.len() < 100;
        contributors.extend(batch);
        if exhausted {
            break;
        }
        page += 1;
    }
    contributors.retain(|c| c.login.is_some() && (include_bots || !c.is_bot()));
    contributors.truncate(max);

    let entries = stream::iter(contributors)
        .map(|c| async move {
            let login = c.login.unwrap_or_default();
            let avatar = match c.avatar_url {
                Some(u) => {
                    let url = format!("{u}&s={avatar_size}");
                    match client.fetch_bytes(&url).await {
                        Ok((mime, bytes)) => Some(format!(
                            "data:{mime};base64,{}",
                            base64::engine::general_purpose::STANDARD.encode(bytes)
                        )),
                        Err(e) => {
                            eprintln!(
                                "reposcope: avatar for {login} failed ({e}); using placeholder"
                            );
                            None
                        }
                    }
                }
                None => None,
            };
            WallEntry {
                login,
                contributions: c.contributions,
                avatar,
            }
        })
        .buffered(AVATAR_CONCURRENCY)
        .collect()
        .await;
    Ok(entries)
}
