//! Golden-file tests pinning rendered SVG output (spec 00 §6.3).
//! Regenerate with: REPOSCOPE_UPDATE_GOLDEN=1 cargo test --test golden

use reposcope::contributors::WallEntry;
use reposcope::star_history::StarPoint;
use reposcope::svg::{self, Theme, WallOptions};

fn assert_golden(name: &str, actual: &str) {
    let path = format!("{}/tests/golden/{name}", env!("CARGO_MANIFEST_DIR"));
    if std::env::var("REPOSCOPE_UPDATE_GOLDEN").is_ok() {
        std::fs::write(&path, actual).unwrap();
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_else(|_| {
        panic!("golden {name} missing; run REPOSCOPE_UPDATE_GOLDEN=1 cargo test --test golden")
    });
    assert_eq!(actual, expected, "golden mismatch for {name}");
    roxmltree::Document::parse(actual).unwrap();
}

fn fixed_series() -> Vec<StarPoint> {
    [
        (1_600_000_000, 1),
        (1_600_086_400, 3),
        (1_600_172_800, 9),
        (1_600_345_600, 20),
        (1_600_604_800, 42),
        (1_601_036_800, 42),
        (1_601_382_400, 108),
    ]
    .iter()
    .map(|&(ts, count)| StarPoint { ts, count })
    .collect()
}

#[test]
fn star_chart_light_golden() {
    assert_golden(
        "star-light.svg",
        &svg::render_star_chart(&fixed_series(), "owner/repo", &Theme::light()),
    );
}

#[test]
fn star_chart_dark_golden() {
    assert_golden(
        "star-dark.svg",
        &svg::render_star_chart(&fixed_series(), "owner/repo", &Theme::dark()),
    );
}

#[test]
fn contributor_wall_golden() {
    let entries = vec![
        WallEntry {
            login: "alice".into(),
            contributions: 42,
            avatar: Some("data:image/png;base64,ZmFrZS1wbmc=".into()),
        },
        WallEntry {
            login: "bob".into(),
            contributions: 7,
            avatar: None,
        },
    ];
    assert_golden(
        "wall.svg",
        &svg::render_contributor_wall(
            &entries,
            &WallOptions {
                cols: 4,
                avatar_size: 48,
            },
        ),
    );
}
