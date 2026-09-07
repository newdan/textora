use std::collections::BTreeMap;
use std::sync::OnceLock;

#[derive(Debug, Clone)]
pub struct MindmapTheme {
    pub canvas: MindmapCanvasTheme,
    pub node: MindmapNodeTheme,
    pub semantic: MindmapSemanticTheme,
    pub geometry: MindmapGeometry,
}

#[derive(Debug, Clone)]
pub struct MindmapCanvasTheme {
    pub background: [f32; 4],
    pub connector: [f32; 4],
    pub connector_hover: [f32; 4],
    pub selection: [f32; 4],
    pub focus_ring: [f32; 4],
    pub drag_invalid: [f32; 4],
    /// 根的一级子树分支调色板，按 branch_index 取模循环。
    pub branch_palette: Vec<[f32; 4]>,
}

impl MindmapCanvasTheme {
    /// 返回分支染色；调色板为空时返回 None，调用方回退到默认色。
    pub fn branch_color(&self, branch_index: usize) -> Option<[f32; 4]> {
        if self.branch_palette.is_empty() {
            return None;
        }
        Some(self.branch_palette[branch_index % self.branch_palette.len()])
    }
}

#[derive(Debug, Clone)]
pub struct MindmapNodeStyle {
    pub fill: [f32; 4],
    pub border: [f32; 4],
    pub text: [f32; 4],
    pub accent: [f32; 4],
}

#[derive(Debug, Clone)]
pub struct MindmapNodeTheme {
    pub default: MindmapNodeStyle,
    pub root: MindmapNodeStyle,
    pub depth: Vec<MindmapNodeStyle>,
}

#[derive(Debug, Clone)]
pub struct MindmapSemanticTheme {
    pub status: MindmapStatusTheme,
    pub priority: MindmapPriorityTheme,
    pub named: BTreeMap<String, MindmapNodeStyle>,
}

#[derive(Debug, Clone)]
pub struct MindmapStatusTheme {
    pub todo: MindmapNodeStyle,
    pub doing: MindmapNodeStyle,
    pub done: MindmapNodeStyle,
    pub blocked: MindmapNodeStyle,
    pub canceled: MindmapNodeStyle,
}

#[derive(Debug, Clone)]
pub struct MindmapPriorityTheme {
    pub p0: MindmapNodeStyle,
    pub p1: MindmapNodeStyle,
    pub p2: MindmapNodeStyle,
    pub p3: MindmapNodeStyle,
}

#[derive(Debug, Clone)]
pub struct MindmapGeometry {
    pub card_height: f32,
    pub card_padding_x: f32,
    pub card_padding_y: f32,
    pub root_child_gap: f32,
    pub nested_child_gap: f32,
    pub sibling_gap: f32,
    pub card_radius: f32,
    pub connector_width: f32,
    pub selection_outline_width: f32,
    pub selection_outline_gap: f32,
    pub drag_source_alpha: f32,
    pub drag_preview_alpha: f32,
    pub same_level_threshold_ratio: f32,
    /// 各深度字号缩放，下标即深度（0=根）。卡片高度同比例推导。
    pub depth_font_scales: Vec<f32>,
}

impl MindmapGeometry {
    /// 深度越界时钳制到最后一档，空数组回退 1.0。
    pub fn font_scale_for_depth(&self, depth: u8) -> f32 {
        if self.depth_font_scales.is_empty() {
            return 1.0;
        }
        let index = (depth as usize).min(self.depth_font_scales.len() - 1);
        self.depth_font_scales[index]
    }
}

impl MindmapTheme {
    pub fn gamma_correct(&mut self) {
        let gamma = 2.2;
        let correct = |c: &mut [f32; 4]| {
            for ch in c[..3].iter_mut() {
                *ch = ch.powf(gamma);
            }
        };

        correct(&mut self.canvas.background);
        correct(&mut self.canvas.connector);
        correct(&mut self.canvas.connector_hover);
        correct(&mut self.canvas.selection);
        correct(&mut self.canvas.focus_ring);
        correct(&mut self.canvas.drag_invalid);
        for c in &mut self.canvas.branch_palette {
            correct(c);
        }

        let correct_style = |s: &mut MindmapNodeStyle| {
            correct(&mut s.fill);
            correct(&mut s.border);
            correct(&mut s.text);
            correct(&mut s.accent);
        };

        correct_style(&mut self.node.default);
        correct_style(&mut self.node.root);
        for s in &mut self.node.depth {
            correct_style(s);
        }

        correct_style(&mut self.semantic.status.todo);
        correct_style(&mut self.semantic.status.doing);
        correct_style(&mut self.semantic.status.done);
        correct_style(&mut self.semantic.status.blocked);
        correct_style(&mut self.semantic.status.canceled);

        correct_style(&mut self.semantic.priority.p0);
        correct_style(&mut self.semantic.priority.p1);
        correct_style(&mut self.semantic.priority.p2);
        correct_style(&mut self.semantic.priority.p3);

        for s in self.semantic.named.values_mut() {
            correct_style(s);
        }
    }
}

// Built-in Geometry defaults
impl Default for MindmapGeometry {
    fn default() -> Self {
        Self {
            card_height: 32.0,
            card_padding_x: 16.0,
            card_padding_y: 6.0,
            root_child_gap: 35.0,
            nested_child_gap: 25.0,
            sibling_gap: 8.0,
            card_radius: 6.0,
            connector_width: 8.0,
            selection_outline_width: 2.0,
            selection_outline_gap: 2.0,
            drag_source_alpha: 0.45,
            drag_preview_alpha: 0.85,
            same_level_threshold_ratio: 0.35,
            depth_font_scales: vec![1.35, 1.15, 1.0, 0.9],
        }
    }
}

impl MindmapTheme {
    pub fn default_dark() -> Self {
        use crate::hex_color::parse_hex;
        let h = |s: &str| parse_hex(s).unwrap();
        Self {
            canvas: MindmapCanvasTheme {
                background: h("#211D19"),
                connector: h("#4D4337"),
                connector_hover: h("#D0B082"),
                selection: h("#D0B08233"),
                focus_ring: h("#D0B082"),
                drag_invalid: h("#D59A8C"),
                branch_palette: vec![
                    h("#D0B082"),
                    h("#CD9C87"),
                    h("#AFBB8C"),
                    h("#C99DAD"),
                    h("#B4A1C8"),
                    h("#8DB9AD"),
                ],
            },
            node: MindmapNodeTheme {
                default: MindmapNodeStyle {
                    fill: h("#302922"),
                    border: h("#302922"),
                    text: h("#CFC3B3"),
                    accent: h("#CFC3B3"),
                },
                root: MindmapNodeStyle {
                    fill: h("#D0B082"),
                    border: h("#D0B082"),
                    text: h("#2B2118"),
                    accent: h("#D0B082"),
                },
                depth: vec![MindmapNodeStyle {
                    fill: h("#302922"),
                    border: h("#D0B082"),
                    text: h("#D0B082"),
                    accent: h("#D0B082"),
                }],
            },
            semantic: MindmapSemanticTheme {
                status: MindmapStatusTheme {
                    todo: MindmapNodeStyle {
                        fill: h("#302922"),
                        border: h("#CFC3B3"),
                        text: h("#CFC3B3"),
                        accent: h("#CFC3B3"),
                    },
                    doing: MindmapNodeStyle {
                        fill: h("#302922"),
                        border: h("#D0B082"),
                        text: h("#D0B082"),
                        accent: h("#D0B082"),
                    },
                    done: MindmapNodeStyle {
                        fill: h("#302922"),
                        border: h("#AFBB8C"),
                        text: h("#AFBB8C"),
                        accent: h("#AFBB8C"),
                    },
                    blocked: MindmapNodeStyle {
                        fill: h("#302922"),
                        border: h("#D59A8C"),
                        text: h("#D59A8C"),
                        accent: h("#D59A8C"),
                    },
                    canceled: MindmapNodeStyle {
                        fill: h("#29231E"),
                        border: h("#29231E"),
                        text: h("#AB9E8E"),
                        accent: h("#AB9E8E"),
                    },
                },
                priority: MindmapPriorityTheme {
                    p0: MindmapNodeStyle {
                        fill: h("#D59A8C33"),
                        border: h("#D59A8C"),
                        text: h("#D59A8C"),
                        accent: h("#D59A8C"),
                    },
                    p1: MindmapNodeStyle {
                        fill: h("#D0B08233"),
                        border: h("#D0B082"),
                        text: h("#D0B082"),
                        accent: h("#D0B082"),
                    },
                    p2: MindmapNodeStyle {
                        fill: h("#8DB9AD33"),
                        border: h("#8DB9AD"),
                        text: h("#8DB9AD"),
                        accent: h("#8DB9AD"),
                    },
                    p3: MindmapNodeStyle {
                        fill: h("#CFC3B333"),
                        border: h("#CFC3B3"),
                        text: h("#CFC3B3"),
                        accent: h("#CFC3B3"),
                    },
                },
                named: BTreeMap::new(),
            },
            geometry: MindmapGeometry::default(),
        }
    }

    pub fn default_light() -> Self {
        use crate::hex_color::parse_hex;
        let h = |s: &str| parse_hex(s).unwrap();
        Self {
            canvas: MindmapCanvasTheme {
                background: h("#F6F3EC"),
                connector: h("#C9BCA8"),
                connector_hover: h("#D9822B"),
                selection: h("#D9822B33"),
                focus_ring: h("#D9822B"),
                drag_invalid: h("#CC4B3C"),
                branch_palette: vec![
                    h("#D9822B"),
                    h("#D1603D"),
                    h("#74942F"),
                    h("#C2537E"),
                    h("#9568C9"),
                    h("#2A9D8F"),
                ],
            },
            node: MindmapNodeTheme {
                default: MindmapNodeStyle {
                    fill: h("#FFFDF8"),
                    border: h("#E0D5C4"),
                    text: h("#4A4238"),
                    accent: h("#A39480"),
                },
                root: MindmapNodeStyle {
                    fill: h("#4A3B2C"),
                    border: h("#4A3B2C"),
                    text: h("#FFFDF8"),
                    accent: h("#D9822B"),
                },
                depth: vec![MindmapNodeStyle {
                    fill: h("#F7EDDD"),
                    border: h("#D8B98A"),
                    text: h("#5C4526"),
                    accent: h("#D9822B"),
                }],
            },
            semantic: MindmapSemanticTheme {
                status: MindmapStatusTheme {
                    todo: MindmapNodeStyle {
                        fill: h("#FFFDF8"),
                        border: h("#E0D5C4"),
                        text: h("#8A8176"),
                        accent: h("#8A8176"),
                    },
                    doing: MindmapNodeStyle {
                        fill: h("#FFFDF8"),
                        border: h("#D9822B"),
                        text: h("#D9822B"),
                        accent: h("#D9822B"),
                    },
                    done: MindmapNodeStyle {
                        fill: h("#F1F6E0"),
                        border: h("#A9C584"),
                        text: h("#3F5522"),
                        accent: h("#74942F"),
                    },
                    blocked: MindmapNodeStyle {
                        fill: h("#FFFDF8"),
                        border: h("#CC4B3C"),
                        text: h("#CC4B3C"),
                        accent: h("#CC4B3C"),
                    },
                    canceled: MindmapNodeStyle {
                        fill: h("#F1EBDD"),
                        border: h("#F1EBDD"),
                        text: h("#8A8176"),
                        accent: h("#8A8176"),
                    },
                },
                priority: MindmapPriorityTheme {
                    p0: MindmapNodeStyle {
                        fill: h("#FBE9E4"),
                        border: h("#D1705C"),
                        text: h("#6B2B20"),
                        accent: h("#CC4B3C"),
                    },
                    p1: MindmapNodeStyle {
                        fill: h("#FBF0DC"),
                        border: h("#E0A765"),
                        text: h("#5E3813"),
                        accent: h("#D9822B"),
                    },
                    p2: MindmapNodeStyle {
                        fill: h("#E2F2EF"),
                        border: h("#7FBDB2"),
                        text: h("#1F4A43"),
                        accent: h("#2A9D8F"),
                    },
                    p3: MindmapNodeStyle {
                        fill: h("#F3EEE2"),
                        border: h("#DCD2C2"),
                        text: h("#8A8176"),
                        accent: h("#8A8176"),
                    },
                },
                named: BTreeMap::new(),
            },
            geometry: MindmapGeometry::default(),
        }
    }
}

pub const DEFAULT_MINDMAP_COLOR_SCHEME_ID: &str = "paper";

#[derive(Clone, Debug)]
pub struct MindmapColorScheme {
    pub id: &'static str,
    pub display_name: &'static str,
    pub canvas: MindmapCanvasTheme,
    pub node: MindmapNodeTheme,
    pub semantic: MindmapSemanticTheme,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MindmapThemeSelection {
    Default,
    Selected(String),
    Unknown(String),
    InvalidMetadata,
}

#[derive(Clone, Copy)]
pub struct MindmapRenderTheme<'a> {
    pub canvas: &'a MindmapCanvasTheme,
    pub node: &'a MindmapNodeTheme,
    pub semantic: &'a MindmapSemanticTheme,
    pub geometry: &'a MindmapGeometry,
}

impl<'a> MindmapRenderTheme<'a> {
    pub fn new(scheme: &'a MindmapColorScheme, geometry: &'a MindmapGeometry) -> Self {
        Self { canvas: &scheme.canvas, node: &scheme.node, semantic: &scheme.semantic, geometry }
    }
}

/// (id, 显示名, 画布背景, 连接线, 主色, 分支调色板)
/// 浅色使用低饱和的墨色分支，兼顾根节点白字与分支染色背景上的文字对比度。
type SchemePalette =
    (&'static str, &'static str, &'static str, &'static str, &'static str, [&'static str; 6]);

const SCHEME_PALETTES: [SchemePalette; 10] = [
    (
        "paper",
        "素纸",
        "#F8F7F3",
        "#C9C6BC",
        "#536575",
        ["#536575", "#895A39", "#5C6B3B", "#885565", "#6E5E88", "#3C6D63"],
    ),
    (
        "dawn",
        "晨曦",
        "#FCF7F3",
        "#D8C7BE",
        "#9A4E49",
        ["#9A4E49", "#895C30", "#63683B", "#406C67", "#4F6787", "#7F5879"],
    ),
    (
        "amber",
        "琥珀",
        "#FBF7EF",
        "#D5C7B0",
        "#895C1E",
        ["#895C1E", "#77612E", "#95553B", "#5E6A3F", "#3F6C63", "#7D5B76"],
    ),
    (
        "meadow",
        "青禾",
        "#F5F8F2",
        "#C4CFBD",
        "#456E4D",
        ["#456E4D", "#616931", "#396D64", "#48687C", "#6E5D7C", "#845B36"],
    ),
    (
        "tide",
        "潮汐",
        "#F3F7FA",
        "#BFCCD5",
        "#37698A",
        ["#37698A", "#406A74", "#3C6D60", "#5A6189", "#775B7D", "#576A42"],
    ),
    (
        "iris",
        "鸢尾",
        "#F8F5FA",
        "#CDC3D5",
        "#79578F",
        ["#79578F", "#845570", "#56638B", "#3F6C68", "#795F31", "#91535D"],
    ),
    (
        "sakura",
        "樱花",
        "#FBF5F7",
        "#D8C3CB",
        "#944C69",
        ["#944C69", "#8F5647", "#626738", "#426D65", "#546588", "#785980"],
    ),
    (
        "mint",
        "薄荷",
        "#F2F8F5",
        "#BFCFC7",
        "#356F5F",
        ["#356F5F", "#516C47", "#386C7A", "#676635", "#835B36", "#6F5C83"],
    ),
    (
        "latte",
        "拿铁",
        "#F8F4EE",
        "#D2C6B8",
        "#7E5C44",
        ["#7E5C44", "#88593B", "#75622F", "#5E683F", "#825665", "#6A5D7F"],
    ),
    (
        "graphite",
        "石墨",
        "#F5F6F7",
        "#C6CBD0",
        "#4C5967",
        ["#4C5967", "#46677F", "#416B62", "#6B5D80", "#855665", "#815D39"],
    ),
];

/// (id, 显示名, 画布背景, 卡片填充, 连接线, 卡片文字, 主色, 根节点文字, 分支调色板)
/// 根节点文字与主色对比度 ≥ 4.5；主色与卡片文字在卡片填充上对比度 ≥ 4.5。
type DarkSchemePalette = (
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    [&'static str; 6],
);

const DARK_SCHEME_PALETTES: [DarkSchemePalette; 5] = [
    (
        "ocean-night",
        "夜航",
        "#151C24",
        "#202B36",
        "#394858",
        "#BDC9D3",
        "#8BB8D5",
        "#15232E",
        ["#8BB8D5", "#89BEAE", "#C8B186", "#CB9DAD", "#ABA2CF", "#A7BC90"],
    ),
    (
        "pine-night",
        "松夜",
        "#171E19",
        "#242E26",
        "#3B4B3D",
        "#C0CDBF",
        "#98BF9B",
        "#19281C",
        ["#98BF9B", "#B5BE8E", "#85BCAD", "#CBB38C", "#93B4CA", "#C29EAB"],
    ),
    (
        "wine-night",
        "绛夜",
        "#21191E",
        "#30252C",
        "#51404A",
        "#D1BFC7",
        "#D49CAA",
        "#2C1922",
        ["#D49CAA", "#CEAF89", "#AAB991", "#8BB9AE", "#B5A0CA", "#CE9F89"],
    ),
    (
        "violet-night",
        "堇夜",
        "#1C1926",
        "#2A2637",
        "#464055",
        "#C9C1D7",
        "#B6A5D6",
        "#241C32",
        ["#B6A5D6", "#C89FB7", "#96B4D0", "#8FBBAF", "#C6B38C", "#A8BB94"],
    ),
    (
        "basalt-night",
        "玄武",
        "#1B1D21",
        "#282B31",
        "#434850",
        "#C6CBD3",
        "#B1BDCC",
        "#202630",
        ["#B1BDCC", "#C5B18E", "#A6BA99", "#91B8AC", "#96B3CF", "#C39EAE"],
    ),
];

pub fn built_in_mindmap_color_schemes() -> &'static [MindmapColorScheme] {
    static SCHEMES: OnceLock<Vec<MindmapColorScheme>> = OnceLock::new();
    SCHEMES.get_or_init(|| {
        let mut schemes = Vec::with_capacity(16);
        schemes.extend(SCHEME_PALETTES.iter().map(
            |&(id, display_name, background, connector, primary, palette)| {
                build_light_scheme(id, display_name, background, connector, primary, palette)
            },
        ));
        schemes.push(build_warm_night_scheme());
        schemes.extend(DARK_SCHEME_PALETTES.iter().map(
            |&(
                id,
                display_name,
                background,
                card_fill,
                connector,
                text,
                primary,
                root_text,
                palette,
            )| {
                build_dark_scheme(
                    id,
                    display_name,
                    background,
                    card_fill,
                    connector,
                    text,
                    primary,
                    root_text,
                    palette,
                )
            },
        ));
        schemes
    })
}

pub fn find_mindmap_color_scheme(id: &str) -> Option<&'static MindmapColorScheme> {
    built_in_mindmap_color_schemes().iter().find(|scheme| scheme.id == id)
}

pub fn resolve_mindmap_theme_selection(id: Option<&str>) -> MindmapThemeSelection {
    match id {
        None => MindmapThemeSelection::Default,
        Some(id) => {
            if find_mindmap_color_scheme(id).is_some() {
                MindmapThemeSelection::Selected(id.to_string())
            } else {
                MindmapThemeSelection::Unknown(id.to_string())
            }
        }
    }
}

fn build_warm_night_scheme() -> MindmapColorScheme {
    let mut theme = MindmapTheme::default_dark();
    theme.gamma_correct();
    MindmapColorScheme {
        id: "warm-night",
        display_name: "暖夜",
        canvas: theme.canvas,
        node: theme.node,
        semantic: theme.semantic,
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "built-in scheme construction maps each documented semantic color explicitly"
)]
fn build_dark_scheme(
    id: &'static str,
    display_name: &'static str,
    background: &'static str,
    card_fill: &'static str,
    connector: &'static str,
    text: &'static str,
    primary_hex: &'static str,
    root_text: &'static str,
    palette: [&'static str; 6],
) -> MindmapColorScheme {
    use crate::hex_color::parse_hex;
    let h = |s: &'static str| parse_hex(s).expect("built-in palette colors are valid hex");
    let primary = h(primary_hex);
    let selection = {
        let mut c = primary;
        c[3] = 0x33 as f32 / 255.0;
        c
    };

    let canvas = MindmapCanvasTheme {
        background: h(background),
        connector: h(connector),
        connector_hover: primary,
        selection,
        focus_ring: primary,
        drag_invalid: h("#E06655"),
        branch_palette: palette.iter().copied().map(h).collect(),
    };

    let node = MindmapNodeTheme {
        default: MindmapNodeStyle {
            fill: h(card_fill),
            border: h(card_fill),
            text: h(text),
            accent: h(text),
        },
        root: MindmapNodeStyle {
            fill: primary,
            border: primary,
            text: h(root_text),
            accent: primary,
        },
        depth: vec![MindmapNodeStyle {
            fill: h(card_fill),
            border: primary,
            text: primary,
            accent: primary,
        }],
    };

    let semantic = MindmapTheme::default_dark().semantic;
    let mut theme = MindmapTheme { canvas, node, semantic, geometry: MindmapGeometry::default() };
    theme.gamma_correct();

    MindmapColorScheme {
        id,
        display_name,
        canvas: theme.canvas,
        node: theme.node,
        semantic: theme.semantic,
    }
}

fn build_light_scheme(
    id: &'static str,
    display_name: &'static str,
    background: &'static str,
    connector: &'static str,
    primary_hex: &'static str,
    palette: [&'static str; 6],
) -> MindmapColorScheme {
    use crate::hex_color::parse_hex;
    let h = |s: &'static str| parse_hex(s).expect("built-in palette colors are valid hex");
    let primary = h(primary_hex);
    let selection = {
        let mut c = primary;
        c[3] = 0x33 as f32 / 255.0;
        c
    };

    let canvas = MindmapCanvasTheme {
        background: h(background),
        connector: h(connector),
        connector_hover: primary,
        selection,
        focus_ring: primary,
        drag_invalid: h("#D93025"),
        branch_palette: palette.iter().copied().map(h).collect(),
    };

    let node = MindmapNodeTheme {
        default: MindmapNodeStyle {
            fill: h("#FFFFFF"),
            border: h("#DADCE0"),
            text: h("#202124"),
            accent: h("#5F6368"),
        },
        root: MindmapNodeStyle {
            fill: primary,
            border: primary,
            text: h("#FFFFFF"),
            accent: primary,
        },
        depth: vec![MindmapNodeStyle {
            fill: h("#FFFFFF"),
            border: primary,
            text: primary,
            accent: primary,
        }],
    };

    let semantic = MindmapTheme::default_light().semantic;
    let mut theme = MindmapTheme { canvas, node, semantic, geometry: MindmapGeometry::default() };
    theme.gamma_correct();

    MindmapColorScheme {
        id,
        display_name,
        canvas: theme.canvas,
        node: theme.node,
        semantic: theme.semantic,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_geometry_keeps_connectors_visibly_tapered() {
        assert_eq!(MindmapGeometry::default().connector_width, 8.0);
    }

    #[test]
    fn default_geometry_uses_compact_parent_child_gaps() {
        let geometry = MindmapGeometry::default();

        assert_eq!(geometry.root_child_gap, 35.0);
        assert_eq!(geometry.nested_child_gap, 25.0);
    }

    #[test]
    fn font_scale_for_depth_clamps_to_last_entry() {
        let geometry = MindmapGeometry::default();

        assert_eq!(geometry.font_scale_for_depth(0), 1.35);
        assert_eq!(geometry.font_scale_for_depth(1), 1.15);
        assert_eq!(geometry.font_scale_for_depth(2), 1.0);
        assert_eq!(geometry.font_scale_for_depth(3), 0.9);
        assert_eq!(geometry.font_scale_for_depth(9), 0.9);
    }

    #[test]
    fn font_scale_for_depth_falls_back_to_one_when_empty() {
        let geometry = MindmapGeometry { depth_font_scales: vec![], ..Default::default() };

        assert_eq!(geometry.font_scale_for_depth(0), 1.0);
    }

    #[test]
    fn branch_color_cycles_through_palette() {
        let dark = MindmapTheme::default_dark();
        let palette_len = dark.canvas.branch_palette.len();
        assert!(palette_len >= 6, "dark palette should distinguish many branches");

        let first = dark.canvas.branch_color(0).expect("palette is non-empty");
        let cycled = dark.canvas.branch_color(palette_len).expect("palette is non-empty");
        assert_eq!(first, cycled);
        assert_ne!(first, dark.canvas.branch_color(1).expect("palette is non-empty"));

        let light = MindmapTheme::default_light();
        assert!(light.canvas.branch_palette.len() >= 6);
    }

    #[test]
    fn branch_color_returns_none_for_empty_palette() {
        let mut dark = MindmapTheme::default_dark();
        dark.canvas.branch_palette.clear();

        assert_eq!(dark.canvas.branch_color(0), None);
    }

    #[test]
    fn gamma_correct_also_corrects_branch_palette() {
        let mut theme = MindmapTheme::default_dark();
        let original = theme.canvas.branch_palette[0];

        theme.gamma_correct();

        let corrected = theme.canvas.branch_palette[0];
        assert!((corrected[0] - original[0].powf(2.2)).abs() < 1e-6, "RGB must be gamma-corrected");
        assert!(
            (corrected[3] - original[3]).abs() < f32::EPSILON,
            "alpha must not be gamma-corrected"
        );
    }

    #[test]
    fn defaults_provide_visible_selection_and_drag_feedback() {
        let dark = MindmapTheme::default_dark();
        let light = MindmapTheme::default_light();

        assert!(dark.canvas.drag_invalid[3] > 0.0);
        assert!(light.canvas.drag_invalid[3] > 0.0);
        assert!(dark.geometry.selection_outline_width > 1.0);
        assert!(light.geometry.selection_outline_width > 1.0);
        assert_eq!(dark.geometry.same_level_threshold_ratio, 0.35);
        assert_eq!(light.geometry.same_level_threshold_ratio, 0.35);
    }

    #[test]
    fn built_in_color_schemes_have_stable_unique_ids() {
        let schemes = built_in_mindmap_color_schemes();
        let ids = schemes.iter().map(|scheme| scheme.id).collect::<Vec<_>>();
        assert_eq!(
            ids,
            [
                "paper",
                "dawn",
                "amber",
                "meadow",
                "tide",
                "iris",
                "sakura",
                "mint",
                "latte",
                "graphite",
                "warm-night",
                "ocean-night",
                "pine-night",
                "wine-night",
                "violet-night",
                "basalt-night",
            ]
        );
        let unique = ids.iter().copied().collect::<std::collections::BTreeSet<_>>();
        assert_eq!(unique.len(), ids.len());
    }

    #[test]
    fn default_color_scheme_is_a_built_in_light_scheme() {
        let scheme = find_mindmap_color_scheme(DEFAULT_MINDMAP_COLOR_SCHEME_ID)
            .expect("default scheme id must resolve to a built-in scheme");

        let background_luma =
            scheme.canvas.background[0] + scheme.canvas.background[1] + scheme.canvas.background[2];
        assert!(background_luma > 1.5, "default scheme should be light, got {background_luma}");
    }

    #[test]
    fn absent_and_unknown_theme_ids_have_distinct_selection_states() {
        assert_eq!(resolve_mindmap_theme_selection(None), MindmapThemeSelection::Default);
        assert_eq!(
            resolve_mindmap_theme_selection(Some("tide")),
            MindmapThemeSelection::Selected("tide".into())
        );
        assert_eq!(
            resolve_mindmap_theme_selection(Some("future-theme")),
            MindmapThemeSelection::Unknown("future-theme".into())
        );
    }

    #[test]
    fn fixed_scheme_colors_do_not_depend_on_application_theme() {
        let scheme = find_mindmap_color_scheme("dawn").expect("dawn is a built-in scheme");
        let dark_geometry = &MindmapTheme::default_dark().geometry;
        let light_geometry = &MindmapTheme::default_light().geometry;
        let dark_render = MindmapRenderTheme::new(scheme, dark_geometry);
        let light_render = MindmapRenderTheme::new(scheme, light_geometry);
        assert_eq!(dark_render.canvas.background, light_render.canvas.background);
        assert_eq!(dark_render.canvas.branch_palette, light_render.canvas.branch_palette);
    }
}
