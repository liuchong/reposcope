//! Integration tests for the star-history pipeline against a mocked GitHub
//! GraphQL API (`POST /graphql`).

use httpmock::prelude::*;
use reposcope::github::GitHubClient;
use reposcope::star_history::{self, StarPoint};

const BASE_TS: i64 = 1_700_000_000; // 2023-11-14T22:13:20Z

fn rfc3339(ts: i64) -> String {
    chrono::DateTime::from_timestamp(ts, 0)
        .unwrap()
        .format("%Y-%m-%dT%H:%M:%SZ")
        .to_string()
}

/// Mock one GraphQL stargazers page. `after_marker` distinguishes pages by
/// the request body (`"after":null` for the first page).
async fn mock_page(
    server: &MockServer,
    after_marker: &str,
    total: u64,
    timestamps: &[i64],
    next_cursor: Option<&str>,
) {
    let edges: Vec<serde_json::Value> = timestamps
        .iter()
        .map(|ts| serde_json::json!({ "starredAt": rfc3339(*ts) }))
        .collect();
    let body = serde_json::json!({
        "data": {
            "repository": {
                "stargazers": {
                    "totalCount": total,
                    "pageInfo": {
                        "hasNextPage": next_cursor.is_some(),
                        "endCursor": next_cursor,
                    },
                    "edges": edges,
                }
            },
            "rateLimit": { "remaining": 4999, "resetAt": "2099-01-01T00:00:00Z" },
        }
    });
    server
        .mock_async(|when, then| {
            when.method(POST)
                .path("/graphql")
                .body_includes(after_marker);
            then.status(200).json_body(body);
        })
        .await;
}

#[tokio::test]
async fn multi_page_repo() {
    let server = MockServer::start_async().await;
    // 230 stars, one per day → pages of 100/100/30.
    let p1: Vec<i64> = (0..100).map(|d| BASE_TS + d * 86_400).collect();
    let p2: Vec<i64> = (100..200).map(|d| BASE_TS + d * 86_400).collect();
    let p3: Vec<i64> = (200..230).map(|d| BASE_TS + d * 86_400).collect();
    mock_page(&server, "\"after\":null", 230, &p1, Some("cursor-1")).await;
    mock_page(
        &server,
        "\"after\":\"cursor-1\"",
        230,
        &p2,
        Some("cursor-2"),
    )
    .await;
    mock_page(&server, "\"after\":\"cursor-2\"", 230, &p3, None).await;

    let client = GitHubClient::with_base(server.base_url(), "t", 1).unwrap();
    let series = star_history::fetch_star_series(&client, "o/r")
        .await
        .unwrap();
    assert_eq!(
        series.first().unwrap(),
        &StarPoint {
            ts: (BASE_TS / 86_400) * 86_400,
            count: 1
        }
    );
    assert_eq!(series.last().unwrap().count, 230);
    assert!(series.len() <= 200, "{} points", series.len());
    for w in series.windows(2) {
        assert!(w[0].ts < w[1].ts && w[0].count < w[1].count, "{series:?}");
    }
}

#[tokio::test]
async fn empty_repo() {
    let server = MockServer::start_async().await;
    mock_page(&server, "\"after\":null", 0, &[], None).await;
    let client = GitHubClient::with_base(server.base_url(), "t", 1).unwrap();
    let series = star_history::fetch_star_series(&client, "o/r")
        .await
        .unwrap();
    assert!(series.is_empty());
}

#[tokio::test]
async fn repo_not_found() {
    let server = MockServer::start_async().await;
    server
        .mock_async(|when, then| {
            when.method(POST).path("/graphql");
            then.status(200).json_body(serde_json::json!({
                "data": { "repository": null, "rateLimit": { "remaining": 4999, "resetAt": "2099-01-01T00:00:00Z" } }
            }));
        })
        .await;
    let client = GitHubClient::with_base(server.base_url(), "t", 1).unwrap();
    let err = star_history::fetch_star_series(&client, "o/r")
        .await
        .unwrap_err();
    assert!(err.to_string().contains("repository not found"), "{err}");
}

#[tokio::test]
async fn graphql_error_surfaces_message() {
    let server = MockServer::start_async().await;
    server
        .mock_async(|when, then| {
            when.method(POST).path("/graphql");
            then.status(200).json_body(serde_json::json!({
                "errors": [{ "message": "Something went wrong", "type": "INTERNAL" }]
            }));
        })
        .await;
    let client = GitHubClient::with_base(server.base_url(), "t", 1).unwrap();
    let err = star_history::fetch_star_series(&client, "o/r")
        .await
        .unwrap_err();
    assert!(err.to_string().contains("Something went wrong"), "{err}");
}

#[tokio::test]
async fn double_run_is_byte_identical() {
    let server = MockServer::start_async().await;
    // Clump several stars per day to exercise bucketing; 2 pages.
    let p1: Vec<i64> = (0..100)
        .map(|i| BASE_TS + i64::from(i / 7) * 86_400)
        .collect();
    let p2: Vec<i64> = (100..187)
        .map(|i| BASE_TS + i64::from(i / 7) * 86_400)
        .collect();
    mock_page(&server, "\"after\":null", 187, &p1, Some("cursor-1")).await;
    mock_page(&server, "\"after\":\"cursor-1\"", 187, &p2, None).await;

    let client = GitHubClient::with_base(server.base_url(), "t", 1).unwrap();
    let a = star_history::fetch_star_series(&client, "o/r")
        .await
        .unwrap();
    let b = star_history::fetch_star_series(&client, "o/r")
        .await
        .unwrap();
    assert_eq!(a, b);
    let s1 = reposcope::svg::render_star_chart(&a, "o/r", &reposcope::svg::Theme::light());
    let s2 = reposcope::svg::render_star_chart(&b, "o/r", &reposcope::svg::Theme::light());
    assert_eq!(s1, s2);
    roxmltree::Document::parse(&s1).unwrap();
    // Endpoint rule: the chart ends at the most recent star's day, count 187.
    assert_eq!(a.last().unwrap().count, 187);
    assert_eq!(a.last().unwrap().ts, (p2[86] / 86_400) * 86_400);
}
