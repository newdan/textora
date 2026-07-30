#!/usr/bin/env python3
"""
SVG 图标生成器 — 将 Lucide SVG 图标转换为 Rust 渲染代码

用法:
  python3 scripts/gen_icons.py                        # 生成默认图标列表
  python3 scripts/gen_icons.py --icons plus,search    # 只生成指定图标
  python3 scripts/gen_icons.py --src /path/to/icons   # 指定 SVG 源目录
  python3 scripts/gen_icons.py --dry-run              # 预览不写文件

工作原理:
  1. 读取 SVG 文件，提取 <path> 和 <circle> 元素
  2. 输出 Rust 源码:
     - tokenize_svg_nums() + parse_svg_path() — SVG 解析器 (pub)
     - struct IconSvg — 图标数据结构
     - const DATA_NAME: IconSvg — 每个图标的硬编码常量
     - fn icon_svg(name) — match 查找
     - ICON_CACHE (RwLock) + ensure_icon() — 懒加载三角形缓存
     - draw_icon() — 绘制入口 (pub)
     - #[cfg(test)] mod tests — 单元测试

添加新图标:
  1. 下载 Lucide SVG 文件到 --src 目录（默认 ~/Downloads/icons/）
  2. 在下方 ICONS 列表中加入图标名（文件名去掉 .svg）
  3. 运行: python3 scripts/gen_icons.py
  4. 在 Rust 代码中调用: draw_icon(&mut list, "图标名", x, y, size, color)

输出文件: crates/ui/src/widgets/icon.rs
依赖: Python 3.8+（标准库，无需额外安装）
SVG 源: Lucide 图标集 (MIT) — https://lucide.dev
"""

import os
import sys
import argparse
import xml.etree.ElementTree as ET

# ============================================================
# 配置
# ============================================================

DEFAULT_SRC = os.path.expanduser("~/Downloads/icons")

ICONS = [
    "plus",
    "folder-open",
    "settings",
    "search",
    "regex",
    "x",
    "chevron-left",
    "chevron-right",
    "eye",
    "eye-off",
    "replace",
    "list",
]

# ============================================================
# SVG 解析
# ============================================================

def extract_svg(path: str):
    """从 SVG 文件提取路径、圆形和笔画宽度。"""
    tree = ET.parse(path)
    root = tree.getroot()
    ns = "http://www.w3.org/2000/svg"
    sw = float(root.get("stroke-width", "2"))
    paths, circles = [], []
    for el in root.iter():
        tag = el.tag.replace(f"{{{ns}}}", "")
        if tag == "path":
            d = el.get("d", "")
            if d:
                paths.append(d)
        elif tag == "circle":
            circles.append((
                float(el.get("cx", "0")),
                float(el.get("cy", "0")),
                float(el.get("r", "0")),
            ))
    return paths, circles, sw


def escape_rust(s: str) -> str:
    """转义为 Rust 字符串字面量。"""
    return s.replace("\\", "\\\\").replace('"', '\\"')


def const_name(icon_name: str) -> str:
    """图标名转 Rust 常量名: 'folder-open' -> 'DATA_FOLDER_OPEN'"""
    return "DATA_" + icon_name.upper().replace("-", "_")


# ============================================================
# Rust 代码生成
# ============================================================

# 解析器代码模板（直接从 icon.rs 中复制，保持一致）
PARSER_CODE = r'''
// ── SVG number tokenizer (handles implicit negatives: 1.5-2.9 → [1.5, -2.9]) ──

pub fn tokenize_svg_nums(s: &str) -> Vec<f32> {
    let b = s.as_bytes();
    let mut nums = Vec::new();
    let mut i = 0;
    let n = b.len();
    while i < n {
        while i < n && (b[i] == b' ' || b[i] == b',' || b[i] == b'\n' || b[i] == b'\r' || b[i] == b'\t') { i += 1; }
        if i >= n { break; }
        let start = i;
        if b[i] == b'+' || b[i] == b'-' { i += 1; }
        while i < n && b[i].is_ascii_digit() { i += 1; }
        if i < n && b[i] == b'.' {
            i += 1;
            while i < n && b[i].is_ascii_digit() { i += 1; }
        }
        if i < n && (b[i] == b'e' || b[i] == b'E') {
            i += 1;
            if i < n && (b[i] == b'+' || b[i] == b'-') { i += 1; }
            while i < n && b[i].is_ascii_digit() { i += 1; }
        }
        if i > start {
            if let Ok(v) = s[start..i].parse::<f32>() { nums.push(v); }
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
        if pts.len() >= 2 { out.push(pts.clone()); }
        pts.clear();
    }

    let bytes = d.as_bytes();
    let mut i = 0;
    let n = bytes.len();

    while i < n {
        while i < n && (bytes[i] == b' ' || bytes[i] == b',' || bytes[i] == b'\n' || bytes[i] == b'\r' || bytes[i] == b'\t') { i += 1; }
        if i >= n { break; }

        let cmd;
        if bytes[i].is_ascii_alphabetic() {
            cmd = bytes[i];
            i += 1;
            last_cmd = cmd;
        } else {
            cmd = match last_cmd {
                b'M' => b'L', b'm' => b'l',
                b'Z' | b'z' => { last_cmd },
                _ => last_cmd,
            };
        }

        let arg_start = i;
        while i < n {
            while i < n && (bytes[i] == b' ' || bytes[i] == b',' || bytes[i] == b'\n' || bytes[i] == b'\r' || bytes[i] == b'\t') { i += 1; }
            if i >= n || bytes[i].is_ascii_alphabetic() { break; }
            if bytes[i] == b'+' || bytes[i] == b'-' { i += 1; }
            while i < n && bytes[i].is_ascii_digit() { i += 1; }
            if i < n && bytes[i] == b'.' { i += 1; while i < n && bytes[i].is_ascii_digit() { i += 1; } }
            if i < n && (bytes[i] == b'e' || bytes[i] == b'E') { i += 1; if i < n && (bytes[i] == b'+' || bytes[i] == b'-') { i += 1; } while i < n && bytes[i].is_ascii_digit() { i += 1; } }
        }
        let args = tokenize_svg_nums(&d[arg_start..i]);

        let rel = cmd.is_ascii_lowercase();
        match cmd | 0x20 {
            b'm' => {
                flush(&mut pts, &mut out);
                for j in (0..args.len()).step_by(2) {
                    let (mut x, mut y) = (args[j], *args.get(j+1).unwrap_or(&0.0));
                    if rel { x += cur[0]; y += cur[1]; } else if j == 0 { }
                    if j == 0 {
                        cur = [x, y]; start = cur;
                        flush(&mut pts, &mut out);
                        pts.push(cur);
                    } else {
                        cur = [x, y]; pts.push(cur);
                    }
                }
            }
            b'l' => {
                for j in (0..args.len()).step_by(2) {
                    let (mut x, mut y) = (args[j], *args.get(j+1).unwrap_or(&0.0));
                    if rel { x += cur[0]; y += cur[1]; }
                    cur = [x, y]; pts.push(cur);
                }
            }
            b'h' => {
                for &a in &args {
                    let x = if rel { cur[0] + a } else { a };
                    cur = [x, cur[1]]; pts.push(cur);
                }
            }
            b'v' => {
                for &a in &args {
                    let y = if rel { cur[1] + a } else { a };
                    cur = [cur[0], y]; pts.push(cur);
                }
            }
            b'a' => {
                for j in (0..args.len()).step_by(7) {
                    if j + 6 >= args.len() { break; }
                    let mut rx = args[j].abs();
                    let mut ry = args[j+1].abs();
                    let phi = args[j+2] * std::f32::consts::PI / 180.0;
                    let large = args[j+3] as i32;
                    let sweep_flag = args[j+4] as i32;
                    let (mut ex, mut ey) = (args[j+5], args[j+6]);
                    if rel { ex += cur[0]; ey += cur[1]; }
                    let (sx, sy) = (cur[0], cur[1]);
                    // Skip degenerate arcs
                    if rx < 0.001 || ry < 0.001 || ((sx-ex).abs() < 0.001 && (sy-ey).abs() < 0.001) {
                        cur = [ex, ey]; pts.push(cur);
                        continue;
                    }
                    // SVG arc endpoint to center parameterization
                    let cos_p = phi.cos(); let sin_p = phi.sin();
                    let dx = (sx - ex) / 2.0; let dy = (sy - ey) / 2.0;
                    let x1p = cos_p * dx + sin_p * dy;
                    let y1p = -sin_p * dx + cos_p * dy;
                    // Ensure radii are large enough
                    let lam = (x1p*x1p)/(rx*rx) + (y1p*y1p)/(ry*ry);
                    if lam > 1.0 { let s = lam.sqrt(); rx *= s; ry *= s; }
                    let num = ((rx*rx*ry*ry - rx*rx*y1p*y1p - ry*ry*x1p*x1p)
                              / (rx*rx*y1p*y1p + ry*ry*x1p*x1p)).max(0.0).sqrt();
                    let sign = if large == sweep_flag { -1.0 } else { 1.0 };
                    let cxp = sign * num * rx * y1p / ry;
                    let cyp = -sign * num * ry * x1p / rx;
                    let cx = cos_p * cxp - sin_p * cyp + (sx + ex) / 2.0;
                    let cy = sin_p * cxp + cos_p * cyp + (sy + ey) / 2.0;
                    // Angles
                    fn vec_angle(ux:f32,uy:f32,vx:f32,vy:f32) -> f32 {
                        let n = (ux*ux+uy*uy).sqrt() * (vx*vx+vy*vy).sqrt();
                        if n < 1e-15 { return 0.0; }
                        let c = ((ux*vx+uy*vy)/n).clamp(-1.0, 1.0).acos();
                        if ux*vy - uy*vx < 0.0 { -c } else { c }
                    }
                    let theta1 = vec_angle(1.0, 0.0, (x1p-cxp)/rx, (y1p-cyp)/ry);
                    let mut dtheta = vec_angle((x1p-cxp)/rx, (y1p-cyp)/ry, (-x1p-cxp)/rx, (-y1p-cyp)/ry);
                    if sweep_flag == 0 && dtheta > 0.0 { dtheta -= 2.0 * std::f32::consts::PI; }
                    if sweep_flag != 0 && dtheta < 0.0 { dtheta += 2.0 * std::f32::consts::PI; }
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
                    if j + 5 >= args.len() { break; }
                    let (mut x1, mut y1) = (args[j], args[j+1]);
                    let (mut x2, mut y2) = (args[j+2], args[j+3]);
                    let (mut x, mut y) = (args[j+4], args[j+5]);
                    if rel { x1+=cur[0]; y1+=cur[1]; x2+=cur[0]; y2+=cur[1]; x+=cur[0]; y+=cur[1]; }
                    for s in 1..=8 {
                        let t = s as f32 / 8.0;
                        let mt = 1.0 - t;
                        pts.push([
                            mt*mt*mt*cur[0]+3.0*mt*mt*t*x1+3.0*mt*t*t*x2+t*t*t*x,
                            mt*mt*mt*cur[1]+3.0*mt*mt*t*y1+3.0*mt*t*t*y2+t*t*t*y,
                        ]);
                    }
                    cur = [x, y];
                }
            }
            b'z' => {
                if !pts.is_empty() && pts[0] != cur { pts.push(start); cur = start; }
                flush(&mut pts, &mut out);
            }
            _ => {}
        }
    }
    flush(&mut pts, &mut out);
    out
}

'''

# 渲染 + 缓存 + 绘制 + 测试模板
TAIL_CODE = r'''// ── Icon data (auto-generated by scripts/gen_icons.py) ──

struct IconSvg {
    paths: &'static [&'static str],
    circles: &'static [(f32, f32, f32)],
    stroke_width: f32,
}

%%DATA_CONSTANTS%%

fn icon_svg(name: &str) -> Option<&'static IconSvg> {
    match name {
%%MATCH_ARMS%%
        _ => None,
    }
}

static ICON_CACHE: LazyLock<RwLock<HashMap<String, Vec<[f32; 6]>>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

fn ensure_icon(name: &str) -> Option<()> {
    { let c = ICON_CACHE.read().unwrap(); if c.contains_key(name) { return Some(()); } }
    let svg = icon_svg(name)?;
    let sw = svg.stroke_width;
    let hw = sw * 0.5;
    let mut tris: Vec<[f32; 6]> = Vec::new();
    for &d in svg.paths {
        for poly in &parse_svg_path(d) {
            for w in poly.windows(2) {
                let [x0, y0] = w[0]; let [x1, y1] = w[1];
                let dx = x1-x0; let dy = y1-y0;
                let len = (dx*dx+dy*dy).sqrt();
                if len < 0.01 { continue; }
                let nx = -dy/len*hw; let ny = dx/len*hw;
                tris.push([x0+nx,y0+ny, x0-nx,y0-ny, x1-nx,y1-ny]);
                tris.push([x0+nx,y0+ny, x1-nx,y1-ny, x1+nx,y1+ny]);
            }
        }
    }
    for &(cx,cy,r) in svg.circles {
        let n = 16;
        let inner = r - hw;
        let outer = r + hw;
        for i in 0..n {
            let a0 = std::f32::consts::TAU * i as f32 / n as f32;
            let a1 = std::f32::consts::TAU * (i + 1) as f32 / n as f32;
            let (c0, s0) = (a0.cos(), a0.sin());
            let (c1, s1) = (a1.cos(), a1.sin());
            tris.push([cx+inner*c0, cy+inner*s0, cx+outer*c0, cy+outer*s0, cx+outer*c1, cy+outer*s1]);
            tris.push([cx+inner*c0, cy+inner*s0, cx+outer*c1, cy+outer*s1, cx+inner*c1, cy+inner*s1]);
        }
    }
    let mut c = ICON_CACHE.write().unwrap();
    c.insert(name.to_string(), tris);
    Some(())
}

/// Draw a named Lucide icon at (x, y) with given logical size and color.
pub fn draw_icon(list: &mut DrawList, name: &str, x: f32, y: f32, size: f32, color: [f32; 4]) {
    if ensure_icon(name).is_none() { return; }
    let c = ICON_CACHE.read().unwrap();
    let tris = match c.get(name) { Some(t) => t, None => return };
    let scale = size / 24.0;
    for t in tris {
        list.fill_triangle(
            [x+t[0]*scale, y+t[1]*scale],
            [x+t[2]*scale, y+t[3]*scale],
            [x+t[4]*scale, y+t[5]*scale],
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
        assert!(icon_svg("plus").is_some());
        assert!(icon_svg("search").is_some());
        assert!(icon_svg("eye").is_some());
        assert!(icon_svg("eye-off").is_some());
        assert!(icon_svg("nonexistent").is_none());
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
'''


def generate_rust(all_data: dict) -> str:
    """生成完整的 icon.rs Rust 源码（const DATA_* 格式）。

    Args:
        all_data: {图标名: (paths列表, circles列表, stroke_width)}

    Returns:
        Rust 源码字符串
    """
    lines = []

    # ── 文件头 ──
    lines += [
        "//! SVG icon renderer with pre-loaded Lucide icons.",
        "//!",
        "//! Adding a new icon:",
        "//!   1. Place `name.svg` in `~/Downloads/icons/`",
        "//!   2. Add `\"name\"` to `ICONS` in `scripts/gen_icons.py`",
        "//!   3. Run: `python3 scripts/gen_icons.py`",
        "//!",
        "//! Or use the parser directly:",
        "//!   ```",
        "//!   let polys = parse_svg_path(\"M5 12h14\");",
        "//!   // tessellate polys into triangles for drawing",
        "//!   ```",
        "//! Core SVG parser is hand-written; Lucide icon data is auto-generated.",
        "//! Source: Lucide icons (MIT license).",
        "",
        "use std::collections::HashMap;",
        "use std::sync::{LazyLock, RwLock};",
        "",
        "use crate::core::paint::DrawList;",
        "",
    ]

    # ── 解析器（从模板复制，确保与手写版本一致）──
    lines.append(PARSER_CODE.rstrip())
    lines.append("")

    # ── IconSvg struct ──
    lines += [
        "// ── Icon data (auto-generated by scripts/gen_icons.py) ──",
        "",
        "struct IconSvg {",
        "    paths: &'static [&'static str],",
        "    circles: &'static [(f32, f32, f32)],",
        "    stroke_width: f32,",
        "}",
        "",
    ]

    # ── const DATA_* 常量 ──
    for name, (paths, circles, sw) in all_data.items():
        cname = const_name(name)
        lines.append(f"const {cname}: IconSvg = IconSvg {{")
        lines.append("    paths: &[")
        for p in paths:
            lines.append(f'        "{escape_rust(p)}",')
        lines.append("    ],")
        if circles:
            lines.append("    circles: &[")
            for c in circles:
                lines.append(f"        ({c[0]}, {c[1]}, {c[2]}),")
            lines.append("    ],")
        else:
            lines.append("    circles: &[],")
        lines.append(f"    stroke_width: {sw},")
        lines.append("};")
        lines.append("")

    # ── icon_svg match ──
    lines += [
        "fn icon_svg(name: &str) -> Option<&'static IconSvg> {",
        "    match name {",
    ]
    for name in all_data:
        lines.append(f'        "{name}" => Some(&{const_name(name)}),')
    lines += [
        "        _ => None,",
        "    }",
        "}",
        "",
    ]

    # ── 渲染 + 缓存 + 测试 ──
    # 从 TAIL_CODE 模板中提取（去掉 %%DATA_CONSTANTS%% 和 %%MATCH_ARMS%% 占位符后的部分）
    # TAIL_CODE 模板从 struct IconSvg 开始，但我们已经生成了数据常量和 match
    # 所以只需要从 ICON_CACHE 开始的部分
    tail_start = TAIL_CODE.find("static ICON_CACHE:")
    lines.append(TAIL_CODE[tail_start:].rstrip())

    return "\n".join(lines) + "\n"


# ============================================================
# 主函数
# ============================================================

def find_project_root() -> str:
    """查找项目根目录（包含 Cargo.toml 的目录）。"""
    d = os.path.dirname(os.path.abspath(__file__))
    while d != "/":
        if os.path.exists(os.path.join(d, "Cargo.toml")):
            return d
        d = os.path.dirname(d)
    return os.path.dirname(os.path.abspath(__file__))


def main():
    parser = argparse.ArgumentParser(description="将 Lucide SVG 图标转换为 Rust 渲染代码")
    parser.add_argument("--src", default=DEFAULT_SRC, help=f"SVG 源目录 (默认: {DEFAULT_SRC})")
    parser.add_argument("--out", default=None, help="输出文件路径 (默认: crates/ui/src/widgets/icon.rs)")
    parser.add_argument("--icons", default=None, help="逗号分隔的图标名列表 (默认: 使用脚本内置列表)")
    parser.add_argument("--dry-run", action="store_true", help="预览模式，不写文件")
    args = parser.parse_args()

    if args.out:
        out_path = args.out
    else:
        root = find_project_root()
        out_path = os.path.join(root, "crates", "ui", "src", "widgets", "icon.rs")

    icons = [s.strip() for s in args.icons.split(",")] if args.icons else ICONS

    print(f"📂 SVG 源目录: {args.src}")
    print(f"📝 目标图标: {', '.join(icons)}")
    print()

    all_data = {}
    for name in icons:
        svg_path = os.path.join(args.src, f"{name}.svg")
        if not os.path.exists(svg_path):
            print(f"  ⚠  {name}: 文件不存在 {svg_path}")
            continue
        paths, circles, sw = extract_svg(svg_path)
        all_data[name] = (paths, circles, sw)
        print(f"  ✓  {name}: {len(paths)} 条路径, {len(circles)} 个圆, 笔宽={sw}")

    if not all_data:
        print("\n❌ 错误: 没有找到任何图标文件")
        sys.exit(1)

    print()
    rust_code = generate_rust(all_data)

    if args.dry_run:
        print(f"[dry-run] 将写入 {out_path}")
        print(f"  大小: {len(rust_code)} bytes, {len(rust_code.splitlines())} lines")
    else:
        os.makedirs(os.path.dirname(out_path), exist_ok=True)
        with open(out_path, "w") as f:
            f.write(rust_code)
        size = os.path.getsize(out_path)
        lines = len(rust_code.splitlines())
        print(f"✅ 写入 {out_path}")
        print(f"   {size} bytes, {lines} lines")
        print()
        print("下一步: cargo check -p textora-ui")


if __name__ == "__main__":
    main()
