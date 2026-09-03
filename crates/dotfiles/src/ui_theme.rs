// ─────────────────────────────────────────────────────────────────────────────
// Design tokens — Apple macOS 27 UI Kit (`Apple-macOS-27-UI-Kit.sketch`)
// + https://developer.apple.com/design/human-interface-guidelines/designing-for-macos
//
// Single source of truth for colors, fills, separators, shadows and button
// specs. All values are extracted from the Sketch document:
//   • Labels  (Labels/Light|Dark/1..6)     — text hierarchy
//   • Fills   (Fills/Light|Dark/1..5)      — surface hierarchy, composited
//     over the opaque window background so egui never stacks translucency
//   • Separators/Light #3C3C43 @29%        — hairlines
//   • System Colors (Light|Dark)           — accent + semantic hues
//   • Alerts/⌘/Light|Dark/Background       — window shadow recipe
//
// Compositing math (source-over over the opaque window background):
//   light: 255 + (c - 255) * a      dark: 30 + (c - 30) * a   (bg #1E1E1E = 30)
// Every returned color is opaque — WCAG ratios are stable in both themes.
// Body text pairs target ≥ 4.5:1; accent-colored text ≥ 3:1 (Apple hyperlink
// grade, matches NSColor.linkColor on its own window background).
// ─────────────────────────────────────────────────────────────────────────────
use eframe::egui;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Theme {
    Light,
    Dark,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Primary,
    Secondary,
    Tertiary,
    /// Kit label level 4 — reserved for disabled/dismissed text.
    #[allow(dead_code)]
    Quaternary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FillLevel {
    Primary,
    Secondary,
    Tertiary,
    Quaternary,
    Quinary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hue {
    Accent,
    Success,
    Warning,
    Danger,
    Neutral,
}

/// Status glyph color source for step rows / dots.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dot {
    Pending,
    /// Running dot color on plain window backgrounds — reserved; running rows
    /// inside tinted surfaces use the tint fg instead (AA).
    #[allow(dead_code)]
    Running,
    Success,
    Failed,
    Skipped,
}

/// Foreground/background pair for pills, badges and banners (AA-validated).
#[derive(Debug, Clone, Copy)]
pub struct Tint {
    pub bg: egui::Color32,
    pub fg: egui::Color32,
}

#[derive(Debug, Clone, Copy)]
pub struct ButtonStyle {
    pub fill: egui::Color32,
    pub stroke: egui::Stroke,
    pub text: egui::Color32,
}

// ── Window backgrounds — “Window Backgrounds/Light|Dark Background” ─────────
pub const WINDOW_BG_LIGHT: egui::Color32 = egui::Color32::from_rgb(255, 255, 255);
pub const WINDOW_BG_DARK: egui::Color32 = egui::Color32::from_rgb(30, 30, 30);

pub fn window_bg(theme: Theme) -> egui::Color32 {
    match theme {
        Theme::Light => WINDOW_BG_LIGHT,
        Theme::Dark => WINDOW_BG_DARK,
    }
}

// ── Labels — “Labels/Light|Dark/1..4”; AA-safe secondaries from the kit’s
//    “Vibrant (use plus lighter/darker)” ramp (#737373 / #8A8A8A) ────────────
pub fn label(theme: Theme, level: Level) -> egui::Color32 {
    match (theme, level) {
        (Theme::Light, Level::Primary) => egui::Color32::from_rgb(38, 38, 38), // black 85% over white
        (Theme::Light, Level::Secondary) => egui::Color32::from_rgb(115, 115, 115), // #737373 — 4.8:1
        (Theme::Light, Level::Tertiary) => egui::Color32::from_rgb(178, 178, 178), // #B2B2B2 — decorative
        (Theme::Light, Level::Quaternary) => egui::Color32::from_rgb(217, 217, 217), // #D9D9D9
        (Theme::Dark, Level::Primary) => egui::Color32::from_rgb(245, 245, 245), // white 96% over #1E1E1E
        (Theme::Dark, Level::Secondary) => egui::Color32::from_rgb(138, 138, 138), // #8A8A8A — 5.2:1
        (Theme::Dark, Level::Tertiary) => egui::Color32::from_rgb(76, 76, 76), // #4C4C4C — decorative
        (Theme::Dark, Level::Quaternary) => egui::Color32::from_rgb(38, 38, 38), // #262626
    }
}

// ── Fills — “Fills/Light|Dark/1..5”, composited over the window background ──
pub fn fill(theme: Theme, level: FillLevel) -> egui::Color32 {
    match (theme, level) {
        (Theme::Light, FillLevel::Primary) => egui::Color32::from_rgb(230, 230, 230), // black 10%
        (Theme::Light, FillLevel::Secondary) => egui::Color32::from_rgb(235, 235, 235), // black 8%
        (Theme::Light, FillLevel::Tertiary) => egui::Color32::from_rgb(242, 242, 242), // black 5%
        (Theme::Light, FillLevel::Quaternary) => egui::Color32::from_rgb(247, 247, 247), // black 3%
        (Theme::Light, FillLevel::Quinary) => egui::Color32::from_rgb(250, 250, 250), // black 2%
        (Theme::Dark, FillLevel::Primary) => egui::Color32::from_rgb(52, 52, 52),     // white 10%
        (Theme::Dark, FillLevel::Secondary) => egui::Color32::from_rgb(48, 48, 48),   // white 8%
        (Theme::Dark, FillLevel::Tertiary) => egui::Color32::from_rgb(41, 41, 41),    // white 5%
        (Theme::Dark, FillLevel::Quaternary) => egui::Color32::from_rgb(37, 37, 37),  // white 3%
        (Theme::Dark, FillLevel::Quinary) => egui::Color32::from_rgb(35, 35, 35),     // white 2%
    }
}

// ── Separator — “Separators/Light #3C3C43 @29%”; dark = white hairline 15% ──
pub fn separator(theme: Theme) -> egui::Color32 {
    match theme {
        Theme::Light => egui::Color32::from_rgb(198, 198, 200), // #3C3C43 @29% over white
        Theme::Dark => egui::Color32::from_rgb(64, 64, 64),     // white @15% over #1E1E1E
    }
}

// ── System colors — “System Colors/Light|Dark” (macOS 27 values) ────────────
pub fn system(theme: Theme, hue: Hue) -> egui::Color32 {
    match (theme, hue) {
        (Theme::Light, Hue::Accent) => egui::Color32::from_rgb(0, 136, 255),    // Blue #0088FF
        (Theme::Light, Hue::Success) => egui::Color32::from_rgb(52, 199, 89),   // Green #34C759
        (Theme::Light, Hue::Warning) => egui::Color32::from_rgb(255, 141, 40),  // Orange #FF8D28
        (Theme::Light, Hue::Danger) => egui::Color32::from_rgb(255, 56, 60),    // Red #FF383C
        (Theme::Light, Hue::Neutral) => egui::Color32::from_rgb(142, 142, 147), // Gray #8E8E93
        (Theme::Dark, Hue::Accent) => egui::Color32::from_rgb(0, 145, 255),     // Blue #0091FF
        (Theme::Dark, Hue::Success) => egui::Color32::from_rgb(48, 209, 88),    // Green #30D158
        (Theme::Dark, Hue::Warning) => egui::Color32::from_rgb(255, 146, 48),   // Orange #FF9230
        (Theme::Dark, Hue::Danger) => egui::Color32::from_rgb(255, 66, 69),     // Red #FF4245
        (Theme::Dark, Hue::Neutral) => egui::Color32::from_rgb(152, 152, 157),  // Gray #98989D
    }
}

/// Tinted pill/banner pair: bg = system color @16% (light) / @18% (dark)
/// composited over the window background; fg is the AA-darkened (light) or
/// system (dark) text variant. Measured contrast: success 5.2:1 / 5.8:1,
/// warning 4.6:1 / 5.3:1, danger 4.8:1 / 4.7:1; accent is hyperlink-grade ≥3:1.
pub fn tint(theme: Theme, hue: Hue) -> Tint {
    match (theme, hue) {
        (Theme::Light, Hue::Accent) => Tint {
            bg: egui::Color32::from_rgb(214, 236, 255),
            fg: egui::Color32::from_rgb(0, 96, 192),
        },
        (Theme::Light, Hue::Success) => Tint {
            bg: egui::Color32::from_rgb(223, 246, 228),
            fg: egui::Color32::from_rgb(28, 115, 49),
        },
        (Theme::Light, Hue::Warning) => Tint {
            bg: egui::Color32::from_rgb(255, 237, 221),
            fg: egui::Color32::from_rgb(160, 91, 0),
        },
        (Theme::Light, Hue::Danger) => Tint {
            bg: egui::Color32::from_rgb(255, 223, 224),
            fg: egui::Color32::from_rgb(195, 30, 18),
        },
        (Theme::Light, Hue::Neutral) => Tint {
            bg: fill(Theme::Light, FillLevel::Tertiary),
            fg: egui::Color32::from_rgb(108, 108, 108), // darkened secondary — 4.9:1 on F3
        },
        (Theme::Dark, Hue::Accent) => Tint {
            bg: egui::Color32::from_rgb(25, 51, 71),
            fg: egui::Color32::from_rgb(10, 153, 255),
        },
        (Theme::Dark, Hue::Success) => Tint {
            bg: egui::Color32::from_rgb(34, 62, 40),
            fg: egui::Color32::from_rgb(48, 209, 88),
        },
        (Theme::Dark, Hue::Warning) => Tint {
            bg: egui::Color32::from_rgb(71, 51, 33),
            fg: egui::Color32::from_rgb(255, 146, 48),
        },
        (Theme::Dark, Hue::Danger) => Tint {
            bg: egui::Color32::from_rgb(71, 37, 37),
            fg: egui::Color32::from_rgb(255, 100, 103), // plus-lighter red — 4.7:1 on tint
        },
        (Theme::Dark, Hue::Neutral) => Tint {
            bg: fill(Theme::Dark, FillLevel::Tertiary),
            fg: egui::Color32::from_rgb(152, 152, 157), // system gray dark — 5.0:1 on F3
        },
    }
}

pub fn status_dot(theme: Theme, dot: Dot) -> egui::Color32 {
    match dot {
        Dot::Pending | Dot::Skipped => label(theme, Level::Tertiary),
        Dot::Running => system(theme, Hue::Warning),
        Dot::Success => system(theme, Hue::Success),
        Dot::Failed => system(theme, Hue::Danger),
    }
}

// ── Buttons — kit “Buttons/Content Area”: Prominent (Default) = accent fill +
//    white label; Bordered = control background + hairline + primary label ───
pub fn primary_button(theme: Theme) -> ButtonStyle {
    ButtonStyle {
        fill: system(theme, Hue::Accent),
        stroke: egui::Stroke::NONE,
        text: egui::Color32::WHITE,
    }
}

pub fn bordered_button(theme: Theme) -> ButtonStyle {
    match theme {
        Theme::Light => ButtonStyle {
            fill: WINDOW_BG_LIGHT,
            stroke: egui::Stroke::new(1.0_f32, separator(Theme::Light)),
            text: label(Theme::Light, Level::Primary),
        },
        Theme::Dark => ButtonStyle {
            fill: fill(Theme::Dark, FillLevel::Primary),
            stroke: egui::Stroke::new(1.0_f32, separator(Theme::Dark)),
            text: label(Theme::Dark, Level::Primary),
        },
    }
}

// ── Cards — “Group Boxes”: fill = Fill Tertiary (5%) + hairline, radius 10 ──
pub fn card(theme: Theme) -> (egui::Color32, egui::Color32) {
    (fill(theme, FillLevel::Tertiary), separator(theme))
}

// ── Window shadow — “Alerts/⌘/Light|Dark/Background” ─────────────────────────
pub fn window_shadow(theme: Theme) -> egui::Shadow {
    match theme {
        Theme::Light => egui::Shadow {
            offset: [0, 18],
            blur: 46,
            spread: 0,
            color: egui::Color32::from_black_alpha(64), // 25%
        },
        Theme::Dark => egui::Shadow {
            offset: [0, 18],
            blur: 48,
            spread: 0,
            color: egui::Color32::from_black_alpha(115), // 45%
        },
    }
}

pub fn popup_shadow(theme: Theme) -> egui::Shadow {
    egui::Shadow {
        offset: [0, 8],
        blur: 24,
        spread: 0,
        color: egui::Color32::from_black_alpha(if theme == Theme::Dark { 70 } else { 38 }),
    }
}
