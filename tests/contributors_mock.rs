//! Integration tests for the contributors wall against a mocked GitHub API.

use httpmock::prelude::*;
use reposcope::contributors;
use reposcope::github::GitHubClient;
use reposcope::svg::{self, WallOptions};

fn contributor(
    server: &MockServer,
    login: &str,
    contributions: u64,
    kind: &str,
) -> serde_json::Value {
    serde_json::json!({
        "login": login,
        "avatar_url": format!("{}/avatars/{login}?v=4", server.base_url()),
        "contributions": contributions,
        "type": kind,
    })
}

async fn mock_contributors_page(server: &MockServer, page: u32, entries: Vec<serde_json::Value>) {
    server
        .mock_async(|when, then| {
            when.method(GET)
                .path("/repos/o/r/contributors")
                .query_param("page", page.to_string());
            then.status(200)
                .json_body(serde_json::Value::Array(entries));
        })
        .await;
}

async fn mock_avatar(server: &MockServer, login: &str, ok: bool) {
    server
        .mock_async(|when, then| {
            when.method(GET).path(format!("/avatars/{login}"));
            if ok {
                then.status(200)
                    .header("content-type", "image/png")
                    .body(b"\x89PNG-fake-image-bytes".as_slice());
            } else {
                then.status(404).body("not found");
            }
        })
        .await;
}

#[tokio::test]
async fn wall_fetch_filters_bots_preserves_order_embeds_avatars() {
    let server = MockServer::start_async().await;
    // Page 1: 100 entries (full page → pagination continues); page 2: 3 (short → stop).
    let p1: Vec<serde_json::Value> = (0..100)
        .map(|i| contributor(&server, &format!("user{i:03}"), 500 - i, "User"))
        .collect();
    mock_contributors_page(&server, 1, p1).await;
    mock_contributors_page(
        &server,
        2,
        vec![
            contributor(&server, "alice", 42, "User"),
            contributor(&server, "dependabot[bot]", 40, "Bot"),
            contributor(&server, "bob", 7, "User"),
        ],
    )
    .await;
    // Only the two tail users get avatar mocks; userNNN avatars are not mocked
    // (httpmock returns 501 for unmatched requests) → they exercise the
    // placeholder fallback too. Give bob a real avatar, alice a 404.
    mock_avatar(&server, "alice", false).await;
    mock_avatar(&server, "bob", true).await;

    let client = GitHubClient::with_base(server.base_url(), "t", 1).unwrap();
    let wall = contributors::fetch_wall(&client, "o/r", 200, false, 64)
        .await
        .unwrap();
    assert_eq!(wall.len(), 102, "100 users + alice + bob (bot excluded)");
    // API order preserved (contributions desc).
    assert_eq!(wall[0].login, "user000");
    assert_eq!(wall[100].login, "alice");
    assert_eq!(wall[101].login, "bob");
    assert!(!wall.iter().any(|e| e.login.contains("bot")));
    // Avatar embedding + placeholder fallback.
    assert!(
        wall[101]
            .avatar
            .as_ref()
            .unwrap()
            .starts_with("data:image/png;base64,")
    );
    assert!(wall[100].avatar.is_none(), "404 avatar → placeholder");

    let opts = WallOptions {
        cols: 12,
        avatar_size: 64,
    };
    let a = svg::render_contributor_wall(&wall, &opts);
    let b = svg::render_contributor_wall(&wall, &opts);
    assert_eq!(a, b, "wall render must be deterministic");
    roxmltree::Document::parse(&a).unwrap();
    assert!(a.contains("data:image/png;base64,"));
    assert!(a.contains("<title>@bob (7 contributions)</title>"));
    assert!(
        a.contains(">A</text>"),
        "alice's 404 avatar → placeholder letter A"
    );
}

#[tokio::test]
async fn include_bots_flag() {
    let server = MockServer::start_async().await;
    mock_contributors_page(
        &server,
        1,
        vec![
            contributor(&server, "alice", 42, "User"),
            contributor(&server, "dependabot[bot]", 40, "Bot"),
        ],
    )
    .await;
    mock_avatar(&server, "alice", true).await;
    mock_avatar(&server, "dependabot[bot]", true).await;

    let client = GitHubClient::with_base(server.base_url(), "t", 1).unwrap();
    let with_bots = contributors::fetch_wall(&client, "o/r", 100, true, 64)
        .await
        .unwrap();
    assert_eq!(with_bots.len(), 2);
    assert_eq!(with_bots[1].login, "dependabot[bot]");
}

#[tokio::test]
async fn empty_contributors_wall() {
    let opts = WallOptions {
        cols: 12,
        avatar_size: 64,
    };
    let svg_text = svg::render_contributor_wall(&[], &opts);
    assert!(svg_text.contains("No contributors"));
    roxmltree::Document::parse(&svg_text).unwrap();
}
