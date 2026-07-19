//! String-built SVG rendering: star-history line/area chart (spec 00 §6.1).
//! No chart library; output is deterministic (fixed-precision floats,
//! sequential IDs, no timestamps of generation).

use crate::contributors::WallEntry;
use crate::star_history::StarPoint;

const W: u32 = 800;
const H: u32 = 533;
const PAD_L: f64 = 90.0;
const PAD_R: f64 = 35.0;
const PAD_T: f64 = 80.0;
const PAD_B: f64 = 70.0;
const FONT: &str = "-apple-system,'Segoe UI',Roboto,Helvetica,Arial,sans-serif";

/// Chart color palette.
pub struct Theme {
    pub bg: &'static str,
    pub text: &'static str,
    pub subtext: &'static str,
    pub grid: &'static str,
    pub axis: &'static str,
    pub line: &'static str,
}

impl Theme {
    pub fn light() -> Self {
        Self {
            bg: "#ffffff",
            text: "#1f2328",
            subtext: "#59636e",
            grid: "#e5e8eb",
            axis: "#8c959f",
            line: "#2563eb",
        }
    }
    pub fn dark() -> Self {
        Self {
            bg: "#0d1117",
            text: "#e6edf3",
            subtext: "#9198a1",
            grid: "#21262d",
            axis: "#3d444d",
            line: "#60a5fa",
        }
    }
}

/// Render the star-history chart. `points` must be sorted by `ts`.
pub fn render_star_chart(points: &[StarPoint], repo: &str, theme: &Theme) -> String {
    let plot_w = W as f64 - PAD_L - PAD_R;
    let plot_h = H as f64 - PAD_T - PAD_B;
    let mut s = String::with_capacity(4096);
    s.push_str(&format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {W} {H}" width="100%" height="auto" role="img" aria-label="Star history for {}">"#,
        escape_xml(repo)
    ));
    s.push_str(&format!("<style>text{{font-family:{FONT};}}</style>"));
    s.push_str(&format!(
        r#"<rect width="{W}" height="{H}" fill="{}"/>"#,
        theme.bg
    ));
    s.push_str(&format!(
        r#"<defs><linearGradient id="g0" x1="0" y1="0" x2="0" y2="1"><stop offset="0%" stop-color="{}" stop-opacity="0.22"/><stop offset="100%" stop-color="{}" stop-opacity="0.02"/></linearGradient></defs>"#,
        theme.line, theme.line
    ));
    // Legend: color swatch + repo name.
    s.push_str(&format!(
        r#"<rect x="{PAD_L:.2}" y="24.00" width="12" height="12" rx="3" fill="{}"/><text x="{:.2}" y="34.00" font-size="14" fill="{}">{}</text>"#,
        theme.line,
        PAD_L + 20.0,
        theme.text,
        escape_xml(repo)
    ));

    if points.is_empty() {
        s.push_str(&format!(
            r#"<text x="{:.2}" y="{:.2}" font-size="16" text-anchor="middle" fill="{}">No stars yet</text>"#,
            W as f64 / 2.0,
            H as f64 / 2.0,
            theme.subtext
        ));
        s.push_str("</svg>");
        return s;
    }

    let t0 = points.first().unwrap().ts;
    let t1 = points.last().unwrap().ts;
    let max_count = points.iter().map(|p| p.count).max().unwrap_or(0);
    // Y domain: nice-number ticks covering max (spec: floor of 25).
    let ymax_raw = max_count.max(25) as f64;
    let ystep = nice_step(ymax_raw / 4.0);
    let y_max = (ystep * (ymax_raw / ystep).ceil()).max(ystep);

    let fx = |ts: i64| -> f64 {
        if t1 == t0 {
            PAD_L + plot_w / 2.0
        } else {
            PAD_L + (ts - t0) as f64 / (t1 - t0) as f64 * plot_w
        }
    };
    let fy = |c: f64| -> f64 { PAD_T + plot_h - c / y_max * plot_h };

    // Horizontal gridlines + y tick labels.
    let mut v = 0.0f64;
    while v <= y_max + f64::EPSILON {
        let y = fy(v);
        if v == 0.0 {
            s.push_str(&format!(
                r#"<line x1="{PAD_L:.2}" y1="{y:.2}" x2="{:.2}" y2="{y:.2}" stroke="{}" stroke-width="1"/>"#,
                W as f64 - PAD_R,
                theme.axis
            ));
        } else {
            s.push_str(&format!(
                r#"<line x1="{PAD_L:.2}" y1="{y:.2}" x2="{:.2}" y2="{y:.2}" stroke="{}" stroke-width="1"/>"#,
                W as f64 - PAD_R,
                theme.grid
            ));
        }
        s.push_str(&format!(
            r#"<text x="{:.2}" y="{:.2}" font-size="12" text-anchor="end" fill="{}">{}</text>"#,
            PAD_L - 8.0,
            y + 4.0,
            theme.subtext,
            format_count(v as u64)
        ));
        v += ystep;
    }

    // X tick labels (adaptive step, deduped labels).
    for (x, label) in x_ticks(t0, t1, 6) {
        s.push_str(&format!(
            r#"<text x="{:.2}" y="{:.2}" font-size="12" text-anchor="middle" fill="{}">{}</text>"#,
            fx(x),
            PAD_T + plot_h + 24.0,
            theme.subtext,
            label
        ));
    }

    // Series: smoothed line + gradient area.
    let px: Vec<(f64, f64)> = points
        .iter()
        .map(|p| (fx(p.ts), fy(p.count as f64)))
        .collect();
    let line_path = smooth_path(&px);
    let base = fy(0.0);
    let (x0, y0) = px[0];
    let (xn, _) = px[px.len() - 1];
    s.push_str(&format!(
        r#"<path d="{line_path} L {xn:.2} {base:.2} L {x0:.2} {base:.2} L {x0:.2} {y0:.2} Z" fill="url(#g0)"/>"#
    ));
    s.push_str(&format!(
        r#"<path d="{line_path}" fill="none" stroke="{}" stroke-width="3" stroke-linecap="round" stroke-linejoin="round"/>"#,
        theme.line
    ));

    // End dot + final count label, clamped inside the canvas.
    let (ex, ey) = px[px.len() - 1];
    let label = format_count(max_count);
    let (lx, anchor) = if ex + 12.0 + label.len() as f64 * 8.0 > W as f64 - PAD_R {
        (ex - 12.0, "end")
    } else {
        (ex + 12.0, "start")
    };
    let ly = (ey - 8.0).max(PAD_T + 12.0);
    s.push_str(&format!(
        r#"<circle cx="{ex:.2}" cy="{ey:.2}" r="5" fill="{}"/><text x="{lx:.2}" y="{ly:.2}" font-size="14" font-weight="bold" text-anchor="{anchor}" fill="{}">{label}</text>"#,
        theme.line, theme.line
    ));
    s.push_str("</svg>");
    s
}

/// Nice-number axis step: 1/2/2.5/5 × 10^k covering `raw`.
fn nice_step(raw: f64) -> f64 {
    if raw <= 0.0 || !raw.is_finite() {
        return 1.0;
    }
    let mag = 10f64.powi(raw.log10().floor() as i32);
    let norm = raw / mag;
    let nice = if norm <= 1.0 {
        1.0
    } else if norm <= 2.0 {
        2.0
    } else if norm <= 2.5 {
        2.5
    } else if norm <= 5.0 {
        5.0
    } else {
        10.0
    };
    nice * mag
}

/// Compact count formatting: `999`, `1K`, `12.5K`, `1M`, `1.5M`.
pub(crate) fn format_count(n: u64) -> String {
    fn trimmed(v: f64, suffix: &str) -> String {
        let t = format!("{v:.1}");
        format!("{}{}", t.strip_suffix(".0").unwrap_or(&t), suffix)
    }
    if n >= 1_000_000 {
        trimmed(n as f64 / 1e6, "M")
    } else if n >= 1_000 {
        trimmed(n as f64 / 1e3, "K")
    } else {
        n.to_string()
    }
}

/// Adaptive x-axis ticks: up to ~`target` ticks, snapped to a time-step
/// ladder, labels deduped. Returns `(ts, label)` pairs.
fn x_ticks(t0: i64, t1: i64, target: i64) -> Vec<(i64, String)> {
    const LADDER: &[i64] = &[
        86_400,     // day
        604_800,    // week
        2_592_000,  // ~month
        7_884_000,  // ~quarter
        15_768_000, // ~half year
        31_536_000, // year
        63_072_000,
        157_680_000,
        315_360_000,
        630_720_000,
        1_576_800_000,
    ];
    let span = (t1 - t0).max(0);
    let step = LADDER
        .iter()
        .copied()
        .find(|s| span / s <= target)
        .unwrap_or(*LADDER.last().unwrap());
    let fmt = if step < 2_592_000 {
        "%b %d"
    } else if step < 31_536_000 {
        "%b %Y"
    } else {
        "%Y"
    };
    let mut out: Vec<(i64, String)> = Vec::new();
    let mut t = (t0 + step - 1) / step * step;
    while t <= t1 {
        let label = chrono::DateTime::from_timestamp(t, 0)
            .map(|dt| dt.format(fmt).to_string())
            .unwrap_or_default();
        if out.last().map(|(_, l)| l != &label).unwrap_or(true) {
            out.push((t, label));
        }
        t += step;
    }
    out
}

/// Catmull-Rom → cubic Bézier smoothing.
/// With a single point, emits a degenerate `M x y` (the end dot marks it).
fn smooth_path(pts: &[(f64, f64)]) -> String {
    let mut d = format!("M {:.2} {:.2}", pts[0].0, pts[0].1);
    for i in 0..pts.len() - 1 {
        let p0 = pts[i.saturating_sub(1)];
        let p1 = pts[i];
        let p2 = pts[i + 1];
        let p3 = pts[(i + 2).min(pts.len() - 1)];
        let c1 = (p1.0 + (p2.0 - p0.0) / 6.0, p1.1 + (p2.1 - p0.1) / 6.0);
        let c2 = (p2.0 - (p3.0 - p1.0) / 6.0, p2.1 - (p3.1 - p1.1) / 6.0);
        d.push_str(&format!(
            " C {:.2} {:.2} {:.2} {:.2} {:.2} {:.2}",
            c1.0, c1.1, c2.0, c2.1, p2.0, p2.1
        ));
    }
    d
}

/// Wall layout options.
pub struct WallOptions {
    pub cols: u32,
    pub avatar_size: u32,
}

/// Render the contributors avatar wall (spec 00 §6.2). Transparent
/// background; intrinsic size with shrink-to-fit. Deterministic.
pub fn render_contributor_wall(entries: &[WallEntry], opts: &WallOptions) -> String {
    const GAP: u32 = 8;
    const PAD: u32 = 16;
    let cols = opts.cols.max(1);
    let size = opts.avatar_size.max(8);
    if entries.is_empty() {
        return String::from(
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 320 64" width="320" height="64" role="img" aria-label="Contributors"><text x="160" y="36" font-size="14" text-anchor="middle" fill="#8c959f">No contributors</text></svg>"##,
        );
    }
    let n = entries.len() as u32;
    let rows = n.div_ceil(cols);
    let w = 2 * PAD + cols * size + (cols - 1) * GAP;
    let h = 2 * PAD + rows * size + (rows - 1) * GAP;
    let r = size / 2;
    let mut s = String::with_capacity(1024 * entries.len());
    s.push_str(&format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {w} {h}" width="{w}" height="{h}" style="max-width:100%;height:auto" role="img" aria-label="Contributors">"#
    ));
    s.push_str(&format!("<style>text{{font-family:{FONT};}}</style>"));
    // Clip paths (sequential IDs — deterministic).
    s.push_str("<defs>");
    for i in 0..n {
        let (cx, cy) = cell_center(i, cols, size, GAP, PAD);
        s.push_str(&format!(
            r#"<clipPath id="c{i}"><circle cx="{cx}" cy="{cy}" r="{r}"/></clipPath>"#
        ));
    }
    s.push_str("</defs>");
    for (i, e) in entries.iter().enumerate() {
        let i = i as u32;
        let (cx, cy) = cell_center(i, cols, size, GAP, PAD);
        let (x0, y0) = (cx - r, cy - r);
        let title = format!("@{} ({} contributions)", e.login, e.contributions);
        s.push_str("<g>");
        match &e.avatar {
            Some(uri) => {
                s.push_str(&format!(
                    r#"<image href="{uri}" x="{x0}" y="{y0}" width="{size}" height="{size}" preserveAspectRatio="xMidYMid slice" clip-path="url(#c{i})"/>"#
                ));
            }
            None => {
                let letter = e
                    .login
                    .chars()
                    .find(|c| c.is_ascii_alphanumeric())
                    .map(|c| c.to_ascii_uppercase())
                    .unwrap_or('?');
                s.push_str(&format!(
                    r##"<circle cx="{cx}" cy="{cy}" r="{r}" fill="#9ea7b3"/><text x="{cx}" y="{cy}" font-size="{}" font-weight="bold" text-anchor="middle" dominant-baseline="central" fill="#ffffff">{letter}</text>"##,
                    size * 45 / 100
                ));
            }
        }
        s.push_str(&format!("<title>{}</title>", escape_xml(&title)));
        s.push_str("</g>");
    }
    s.push_str("</svg>");
    s
}

fn cell_center(i: u32, cols: u32, size: u32, gap: u32, pad: u32) -> (u32, u32) {
    let (col, row) = (i % cols, i / cols);
    (
        pad + col * (size + gap) + size / 2,
        pad + row * (size + gap) + size / 2,
    )
}

/// Escape XML text/attribute content.
pub(crate) fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pts(v: &[(i64, u64)]) -> Vec<StarPoint> {
        v.iter()
            .map(|&(ts, count)| StarPoint { ts, count })
            .collect()
    }

    #[test]
    fn count_formatting() {
        assert_eq!(format_count(0), "0");
        assert_eq!(format_count(999), "999");
        assert_eq!(format_count(1_000), "1K");
        assert_eq!(format_count(12_000), "12K");
        assert_eq!(format_count(12_500), "12.5K");
        assert_eq!(format_count(1_000_000), "1M");
        assert_eq!(format_count(1_500_000), "1.5M");
    }

    #[test]
    fn nice_steps() {
        assert_eq!(nice_step(0.0), 1.0);
        assert_eq!(nice_step(0.9), 1.0);
        assert_eq!(nice_step(1.5), 2.0);
        assert_eq!(nice_step(2.25), 2.5);
        assert_eq!(nice_step(3.0), 5.0);
        assert_eq!(nice_step(7.0), 10.0);
        assert_eq!(nice_step(121.75), 200.0);
        assert_eq!(nice_step(26_250.0), 50_000.0);
    }

    #[test]
    fn xml_escaping() {
        assert_eq!(escape_xml("a<&>\"'"), "a&lt;&amp;&gt;&quot;&#39;");
    }

    #[test]
    fn render_is_deterministic_and_valid() {
        let p = pts(&[
            (0, 1),
            (86_400, 3),
            (172_800, 10),
            (259_200, 10),
            (345_600, 42),
        ]);
        let a = render_star_chart(&p, "owner/name", &Theme::light());
        let b = render_star_chart(&p, "owner/name", &Theme::light());
        assert_eq!(a, b);
        roxmltree::Document::parse(&a).unwrap();
        assert!(a.contains("owner/name"));
        assert!(!a.contains("NaN"));
    }

    #[test]
    fn render_edge_cases() {
        // Empty: empty-state text, still valid SVG.
        let empty = render_star_chart(&[], "o/r", &Theme::dark());
        assert!(empty.contains("No stars yet"));
        roxmltree::Document::parse(&empty).unwrap();
        // Single point: no NaN/inf, valid.
        let one = render_star_chart(&pts(&[(1_700_000_000, 5)]), "o/r", &Theme::light());
        assert!(!one.contains("NaN") && !one.contains("inf"));
        roxmltree::Document::parse(&one).unwrap();
        // Escaped repo name.
        let esc = render_star_chart(&pts(&[(0, 1), (86_400, 2)]), "o<&/r", &Theme::light());
        assert!(esc.contains("o&lt;&amp;/r"));
        roxmltree::Document::parse(&esc).unwrap();
    }

    #[test]
    fn x_tick_labels_dedup() {
        // ~2.5 years span → yearly labels, no duplicates.
        let ticks = x_ticks(0, 900 * 86_400, 6);
        let labels: Vec<&str> = ticks.iter().map(|(_, l)| l.as_str()).collect();
        let mut dedup = labels.clone();
        dedup.dedup();
        assert_eq!(labels, dedup, "{labels:?}");
        assert!(!ticks.is_empty());
    }
}
