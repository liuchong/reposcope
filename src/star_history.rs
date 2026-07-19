//! Star-history series construction: full cursor pagination over the GraphQL
//! stargazers connection, then UTC-day bucketing (spec 00 §4.1). The series
//! is a pure function of immutable star history — deterministic (§5).

use crate::github::GitHubClient;
use crate::{Result, ScopeError};

/// Max points emitted after day-bucketing.
const MAX_DAILY_POINTS: usize = 200;
/// Stderr progress cadence (pages).
const PROGRESS_EVERY_PAGES: u32 = 50;

/// One chart point: unix seconds (UTC) and cumulative star count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StarPoint {
    pub ts: i64,
    pub count: u64,
}

/// Fetch the full star history and build the deterministic time series.
///
/// The final edge of the last page is the most recent star, so the series
/// naturally ends at `(most recent star day, total)` — never wall-clock.
pub async fn fetch_star_series(client: &GitHubClient, repo: &str) -> Result<Vec<StarPoint>> {
    let mut timestamps: Vec<i64> = Vec::new();
    let mut after: Option<String> = None;
    let mut total: u64;
    let mut pages = 0u32;
    loop {
        let page = client.stargazers_page(repo, after.as_deref()).await?;
        total = page.total_count;
        pages += 1;
        for s in &page.starred_at {
            timestamps.push(parse_ts(s)?);
        }
        if pages % PROGRESS_EVERY_PAGES == 0 {
            eprintln!(
                "reposcope: fetched {}/{total} stars{}",
                timestamps.len(),
                page.rate_remaining
                    .map(|r| format!(", rate remaining {r}"))
                    .unwrap_or_default()
            );
        }
        if !page.has_next_page {
            break;
        }
        after = Some(
            page.end_cursor
                .ok_or_else(|| ScopeError::Api("stargazers page missing endCursor".into()))?,
        );
    }
    if total == 0 {
        return Ok(vec![]);
    }
    Ok(subsample(daily_cumulative(&timestamps), MAX_DAILY_POINTS))
}

/// Bucket timestamps by UTC day and accumulate.
pub fn daily_cumulative(timestamps: &[i64]) -> Vec<StarPoint> {
    let mut days: std::collections::BTreeMap<i64, u64> = std::collections::BTreeMap::new();
    for ts in timestamps {
        *days.entry(ts / 86_400).or_default() += 1;
    }
    let mut acc = 0u64;
    days.into_iter()
        .map(|(day, n)| {
            acc += n;
            StarPoint {
                ts: day * 86_400,
                count: acc,
            }
        })
        .collect()
}

/// Keep at most `max` points, evenly (always keeps first and last).
pub fn subsample(points: Vec<StarPoint>, max: usize) -> Vec<StarPoint> {
    if points.len() <= max || max < 2 {
        return points;
    }
    let step = points.len().div_ceil(max);
    let mut out: Vec<StarPoint> = points.iter().step_by(step).copied().collect();
    if let Some(&last) = points.last()
        && out.last() != Some(&last)
    {
        out.push(last);
    }
    out
}

/// Parse an RFC 3339 timestamp to unix seconds.
fn parse_ts(s: &str) -> Result<i64> {
    chrono::DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.timestamp())
        .map_err(|e| ScopeError::Api(format!("bad starredAt {s:?}: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daily_cumulative_buckets_and_accumulates() {
        // Two stars on day 0, one on day 2 (unsorted input).
        let pts = daily_cumulative(&[100, 0, 2 * 86_400]);
        assert_eq!(
            pts,
            vec![
                StarPoint { ts: 0, count: 2 },
                StarPoint {
                    ts: 2 * 86_400,
                    count: 3
                },
            ]
        );
    }

    #[test]
    fn subsample_keeps_bounds_within_limit() {
        let pts: Vec<StarPoint> = (0..500)
            .map(|i| StarPoint {
                ts: i * 86_400,
                count: i as u64 + 1,
            })
            .collect();
        let out = subsample(pts.clone(), 200);
        assert!(out.len() <= 200, "{}", out.len());
        assert_eq!(out.first(), pts.first());
        assert_eq!(out.last(), pts.last());
        // Untouched when already small.
        let small = subsample(pts[..10].to_vec(), 200);
        assert_eq!(small.len(), 10);
    }
}
