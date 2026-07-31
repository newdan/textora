//! SVG icon renderer with pre-loaded Lucide icons.
//!
//! Adding a new icon:
//!   1. Place `name.svg` in `~/Downloads/icons/`
//!   2. Add `"name"` to `ICONS` in `scripts/gen_icons.py`
//!   3. Run: `python3 scripts/gen_icons.py`
//!
//! Or use the parser directly:
//!   ```ignore
//!   let polys = parse_svg_path("M5 12h14");
//!   // tessellate polys into triangles for drawing
//!   ```
//! Core SVG parser is hand-written; Lucide icon data is auto-generated.
//! Source: Lucide icons (MIT license).

use std::collections::HashMap;
use std::sync::{LazyLock, RwLock};

use crate::core::paint::DrawList;

// ── SVG number tokenizer (handles implicit negatives: 1.5-2.9 → [1.5, -2.9]) ──

pub fn tokenize_svg_nums(s: &str) -> Vec<f32> {
    let b = s.as_bytes();
    let mut nums = Vec::new();
    let mut i = 0;
    let n = b.len();
    while i < n {
        while i < n
            && (b[i] == b' ' || b[i] == b',' || b[i] == b'\n' || b[i] == b'\r' || b[i] == b'\t')
        {
            i += 1;
        }
        if i >= n {
            break;
        }
        let start = i;
        if b[i] == b'+' || b[i] == b'-' {
            i += 1;
        }
        while i < n && b[i].is_ascii_digit() {
            i += 1;
        }
        if i < n && b[i] == b'.' {
            i += 1;
            while i < n && b[i].is_ascii_digit() {
                i += 1;
            }
        }
        if i < n && (b[i] == b'e' || b[i] == b'E') {
            i += 1;
            if i < n && (b[i] == b'+' || b[i] == b'-') {
                i += 1;
            }
            while i < n && b[i].is_ascii_digit() {
                i += 1;
            }
        }
        if i > start
            && let Ok(v) = s[start..i].parse::<f32>()
        {
            nums.push(v);
        }
    }
    nums
}

// ── SVG path parser → polylines ──

pub fn parse_svg_path(d: &str) -> Vec<Vec<[f32; 2]>> {
    let mut out: Vec<Vec<[f32; 2]>> = Vec::new();
    let mut cur = [0.0f32; 2];
    let mut start = [0.0f32; 2];
    let mut pts: Vec<[f32; 2]> = Vec::new();
    let mut last_cmd = b'\0';

    fn flush(pts: &mut Vec<[f32; 2]>, out: &mut Vec<Vec<[f32; 2]>>) {
        if pts.len() >= 2 {
            out.push(pts.clone());
        }
        pts.clear();
    }

    let bytes = d.as_bytes();
    let mut i = 0;
    let n = bytes.len();

    while i < n {
        while i < n
            && (bytes[i] == b' '
                || bytes[i] == b','
                || bytes[i] == b'\n'
                || bytes[i] == b'\r'
                || bytes[i] == b'\t')
        {
            i += 1;
        }
        if i >= n {
            break;
        }

        let cmd;
        if bytes[i].is_ascii_alphabetic() {
            cmd = bytes[i];
            i += 1;
            last_cmd = cmd;
        } else {
            cmd = match last_cmd {
                b'M' => b'L',
                b'm' => b'l',
                b'Z' | b'z' => last_cmd,
                _ => last_cmd,
            };
        }

        let arg_start = i;
        while i < n {
            while i < n
                && (bytes[i] == b' '
                    || bytes[i] == b','
                    || bytes[i] == b'\n'
                    || bytes[i] == b'\r'
                    || bytes[i] == b'\t')
            {
                i += 1;
            }
            if i >= n || bytes[i].is_ascii_alphabetic() {
                break;
            }
            if bytes[i] == b'+' || bytes[i] == b'-' {
                i += 1;
            }
            while i < n && bytes[i].is_ascii_digit() {
                i += 1;
            }
            if i < n && bytes[i] == b'.' {
                i += 1;
                while i < n && bytes[i].is_ascii_digit() {
                    i += 1;
                }
            }
            if i < n && (bytes[i] == b'e' || bytes[i] == b'E') {
                i += 1;
                if i < n && (bytes[i] == b'+' || bytes[i] == b'-') {
                    i += 1;
                }
                while i < n && bytes[i].is_ascii_digit() {
                    i += 1;
                }
            }
        }
        let args = tokenize_svg_nums(&d[arg_start..i]);

        let rel = cmd.is_ascii_lowercase();
        match cmd | 0x20 {
            b'm' => {
                flush(&mut pts, &mut out);
                for j in (0..args.len()).step_by(2) {
                    let (mut x, mut y) = (args[j], *args.get(j + 1).unwrap_or(&0.0));
                    if rel {
                        x += cur[0];
                        y += cur[1];
                    } else if j == 0 {
                    }
                    if j == 0 {
                        cur = [x, y];
                        start = cur;
                        flush(&mut pts, &mut out);
                        pts.push(cur);
                    } else {
                        cur = [x, y];
                        pts.push(cur);
                    }
                }
            }
            b'l' => {
                for j in (0..args.len()).step_by(2) {
                    let (mut x, mut y) = (args[j], *args.get(j + 1).unwrap_or(&0.0));
                    if rel {
                        x += cur[0];
                        y += cur[1];
                    }
                    cur = [x, y];
                    pts.push(cur);
                }
            }
            b'h' => {
                for &a in &args {
                    let x = if rel { cur[0] + a } else { a };
                    cur = [x, cur[1]];
                    pts.push(cur);
                }
            }
            b'v' => {
                for &a in &args {
                    let y = if rel { cur[1] + a } else { a };
                    cur = [cur[0], y];
                    pts.push(cur);
                }
            }
            b'a' => {
                for j in (0..args.len()).step_by(7) {
                    if j + 6 >= args.len() {
                        break;
                    }
                    let mut rx = args[j].abs();
                    let mut ry = args[j + 1].abs();
                    let phi = args[j + 2] * std::f32::consts::PI / 180.0;
                    let large = args[j + 3] as i32;
                    let sweep_flag = args[j + 4] as i32;
                    let (mut ex, mut ey) = (args[j + 5], args[j + 6]);
                    if rel {
                        ex += cur[0];
                        ey += cur[1];
                    }
                    let (sx, sy) = (cur[0], cur[1]);
                    // Skip degenerate arcs
                    if rx < 0.001
                        || ry < 0.001
                        || ((sx - ex).abs() < 0.001 && (sy - ey).abs() < 0.001)
                    {
                        cur = [ex, ey];
                        pts.push(cur);
                        continue;
                    }
                    // SVG arc endpoint to center parameterization
                    let cos_p = phi.cos();
                    let sin_p = phi.sin();
                    let dx = (sx - ex) / 2.0;
                    let dy = (sy - ey) / 2.0;
                    let x1p = cos_p * dx + sin_p * dy;
                    let y1p = -sin_p * dx + cos_p * dy;
                    // Ensure radii are large enough
                    let lam = (x1p * x1p) / (rx * rx) + (y1p * y1p) / (ry * ry);
                    if lam > 1.0 {
                        let s = lam.sqrt();
                        rx *= s;
                        ry *= s;
                    }
                    let num = ((rx * rx * ry * ry - rx * rx * y1p * y1p - ry * ry * x1p * x1p)
                        / (rx * rx * y1p * y1p + ry * ry * x1p * x1p))
                        .max(0.0)
                        .sqrt();
                    let sign = if large == sweep_flag { -1.0 } else { 1.0 };
                    let cxp = sign * num * rx * y1p / ry;
                    let cyp = -sign * num * ry * x1p / rx;
                    let cx = cos_p * cxp - sin_p * cyp + (sx + ex) / 2.0;
                    let cy = sin_p * cxp + cos_p * cyp + (sy + ey) / 2.0;
                    // Angles
                    fn vec_angle(ux: f32, uy: f32, vx: f32, vy: f32) -> f32 {
                        let n = (ux * ux + uy * uy).sqrt() * (vx * vx + vy * vy).sqrt();
                        if n < 1e-15 {
                            return 0.0;
                        }
                        let c = ((ux * vx + uy * vy) / n).clamp(-1.0, 1.0).acos();
                        if ux * vy - uy * vx < 0.0 { -c } else { c }
                    }
                    let theta1 = vec_angle(1.0, 0.0, (x1p - cxp) / rx, (y1p - cyp) / ry);
                    let mut dtheta = vec_angle(
                        (x1p - cxp) / rx,
                        (y1p - cyp) / ry,
                        (-x1p - cxp) / rx,
                        (-y1p - cyp) / ry,
                    );
                    if sweep_flag == 0 && dtheta > 0.0 {
                        dtheta -= 2.0 * std::f32::consts::PI;
                    }
                    if sweep_flag != 0 && dtheta < 0.0 {
                        dtheta += 2.0 * std::f32::consts::PI;
                    }
                    // Sample arc
                    let n_seg = 12;
                    for s in 1..=n_seg {
                        let t = s as f32 / n_seg as f32;
                        let ang = theta1 + dtheta * t;
                        let px = cos_p * rx * ang.cos() - sin_p * ry * ang.sin() + cx;
                        let py = sin_p * rx * ang.cos() + cos_p * ry * ang.sin() + cy;
                        pts.push([px, py]);
                    }
                    cur = [ex, ey];
                }
            }
            b'c' => {
                for j in (0..args.len()).step_by(6) {
                    if j + 5 >= args.len() {
                        break;
                    }
                    let (mut x1, mut y1) = (args[j], args[j + 1]);
                    let (mut x2, mut y2) = (args[j + 2], args[j + 3]);
                    let (mut x, mut y) = (args[j + 4], args[j + 5]);
                    if rel {
                        x1 += cur[0];
                        y1 += cur[1];
                        x2 += cur[0];
                        y2 += cur[1];
                        x += cur[0];
                        y += cur[1];
                    }
                    for s in 1..=8 {
                        let t = s as f32 / 8.0;
                        let mt = 1.0 - t;
                        pts.push([
                            mt * mt * mt * cur[0]
                                + 3.0 * mt * mt * t * x1
                                + 3.0 * mt * t * t * x2
                                + t * t * t * x,
                            mt * mt * mt * cur[1]
                                + 3.0 * mt * mt * t * y1
                                + 3.0 * mt * t * t * y2
                                + t * t * t * y,
                        ]);
                    }
                    cur = [x, y];
                }
            }
            b'z' => {
                if !pts.is_empty() && pts[0] != cur {
                    pts.push(start);
                    cur = start;
                }
                flush(&mut pts, &mut out);
            }
            _ => {}
        }
    }
    flush(&mut pts, &mut out);
    out
}

// ── Icon data (auto-generated by scripts/gen_icons.py) ──

struct IconSvg {
    paths: &'static [&'static str],
    circles: &'static [(f32, f32, f32)],
    stroke_width: f32,
}

const DATA_PLUS: IconSvg =
    IconSvg { paths: &["M5 12h14", "M12 5v14"], circles: &[], stroke_width: 2.0 };

const DATA_FOLDER_OPEN: IconSvg = IconSvg {
    paths: &[
        "m6 14 1.5-2.9A2 2 0 0 1 9.24 10H20a2 2 0 0 1 1.94 2.5l-1.54 6a2 2 0 0 1-1.95 1.5H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h3.9a2 2 0 0 1 1.69.9l.81 1.2a2 2 0 0 0 1.67.9H18a2 2 0 0 1 2 2v2",
    ],
    circles: &[],
    stroke_width: 2.0,
};

const DATA_STAR: IconSvg = IconSvg {
    paths: &[
        "M12 2 15.09 8.26 22 9.27 17 14.14 18.18 21.02 12 17.77 5.82 21.02 7 14.14 2 9.27 8.91 8.26Z",
    ],
    circles: &[],
    stroke_width: 2.0,
};

const DATA_TRASH_2: IconSvg = IconSvg {
    paths: &[
        "M3 6h18",
        "M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6",
        "M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2",
        "M10 11v6",
        "M14 11v6",
    ],
    circles: &[],
    stroke_width: 2.0,
};

const DATA_FILE: IconSvg = IconSvg {
    paths: &["M14.5 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7.5Z", "M14 2v6h6"],
    circles: &[],
    stroke_width: 2.0,
};

const DATA_FILE_TEXT: IconSvg = IconSvg {
    paths: &[
        "M14.5 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7.5Z",
        "M14 2v6h6",
        "M16 13H8",
        "M16 17H8",
        "M10 9H8",
    ],
    circles: &[],
    stroke_width: 2.0,
};

const DATA_NOTEBOOK_PEN: IconSvg = IconSvg {
    paths: &[
        "M13.4 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2v-7.4",
        "M2 6h4",
        "M2 10h4",
        "M2 14h4",
        "M2 18h4",
        "M21.378 5.626a1 1 0 0 0-3.004-3.004l-5.01 5.012a2 2 0 0 0-.506.854l-.837 2.87a.5.5 0 0 0 .62.62l2.87-.837a2 2 0 0 0 .854-.506Z",
    ],
    circles: &[],
    stroke_width: 2.0,
};

const DATA_SETTINGS: IconSvg = IconSvg {
    paths: &[
        "M9.671 4.136a2.34 2.34 0 0 1 4.659 0 2.34 2.34 0 0 0 3.319 1.915 2.34 2.34 0 0 1 2.33 4.033 2.34 2.34 0 0 0 0 3.831 2.34 2.34 0 0 1-2.33 4.033 2.34 2.34 0 0 0-3.319 1.915 2.34 2.34 0 0 1-4.659 0 2.34 2.34 0 0 0-3.32-1.915 2.34 2.34 0 0 1-2.33-4.033 2.34 2.34 0 0 0 0-3.831A2.34 2.34 0 0 1 6.35 6.051a2.34 2.34 0 0 0 3.319-1.915",
    ],
    circles: &[(12.0, 12.0, 3.0)],
    stroke_width: 2.0,
};

const DATA_SEARCH: IconSvg =
    IconSvg { paths: &["m21 21-4.34-4.34"], circles: &[(11.0, 11.0, 8.0)], stroke_width: 2.0 };

const DATA_REGEX: IconSvg = IconSvg {
    paths: &[
        "M17 3v10",
        "m12.67 5.5 8.66 5",
        "m12.67 10.5 8.66-5",
        "M9 17a2 2 0 0 0-2-2H5a2 2 0 0 0-2 2v2a2 2 0 0 0 2 2h2a2 2 0 0 0 2-2v-2z",
    ],
    circles: &[],
    stroke_width: 2.0,
};

const DATA_X: IconSvg =
    IconSvg { paths: &["M18 6 6 18", "m6 6 12 12"], circles: &[], stroke_width: 2.0 };

const DATA_CHEVRON_LEFT: IconSvg =
    IconSvg { paths: &["m15 18-6-6 6-6"], circles: &[], stroke_width: 2.0 };

const DATA_CHEVRON_RIGHT: IconSvg =
    IconSvg { paths: &["m9 18 6-6-6-6"], circles: &[], stroke_width: 2.0 };

const DATA_EYE: IconSvg = IconSvg {
    paths: &[
        "M2.062 12.348a1 1 0 0 1 0-.696 10.75 10.75 0 0 1 19.876 0 1 1 0 0 1 0 .696 10.75 10.75 0 0 1-19.876 0",
    ],
    circles: &[(12.0, 12.0, 3.0)],
    stroke_width: 2.0,
};

const DATA_EYE_OFF: IconSvg = IconSvg {
    paths: &[
        "M10.733 5.076a10.744 10.744 0 0 1 11.205 6.575 1 1 0 0 1 0 .696 10.747 10.747 0 0 1-1.444 2.49",
        "M14.084 14.158a3 3 0 0 1-4.242-4.242",
        "M17.479 17.499a10.75 10.75 0 0 1-15.417-5.151 1 1 0 0 1 0-.696 10.75 10.75 0 0 1 4.446-5.143",
        "m2 2 20 20",
    ],
    circles: &[],
    stroke_width: 2.0,
};

const DATA_REPLACE: IconSvg = IconSvg {
    paths: &[
        "M14 4a1 1 0 0 1 1-1",
        "M15 10a1 1 0 0 1-1-1",
        "M21 4a1 1 0 0 0-1-1",
        "M21 9a1 1 0 0 1-1 1",
        "m3 7 3 3 3-3",
        "M6 10V5a2 2 0 0 1 2-2h2",
    ],
    circles: &[],
    stroke_width: 2.0,
};

const DATA_LIST: IconSvg = IconSvg {
    paths: &["M3 5h.01", "M3 12h.01", "M3 19h.01", "M8 5h13", "M8 12h13", "M8 19h13"],
    circles: &[],
    stroke_width: 2.0,
};

const DATA_LIST_TREE: IconSvg = IconSvg {
    paths: &["M9 3v18", "M9 6h11", "M9 12h11", "M9 18h11"],
    circles: &[(4.0, 6.0, 2.0), (4.0, 12.0, 2.0), (4.0, 18.0, 2.0)],
    stroke_width: 2.0,
};

const DATA_PALETTE: IconSvg = IconSvg {
    paths: &[
        "M12 22a1 1 0 0 1 0-20 10 9 0 0 1 10 9 5 5 0 0 1-5 5h-2.25a1.75 1.75 0 0 0-1.4 2.8l.3.4a1.75 1.75 0 0 1-1.4 2.8z",
        "M7.5 10.5h.01",
        "M10.5 7.5h.01",
        "M14.5 6.5h.01",
        "M17.5 9.5h.01",
    ],
    circles: &[],
    stroke_width: 2.0,
};

fn icon_svg(name: &str) -> Option<&'static IconSvg> {
    match name {
        "plus" => Some(&DATA_PLUS),
        "folder-open" => Some(&DATA_FOLDER_OPEN),
        "star" => Some(&DATA_STAR),
        "trash-2" => Some(&DATA_TRASH_2),
        "file" => Some(&DATA_FILE),
        "file-text" => Some(&DATA_FILE_TEXT),
        "notebook-pen" => Some(&DATA_NOTEBOOK_PEN),
        "settings" => Some(&DATA_SETTINGS),
        "search" => Some(&DATA_SEARCH),
        "regex" => Some(&DATA_REGEX),
        "x" => Some(&DATA_X),
        "chevron-left" => Some(&DATA_CHEVRON_LEFT),
        "chevron-right" => Some(&DATA_CHEVRON_RIGHT),
        "eye" => Some(&DATA_EYE),
        "eye-off" => Some(&DATA_EYE_OFF),
        "replace" => Some(&DATA_REPLACE),
        "list" => Some(&DATA_LIST),
        "list-tree" => Some(&DATA_LIST_TREE),
        "palette" => Some(&DATA_PALETTE),
        _ => None,
    }
}

static ICON_CACHE: LazyLock<RwLock<HashMap<String, Vec<[f32; 6]>>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

fn ensure_icon(name: &str) -> Option<()> {
    {
        let c = ICON_CACHE.read().unwrap();
        if c.contains_key(name) {
            return Some(());
        }
    }
    let svg = icon_svg(name)?;
    let sw = svg.stroke_width;
    let hw = sw * 0.5;
    let mut tris: Vec<[f32; 6]> = Vec::new();
    for &d in svg.paths {
        for poly in &parse_svg_path(d) {
            for w in poly.windows(2) {
                let [x0, y0] = w[0];
                let [x1, y1] = w[1];
                let dx = x1 - x0;
                let dy = y1 - y0;
                let len = (dx * dx + dy * dy).sqrt();
                if len < 0.01 {
                    continue;
                }
                let nx = -dy / len * hw;
                let ny = dx / len * hw;
                tris.push([x0 + nx, y0 + ny, x0 - nx, y0 - ny, x1 - nx, y1 - ny]);
                tris.push([x0 + nx, y0 + ny, x1 - nx, y1 - ny, x1 + nx, y1 + ny]);
            }
        }
    }
    for &(cx, cy, r) in svg.circles {
        let n = 16;
        let inner = r - hw;
        let outer = r + hw;
        for i in 0..n {
            let a0 = std::f32::consts::TAU * i as f32 / n as f32;
            let a1 = std::f32::consts::TAU * (i + 1) as f32 / n as f32;
            let (c0, s0) = (a0.cos(), a0.sin());
            let (c1, s1) = (a1.cos(), a1.sin());
            tris.push([
                cx + inner * c0,
                cy + inner * s0,
                cx + outer * c0,
                cy + outer * s0,
                cx + outer * c1,
                cy + outer * s1,
            ]);
            tris.push([
                cx + inner * c0,
                cy + inner * s0,
                cx + outer * c1,
                cy + outer * s1,
                cx + inner * c1,
                cy + inner * s1,
            ]);
        }
    }
    let mut c = ICON_CACHE.write().unwrap();
    c.insert(name.to_string(), tris);
    Some(())
}

/// Draw a named Lucide icon at (x, y) with given logical size and color.
pub fn draw_icon(list: &mut DrawList, name: &str, x: f32, y: f32, size: f32, color: [f32; 4]) {
    if ensure_icon(name).is_none() {
        return;
    }
    let c = ICON_CACHE.read().unwrap();
    let tris = match c.get(name) {
        Some(t) => t,
        None => return,
    };
    let scale = size / 24.0;
    for t in tris {
        list.fill_triangle(
            [x + t[0] * scale, y + t[1] * scale],
            [x + t[2] * scale, y + t[3] * scale],
            [x + t[4] * scale, y + t[5] * scale],
            color,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenize_implicit_negative() {
        assert_eq!(tokenize_svg_nums("1.5-2.9"), vec![1.5, -2.9]);
    }

    #[test]
    fn tokenize_implicit_decimal() {
        assert_eq!(tokenize_svg_nums("10.5.8"), vec![10.5, 0.8]);
    }

    #[test]
    fn tokenize_scientific() {
        assert_eq!(tokenize_svg_nums("1e3"), vec![1000.0]);
    }

    #[test]
    fn tokenize_mixed() {
        let nums = tokenize_svg_nums("0 0 1 0-.696");
        assert_eq!(nums, vec![0.0, 0.0, 1.0, 0.0, -0.696]);
    }

    #[test]
    fn parse_moveto_lineto() {
        let polys = parse_svg_path("M5 12h14");
        assert_eq!(polys.len(), 1);
        assert_eq!(polys[0][0], [5.0, 12.0]);
        assert_eq!(polys[0][1], [19.0, 12.0]);
    }

    #[test]
    fn parse_implicit_repeat() {
        let polys = parse_svg_path("M5 12 10 15");
        assert_eq!(polys.len(), 1);
        assert_eq!(polys[0].len(), 2);
        assert_eq!(polys[0][1], [10.0, 15.0]);
    }

    #[test]
    fn parse_close_path() {
        let polys = parse_svg_path("M0 0 L10 0 L10 10 Z");
        assert_eq!(polys.len(), 1);
        assert_eq!(polys[0].last(), Some(&[0.0, 0.0]));
    }

    #[test]
    fn parse_relative() {
        let polys = parse_svg_path("M5 5 l3 3");
        assert_eq!(polys[0][1], [8.0, 8.0]);
    }

    #[test]
    fn parse_empty() {
        let polys = parse_svg_path("");
        assert!(polys.is_empty());
    }

    #[test]
    fn icon_svg_known_names() {
        for icon_name in [
            "plus",
            "search",
            "eye",
            "eye-off",
            "star",
            "trash-2",
            "file",
            "file-text",
            "notebook-pen",
        ] {
            assert!(icon_svg(icon_name).is_some(), "{icon_name} should be registered");
            assert!(ensure_icon(icon_name).is_some(), "{icon_name} should tessellate");
            let cache = ICON_CACHE.read().expect("icon cache lock should remain available");
            assert!(
                cache.get(icon_name).is_some_and(|triangles| !triangles.is_empty()),
                "{icon_name} should produce drawable triangles"
            );
        }
        assert!(icon_svg("nonexistent").is_none());
    }

    #[test]
    fn palette_icon_is_registered_and_tessellates() {
        assert!(icon_svg("palette").is_some());
        assert!(ensure_icon("palette").is_some());
    }

    #[test]
    fn ensure_icon_caches() {
        assert!(ensure_icon("plus").is_some());
        assert!(ensure_icon("plus").is_some());
        assert!(ensure_icon("nonexistent").is_none());
    }

    #[test]
    fn eye_and_eye_off_differ() {
        let eye = icon_svg("eye").unwrap();
        let off = icon_svg("eye-off").unwrap();
        assert!(eye.paths != off.paths, "eye and eye-off should have different paths");
    }

    #[test]
    fn draw_icon_unknown_is_noop() {
        let mut dl = DrawList::new();
        draw_icon(&mut dl, "nonexistent", 0.0, 0.0, 14.0, [1.0, 1.0, 1.0, 1.0]);
        assert!(dl.cmds.is_empty());
    }
}
