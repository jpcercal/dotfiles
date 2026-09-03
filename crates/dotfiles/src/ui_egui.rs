use crate::upgrade::Consent;
use crate::ui_theme;
use crossbeam_channel::{unbounded, Receiver, Sender};
use dotfiles_core::gates;
use dotfiles_core::paths::Paths;
use dotfiles_core::probes::Section;
use dotfiles_core::steps::{LogStream, PipelineEvent};
use eframe::egui;
use eframe::egui::Widget;
use std::path::PathBuf;
use std::thread;

pub use crate::ui_theme::Theme;

// ─────────────────────────────────────────────────────────────────────────────
// Typography — single source of truth, 13pt base + per-section factors
// https://developer.apple.com/design/human-interface-guidelines/typography
// Every size is `BASE_PT * factor` so a single base change scales the whole app.
// Factors are derived from the HIG itself (macOS built-in table, platform defaults,
// optical-size threshold, tracking table). Each “section” of the HIG doc gets a factor.
// ─────────────────────────────────────────────────────────────────────────────
#[allow(dead_code)]
pub mod typography {
    /// HIG macOS default — `NSFont.systemFontSize` is 13pt, minimum 10pt.
    /// All type is derived from this. Change here and every style scales.
    pub const BASE_PT: f32 = 13.0;

    // ── HIG § “Text styles” → macOS built-in table (144 ppi, HIG “Specifications”) ──
    // Weight/Size/Leading as in the doc; factor = size / BASE_PT
    pub const LARGE_TITLE: f32 = 26.0 / BASE_PT; // 2.0 — Large Title 26/32 Regular/Bold
    pub const TITLE1: f32 = 22.0 / BASE_PT;      // 1.692 — Title 1 22/26 Regular/Bold
    pub const TITLE2: f32 = 17.0 / BASE_PT;      // 1.307 — Title 2 17/22 Regular/Bold
    pub const TITLE3: f32 = 15.0 / BASE_PT;      // 1.154 — Title 3 15/20 Regular/Semibold
    pub const HEADLINE: f32 = 13.0 / BASE_PT;    // 1.0   — Headline 13/16 Bold → Heavy emphasized
    pub const BODY: f32 = 13.0 / BASE_PT;        // 1.0   — Body 13/16 Regular → Semibold
    pub const CALLOUT: f32 = 12.0 / BASE_PT;     // 0.923 — Callout 12/15 Regular → Semibold
    pub const SUBHEADLINE: f32 = 11.0 / BASE_PT; // 0.846 — Subheadline 11/14 Regular → Semibold
    pub const FOOTNOTE: f32 = 10.0 / BASE_PT;    // 0.769 — Footnote 10/13 Regular → Semibold
    pub const CAPTION1: f32 = 12.0 / BASE_PT;    // 0.923 — Caption 1 (iOS parity, used for badges)
    pub const CAPTION2: f32 = 11.0 / BASE_PT;    // 0.846 — Caption 2 — badge secondary

    // ── HIG § “Ensuring legibility” — platform defaults/minimums ──
    pub const DEFAULT_MACOS: f32 = 13.0 / BASE_PT; // 1.0
    pub const MINIMUM_MACOS: f32 = 10.0 / BASE_PT; // 0.769 — never go below
    pub const DEFAULT_IOS: f32 = 17.0 / BASE_PT;   // 1.307 — reference only
    pub const MINIMUM_IOS: f32 = 11.0 / BASE_PT;   // 0.846

    // ── HIG § “Optical sizes” — SF Pro Text ≤19pt, Display ≥20pt ──
    // At BASE 13 we are in Text (≤19). Factors ≥ 20/13 ≈1.538 would use Display.
    pub const OPTICAL_THRESHOLD_PT: f32 = 20.0;
    pub const OPTICAL_THRESHOLD_FACTOR: f32 = OPTICAL_THRESHOLD_PT / BASE_PT; // 1.538

    // ── HIG § “Tracking values” (SF Pro) — 6pt +41 … 13pt −6 … 17pt −26 etc ──
    // In a running app the variable font interpolates tracking automatically; for mockups
    // apply the table. We expose the HIG tracking for the sizes we use, to be applied via
    // `RichText::extra_letter_spacing` where egui supports it.
    pub const TRACKING_BODY_13: f32 = -0.08; // 13pt −6/1000em
    pub const TRACKING_SMALL_11: f32 = 0.06;  // 11pt +6/1000em
    pub const TRACKING_CAPTION_12: f32 = 0.0; // 12pt 0

    #[inline]
    pub fn pt(factor: f32) -> f32 {
        BASE_PT * factor
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Theme — dark / light / system, persisted in application metadata
// HIG: respect system appearance by default, let user override, persist choice.
// Stored at ~/.local/state/dotfiles-updater/ui.json  { "theme": "system"|"light"|"dark" }
// ─────────────────────────────────────────────────────────────────────────────
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ThemePreference {
    #[serde(rename = "system")]
    System,
    #[serde(rename = "light")]
    Light,
    #[serde(rename = "dark")]
    Dark,
}
impl Default for ThemePreference {
    fn default() -> Self {
        Self::System
    }
}

fn theme_prefs_path() -> std::path::PathBuf {
    // Application metadata — alongside state.json
    Paths::detect().state_dir.join("ui.json")
}

pub fn load_theme_preference() -> ThemePreference {
    let p = theme_prefs_path();
    if let Ok(s) = std::fs::read_to_string(&p) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&s) {
            if let Some(t) = v.get("theme").and_then(|x| x.as_str()) {
                return match t {
                    "light" => ThemePreference::Light,
                    "dark" => ThemePreference::Dark,
                    _ => ThemePreference::System,
                };
            }
            // also support direct string
            if let Ok(pref) = serde_json::from_str::<ThemePreference>(&s) {
                return pref;
            }
        }
        if let Ok(pref) = serde_json::from_str::<ThemePreference>(&s) {
            return pref;
        }
    }
    ThemePreference::System
}

pub fn save_theme_preference(pref: ThemePreference) {
    let p = theme_prefs_path();
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let v = serde_json::json!({ "theme": match pref {
        ThemePreference::Light => "light",
        ThemePreference::Dark => "dark",
        ThemePreference::System => "system",
    }});
    let _ = std::fs::write(&p, serde_json::to_string_pretty(&v).unwrap_or_default());
}

pub fn detect_system_theme() -> Theme {
    match dark_light::detect() {
        Ok(dark_light::Mode::Dark) => Theme::Dark,
        Ok(dark_light::Mode::Light) => Theme::Light,
        _ => Theme::Light,
    }
}

pub fn resolve_theme(pref: ThemePreference) -> Theme {
    match pref {
        ThemePreference::Light => Theme::Light,
        ThemePreference::Dark => Theme::Dark,
        ThemePreference::System => detect_system_theme(),
    }
}

fn theme_icon(pref: ThemePreference) -> &'static str {
    match pref {
        ThemePreference::System => "🖥",
        ThemePreference::Light => "☀",
        ThemePreference::Dark => "🌙",
    }
}

fn next_theme_preference(pref: ThemePreference) -> ThemePreference {
    match pref {
        ThemePreference::System => ThemePreference::Light,
        ThemePreference::Light => ThemePreference::Dark,
        ThemePreference::Dark => ThemePreference::System,
    }
}

// SF Symbol for dock + header — HIG SF Symbols, “arrow.triangle.2.circlepath” (system update)
// https://developer.apple.com/design/human-interface-guidelines/sf-symbols
// We render it as a painted circular arrow so it never tofu, and reuse the same shape for the dock icon.
fn dock_icon_data() -> std::sync::Arc<egui::IconData> {
    // 512×512, transparent → blue rounded-square → white circular arrow (SF Symbol style)
    let size: usize = 512;
    let mut rgba = vec![0u8; size * size * 4];
    let cx = size as f32 / 2.0;
    let cy = size as f32 / 2.0;
    let outer_r: f32 = 96.0; // corner radius of the squircle
    let half = size as f32 / 2.0 - 32.0;
    for y in 0..size {
        for x in 0..size {
            let fx = x as f32 + 0.5;
            let fy = y as f32 + 0.5;
            // rounded-rect (squircle) test for macOS icon shape
            let dx = (fx - cx).abs() - (half - outer_r);
            let dy = (fy - cy).abs() - (half - outer_r);
            let inside = if dx <= 0.0 || dy <= 0.0 {
                true
            } else {
                dx * dx + dy * dy <= outer_r * outer_r
            };
            if inside {
                let idx = (y * size + x) * 4;
                rgba[idx] = 0;
                rgba[idx + 1] = 122;
                rgba[idx + 2] = 255;
                rgba[idx + 3] = 255;
            }
        }
    }
    // white circular arrow — two arcs with arrowheads (SF Symbol “arrow.triangle.2.circlepath” simplified)
    // Outer arc radius 120, thickness 18, gap 18° at top and bottom, arrowheads as small triangles.
    let r: f32 = 120.0;
    let thickness: f32 = 18.0;
    for y in 0..size {
        for x in 0..size {
            let fx = x as f32 - cx;
            let fy = y as f32 - cy;
            let dist = (fx * fx + fy * fy).sqrt();
            if (dist - r).abs() > thickness / 2.0 {
                continue;
            }
            let ang = fy.atan2(fx).to_degrees(); // -180..180, 0 = +X (3 o’clock), CCW
            // normalize to 0..360 where 0 = 3 o’clock, 90 = 12 o’clock? Actually atan2 90 = 12? Let's just use standard.
            // We want gaps at ~90° (12 o’clock) and 270° (6 o’clock) for the two arrows.
            // Create two arcs: arc A 100°..260°, arc B 280°..80° (wrapping)
            let norm = if ang < 0.0 { ang + 360.0 } else { ang };
            let in_arc_a = norm > 100.0 && norm < 260.0;
            let in_arc_b = norm > 280.0 || norm < 80.0;
            if !(in_arc_a || in_arc_b) {
                continue;
            }
            let idx = (y * size + x) * 4;
            // if already blue, overdraw white
            rgba[idx] = 255;
            rgba[idx + 1] = 255;
            rgba[idx + 2] = 255;
            rgba[idx + 3] = 255;
        }
    }
    // Arrowheads — two small white triangles at the gaps
    // Top gap arrow (pointing clockwise, roughly at 90°)
    // Bottom gap arrow (at 270°)
    // For simplicity, draw two 24px triangles via barycentric test
    let draw_tri = |rgba: &mut [u8], p0: (f32, f32), p1: (f32, f32), p2: (f32, f32)| {
        let min_x = p0.0.min(p1.0).min(p2.0).floor() as i32;
        let max_x = p0.0.max(p1.0).max(p2.0).ceil() as i32;
        let min_y = p0.1.min(p1.1).min(p2.1).floor() as i32;
        let max_y = p0.1.max(p1.1).max(p2.1).ceil() as i32;
        let area = (p1.0 - p0.0) * (p2.1 - p0.1) - (p2.0 - p0.0) * (p1.1 - p0.1);
        if area == 0.0 {
            return;
        }
        for y in min_y..=max_y {
            for x in min_x..=max_x {
                let px = x as f32 + 0.5;
                let py = y as f32 + 0.5;
                let w0 = (p1.0 - p0.0) * (py - p0.1) - (p1.1 - p0.1) * (px - p0.0);
                let w1 = (p2.0 - p1.0) * (py - p1.1) - (p2.1 - p1.1) * (px - p1.0);
                let w2 = (p0.0 - p2.0) * (py - p2.1) - (p0.1 - p2.1) * (px - p2.0);
                if (w0 >= 0.0 && w1 >= 0.0 && w2 >= 0.0) || (w0 <= 0.0 && w1 <= 0.0 && w2 <= 0.0) {
                    if x >= 0 && x < size as i32 && y >= 0 && y < size as i32 {
                        let idx = (y as usize * size + x as usize) * 4;
                        rgba[idx] = 255;
                        rgba[idx + 1] = 255;
                        rgba[idx + 2] = 255;
                        rgba[idx + 3] = 255;
                    }
                }
            }
        }
    };
    // Top arrow (12 o’clock, pointing right/clockwise)
    let top_center_ang: f32 = 90.0_f32.to_radians();
    let top_cx = cx + r * top_center_ang.cos();
    let top_cy = cy + r * top_center_ang.sin();
    // triangle points around gap
    let t0 = (top_cx + 18.0, top_cy - 8.0);
    let t1 = (top_cx + 18.0, top_cy + 14.0);
    let t2 = (top_cx - 14.0, top_cy + 3.0);
    draw_tri(&mut rgba, t0, t1, t2);
    // Bottom arrow (6 o’clock)
    let bot_ang: f32 = 270.0_f32.to_radians();
    let bot_cx = cx + r * bot_ang.cos();
    let bot_cy = cy + r * bot_ang.sin();
    let b0 = (bot_cx - 18.0, bot_cy + 8.0);
    let b1 = (bot_cx - 18.0, bot_cy - 14.0);
    let b2 = (bot_cx + 14.0, bot_cy - 3.0);
    draw_tri(&mut rgba, b0, b1, b2);
    std::sync::Arc::new(egui::IconData {
        rgba,
        width: size as u32,
        height: size as u32,
    })
}

// macOS San Francisco fonts + modern system styling
fn apply_macos_appearance(ctx: &egui::Context, theme: Theme) {
    // ── Fonts: try to load SF Pro (SFNS) and SF Mono directly from the system ──
    // egui will fall back to its built-in Inter/Emoji if the files are missing (e.g. sandbox).
    let mut fonts = egui::FontDefinitions::default();

    // Proportional → SF Pro (SFNS) — the macOS system font. 13pt is the macOS standard.
    if let Ok(data) = std::fs::read("/System/Library/Fonts/SFNS.ttf") {
        fonts
            .font_data
            .insert("SFPro".to_owned(), std::sync::Arc::new(egui::FontData::from_owned(data)));
        if let Some(fam) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
            fam.insert(0, "SFPro".to_owned());
        }
    } else if let Ok(data) = std::fs::read("/System/Library/Fonts/Helvetica.ttc") {
        // tiny TTC fallback — only index 0 will be used by ab_glyph, still renders correctly for 13pt
        fonts
            .font_data
            .insert("Helvetica".to_owned(), std::sync::Arc::new(egui::FontData::from_owned(data)));
        if let Some(fam) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
            fam.insert(0, "Helvetica".to_owned());
        }
    }

    // Monospace → SF Mono (and Menlo as secondary) — Xcode / Terminal style
    if let Ok(data) = std::fs::read("/System/Library/Fonts/SFNSMono.ttf") {
        fonts
            .font_data
            .insert("SFMono".to_owned(), std::sync::Arc::new(egui::FontData::from_owned(data)));
        if let Some(fam) = fonts.families.get_mut(&egui::FontFamily::Monospace) {
            fam.insert(0, "SFMono".to_owned());
        }
    }
    // Ensure Apple Color Emoji stays available for the few glyphs we still use (● etc.)
    // egui's default already bundles NotoEmoji, which is a fine fallback.

    ctx.set_fonts(fonts);

    // ── Text styles: Apple HIG Typography — by the book ─────────────────
    // https://developer.apple.com/design/human-interface-guidelines/typography
    // Implements every normative section + sublinks for components used by this app:
    //
    // • Typeface family: SF Pro (Text/Display), SF Compact, SF Mono (HIG § “San Francisco (SF)”, § “New York”)
    //   – Variable font with 9 weights Ultralight(100)→Black(900) and 4 widths (incl. Condensed/Expanded).
    //     SF Symbols weights match SF exactly, so symbols align with adjacent text at any size.
    //   – Optical sizes: SF Pro Text optimizes glyphs for ≤19pt (body, lists), SF Pro Display for ≥20pt (large titles).
    //     The variable format (fvar `opsz` axis) interpolates between them; we load the single variable SFNS.ttf
    //     which the system interpolates to the point size — no discrete file swap needed unless a design tool
    //     lacks variable support (HIG “Variable fonts support optical sizing”).
    // • Hierarchy (§ “Conveying hierarchy”): weight/size/color create levels. Single typeface + few styles, not
    //     fragmented fonts. Relative hierarchy preserved.
    // • Legibility (§ “Ensuring legibility”): platform defaults/monimums — macOS 13pt/10pt, iOS 17/11, etc. Test in
    //     contexts, avoid Ultralight/Thin/Light at small sizes → prefer Regular/Medium/Semibold/Bold for ≤13pt.
    // • System text styles (§ “Text styles”): each style = weight+size+leading(+tracking). We map egui TextStyles
    //     to the macOS built-in table at 144ppi (HIG “macOS built-in text styles”):
    //       Large Title 26/32, Title1 22/26, Title2 17/22, Title3 15/20, Headline 13/16 (Bold), Body 13/16,
    //       Callout 12/15, Subheadline 11/14, Footnote 10/13 — all Regular except Headline Bold.
    // • Tracking (§ “Tracking values”): SF Pro tracking per size (6pt +41 → 13pt −6 → 17pt −26 etc). In a running
    //     app the variable font adjusts tracking automatically at each point size; for mockups you’d apply the table.
    //     We store the HIG tracking as `extra_letter_spacing` where egui supports it (RichText), and rely on the
    //     variable font for the rest.
    // • Platform (§ “Platform considerations → macOS”): SF Pro is system font, NY only for Catalyst, **no Dynamic
    //     Type on macOS**. To match standard controls we use the NSFont dynamic variants:
    //       controlContentFont(13) → Button/Control, labelFont(13) → Label/Body, messageFont(13) → Alert message,
    //       menuFont(14)/menuBarFont(14), titleBarFont(13), paletteFont(11), toolTipsFont(11), userFont(13),
    //       userFixedPitchFont(11) → Mono log, systemFont(13)/boldSystemFont(13). Table below.
    // • Components used by this app (sublinks):
    //     – Windows/Panels → titleBarFont 13 Semibold
    //     – Alerts/Dialogs (consent) → Headline/Bold 13 + Message/Body 13, secondary Callout 12/Subheadline 11
    //     – Buttons (push, default) → controlContentFont 13; default button Semibold, tinted systemBlue #007AFF
    //     – Sidebars (progress left rail) → labelFont 13 + Subheadline 11 for durations/notes
    //     – Lists & Tables (update cards) → Body 13 for names, Callout/Subheadline 11-12 for versions, Caption 11 for badges
    //     – Text views / Scroll views (live log) → SF Mono 11 (userFixedPitch), leading 13
    //
    // Implementation: Proportional → SF Pro (SFNS), Monospace → SF Mono, both loaded from /System/Library/Fonts
    // with Helvetica/Menlo fallbacks. Variable optical sizing is left to the system (fvar opsz). Weights are
    // expressed via `strong` (Semibold/Bold) on the same variable file; egui fakes bold when no separate file is present,
    // which matches HIG’s “prefer Regular/Medium/Semibold/Bold” guidance.
    let mut style = (*ctx.style()).clone();
    use egui::{FontFamily as FF, FontId, TextStyle as TS};
    // HIG macOS built-in — exact point sizes, no custom 50% scale (HIG says test legibility at default/minimum;
    // macOS default 13pt / minimum 10pt — scaling is only via Accessibility > Display > Text size, not arbitrary).
    style.text_styles.insert(TS::Heading, FontId::new(typography::pt(typography::TITLE3), FF::Proportional)); // Title 3 — factor TITLE3
    style.text_styles.insert(TS::Body, FontId::new(typography::pt(typography::BODY), FF::Proportional)); // Body — factor BODY
    style.text_styles.insert(TS::Button, FontId::new(typography::pt(typography::BODY), FF::Proportional)); // controlContentFont — factor BODY
    style.text_styles.insert(TS::Small, FontId::new(typography::pt(typography::SUBHEADLINE), FF::Proportional)); // Subheadline — factor SUBHEADLINE
    style.text_styles.insert(TS::Monospace, FontId::new(typography::pt(typography::SUBHEADLINE), FF::Monospace)); // SF Mono 11 — factor SUBHEADLINE
    style.text_styles.insert(TS::Name("Caption".into()), FontId::new(typography::pt(typography::CAPTION2), FF::Proportional)); // Caption — factor CAPTION2
    // Additional HIG styles we use explicitly via RichText::size() below are derived from the same table:
    // Title2 17 for large window titles, Callout 12 for gate details, Footnote 10 for durations — see inline comments.

    // ── Visuals — macOS 27 UI Kit tokens (single source: ui_theme.rs) ───────
    // Opaque window/panel surfaces (#FFFFFF / #1E1E1E), the kit’s 5-level fill
    // ladder composited over the window background, hairline separators,
    // semantic system colors per appearance, and the kit’s alert window shadow.
    // All text pairs are AA-validated — see the ui_theme.rs header.
    let mut visuals = match theme {
        Theme::Dark => egui::Visuals::dark(),
        Theme::Light => egui::Visuals::light(),
    };
    visuals.dark_mode = theme == Theme::Dark;
    visuals.panel_fill = ui_theme::window_bg(theme);
    visuals.window_fill = ui_theme::window_bg(theme);
    visuals.extreme_bg_color = ui_theme::fill(theme, ui_theme::FillLevel::Quinary);
    visuals.faint_bg_color = ui_theme::fill(theme, ui_theme::FillLevel::Primary);
    visuals.code_bg_color = ui_theme::fill(theme, ui_theme::FillLevel::Secondary);
    visuals.window_stroke = egui::Stroke::new(1.0_f32, ui_theme::separator(theme));
    visuals.window_corner_radius = egui::CornerRadius::same(12);
    visuals.window_shadow = ui_theme::window_shadow(theme);
    visuals.popup_shadow = ui_theme::popup_shadow(theme);
    for w in [
        &mut visuals.widgets.noninteractive,
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
        &mut visuals.widgets.open,
    ] {
        w.corner_radius = egui::CornerRadius::same(8);
        w.bg_stroke = egui::Stroke::new(1.0_f32, ui_theme::separator(theme));
    }
    visuals.widgets.noninteractive.bg_fill = ui_theme::window_bg(theme);
    visuals.widgets.noninteractive.weak_bg_fill = ui_theme::fill(theme, ui_theme::FillLevel::Quinary);
    // noninteractive.fg_stroke is egui’s global text color (Visuals::text_color)
    visuals.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0_f32, ui_theme::label(theme, ui_theme::Level::Primary));
    for w in [
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
        &mut visuals.widgets.open,
    ] {
        w.fg_stroke = egui::Stroke::new(1.0_f32, ui_theme::label(theme, ui_theme::Level::Primary));
    }
    match theme {
        Theme::Dark => {
            visuals.widgets.inactive.bg_fill = ui_theme::fill(theme, ui_theme::FillLevel::Primary);
            visuals.widgets.inactive.weak_bg_fill = ui_theme::fill(theme, ui_theme::FillLevel::Primary);
            // idle → hover → pressed ladder: white 10% → 13% → 18% over #1E1E1E
            visuals.widgets.hovered.bg_fill = egui::Color32::from_rgb(59, 59, 59);
            visuals.widgets.hovered.weak_bg_fill = egui::Color32::from_rgb(59, 59, 59);
            visuals.widgets.open.bg_fill = egui::Color32::from_rgb(59, 59, 59);
            visuals.widgets.open.weak_bg_fill = egui::Color32::from_rgb(59, 59, 59);
            visuals.widgets.active.bg_fill = egui::Color32::from_rgb(71, 71, 71);
            visuals.widgets.active.weak_bg_fill = egui::Color32::from_rgb(71, 71, 71);
        }
        Theme::Light => {
            visuals.widgets.inactive.bg_fill = ui_theme::window_bg(theme);
            visuals.widgets.inactive.weak_bg_fill = ui_theme::window_bg(theme);
            // idle → hover → pressed ladder: white → 5% black → 10% black
            visuals.widgets.hovered.bg_fill = ui_theme::fill(theme, ui_theme::FillLevel::Tertiary);
            visuals.widgets.hovered.weak_bg_fill = ui_theme::fill(theme, ui_theme::FillLevel::Tertiary);
            visuals.widgets.open.bg_fill = ui_theme::fill(theme, ui_theme::FillLevel::Tertiary);
            visuals.widgets.open.weak_bg_fill = ui_theme::fill(theme, ui_theme::FillLevel::Tertiary);
            visuals.widgets.active.bg_fill = ui_theme::fill(theme, ui_theme::FillLevel::Primary);
            visuals.widgets.active.weak_bg_fill = ui_theme::fill(theme, ui_theme::FillLevel::Primary);
        }
    }
    visuals.selection.bg_fill = ui_theme::system(theme, ui_theme::Hue::Accent);
    visuals.selection.stroke = egui::Stroke::new(1.0_f32, egui::Color32::WHITE);
    visuals.hyperlink_color = ui_theme::system(theme, ui_theme::Hue::Accent);
    visuals.error_fg_color = ui_theme::system(theme, ui_theme::Hue::Danger);
    visuals.warn_fg_color = ui_theme::system(theme, ui_theme::Hue::Warning);
    visuals.override_text_color = None;
    visuals.weak_text_color = Some(ui_theme::label(theme, ui_theme::Level::Secondary));
    style.visuals = visuals;

    style.spacing.item_spacing = egui::vec2(10.0, 10.0);
    style.spacing.window_margin = egui::Margin::same(16);
    style.spacing.button_padding = egui::vec2(16.0, 9.0);
    style.spacing.indent = 16.0;
    style.spacing.menu_margin = egui::Margin::same(8);
    ctx.set_style(style);
}

// ---------------------------------------------------------------------------
// Shared widget builders — macOS 27 UI Kit specs from ui_theme.rs
// ---------------------------------------------------------------------------

/// Pill/banner frame from an AA-validated tint pair (bg + fg).
fn tint_frame(t: ui_theme::Tint) -> egui::Frame {
    egui::Frame::new()
        .fill(t.bg)
        .stroke(egui::Stroke::new(1.0_f32, t.fg.gamma_multiply(0.35)))
        .corner_radius(8)
}

/// Button from a kit spec — Prominent (accent + white label) or
/// Bordered (control background + hairline + primary label).
fn kit_button(
    ui: &mut egui::Ui,
    text: &str,
    s: ui_theme::ButtonStyle,
    size: Option<[f32; 2]>,
) -> egui::Response {
    let btn = egui::Button::new(
        egui::RichText::new(text)
            .size(typography::pt(typography::BODY))
            .strong()
            .color(s.text),
    )
    .fill(s.fill)
    .stroke(s.stroke)
    .corner_radius(8);
    match size {
        Some(sz) => ui.add_sized(sz, btn),
        None => ui.add(btn),
    }
}

// ---------------------------------------------------------------------------
// Consent window — polished
// ---------------------------------------------------------------------------

#[allow(dead_code)]
pub fn show_consent(summary: &str, sections: &[Section], paths: &Paths) -> anyhow::Result<Consent> {
    let summary = summary.to_string();
    let sections = sections.to_vec();
    let paths_clone = paths.clone();

    let (tx, rx) = std::sync::mpsc::channel::<Consent>();

    let result = eframe::run_native(
        "dotfiles — update available",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([560.0, 480.0])
                .with_min_inner_size([560.0, 480.0]),
            ..Default::default()
        },
        Box::new(|cc| {
            let pref = load_theme_preference();
            let theme = resolve_theme(pref);
            apply_macos_appearance(&cc.egui_ctx, theme);
            Ok(Box::new(ConsentApp {
                summary,
                sections,
                paths: paths_clone,
                tx: Some(tx),
                gate_status: compute_gate_status(),
                theme_preference: pref,
                theme,
            }))
        }),
    );

    match result {
        Ok(_) => match rx.try_recv() {
            Ok(c) => Ok(c),
            Err(_) => Ok(Consent::Postpone),
        },
        Err(e) => {
            eprintln!("egui consent failed: {} — falling back to terminal", e);
            Ok(Consent::Postpone)
        }
    }
}

fn compute_gate_status() -> Vec<gates::GateResult> {
    let battery = gates::battery_info();
    let free = gates::free_disk_gb();
    let state = dotfiles_core::state::State::load(&Paths::detect().state_file).unwrap_or_default();
    vec![
        gates::gate_power(&battery),
        gates::gate_network(),
        gates::gate_disk(free),
        gates::gate_pkgmgr(),
        gates::gate_schedule(&state),
        gates::gate_dialog_cooldown(&state),
    ]
}

#[allow(dead_code)]
struct ConsentApp {
    summary: String,
    sections: Vec<Section>,
    #[allow(dead_code)]
    paths: Paths,
    tx: Option<std::sync::mpsc::Sender<Consent>>,
    gate_status: Vec<gates::GateResult>,
    theme_preference: ThemePreference,
    theme: Theme,
}

impl eframe::App for ConsentApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let total_updates: usize = self.sections.iter().map(|s| s.count).sum();
        let sources_with_updates = self.sections.iter().filter(|s| s.count > 0).count();
        let has_gate_fail = self
            .gate_status
            .iter()
            .any(|g| !g.ok && g.name != "schedule" && g.name != "dialog_cooldown");
        let all_gates_ok = !has_gate_fail;

        // ── Bottom bar: actions (always visible) ──────────────────────────
        egui::TopBottomPanel::bottom("consent_actions")
            .frame(
                egui::Frame::new()
                    .fill(ctx.style().visuals.panel_fill)
                    .inner_margin(egui::Margin {
                        left: 16,
                        right: 16,
                        top: 12,
                        bottom: 12,
                    })
                    .stroke(egui::Stroke::new(
                        1.0_f32,
                        ctx.style().visuals.widgets.noninteractive.bg_stroke.color,
                    )),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    // Left: hint — wrap so it never pushes buttons off-screen
                    if has_gate_fail {
                        tint_frame(ui_theme::tint(self.theme, ui_theme::Hue::Warning))
                            .inner_margin(egui::Margin::symmetric(10, 6))
                            .show(ui, |ui| {
                                ui.add(egui::Label::new(
                                    egui::RichText::new("Some checks failed — will still update if you proceed")
                                        .size(typography::pt(typography::SUBHEADLINE))
                                        .color(ui_theme::tint(self.theme, ui_theme::Hue::Warning).fg),
                                ).wrap());
                            });
                    } else {
                        ui.add(egui::Label::new(
                            egui::RichText::new("Press Enter to update • Esc to postpone")
                                .size(typography::pt(typography::SUBHEADLINE))
                                .weak()
                                .italics(),
                        ).wrap());
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // Right: buttons — primary is trailing (rightmost) per HIG
                        if kit_button(ui, "Update Now", ui_theme::primary_button(self.theme), Some([124.0, 32.0])).clicked() {
                            eprintln!("[dotfiles] Update Now clicked");
                            if let Some(tx) = self.tx.take() {
                                let _ = tx.send(Consent::Proceed);
                            }
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                        if kit_button(ui, "Postpone", ui_theme::bordered_button(self.theme), Some([124.0, 32.0])).clicked() {
                            eprintln!("[dotfiles] Postpone clicked");
                            if let Some(tx) = self.tx.take() {
                                let _ = tx.send(Consent::Postpone);
                            }
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                    });
                });
            });

        // ── Central scroll content ────────────────────────────────────────
        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false; 2])
                .show(ui, |ui| {
                    ui.add_space(4.0);

                    // Header: icon + title + total badge
                    ui.horizontal(|ui| {
                        // Icon box — SF Symbol “arrow.triangle.2.circlepath” (HIG SF Symbols)
                        // https://developer.apple.com/design/human-interface-guidelines/sf-symbols
                        // Same glyph is used for the Dock icon (dock_icon_data) so window + Dock match.
                        let accent_tint = ui_theme::tint(self.theme, ui_theme::Hue::Accent);
                        egui::Frame::new()
                            .fill(accent_tint.bg)
                            .stroke(egui::Stroke::new(1.0_f32, ui_theme::separator(self.theme)))
                            .corner_radius(10)
                            .inner_margin(6)
                            .show(ui, |ui| {
                                ui.label(
                                    egui::RichText::new("↻")
                                        .size(22.0)
                                        .color(accent_tint.fg),
                                );
                            });
                        ui.add_space(8.0);
                        ui.vertical(|ui| {
                            ui.label(
                                egui::RichText::new("Updates ready")
                                    .size(typography::pt(typography::TITLE3)) // HIG Title 3 15pt Semibold / 20 leading
                                    .strong()
                                    .line_height(Some(20.0)),
                            );
                            ui.label(
                                egui::RichText::new("Review what will change — you choose when to run it")
                                    .size(typography::pt(typography::SUBHEADLINE) * 1.25) // HIG Subheadline 11pt Regular / 14 leading
                                    .weak(),
                            );
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            // Total badge
                            let success_tint = ui_theme::tint(self.theme, ui_theme::Hue::Success);
                            let (badge_fill, badge_fg, badge_text) = if total_updates > 0 {
                                (
                                    success_tint.bg,
                                    success_tint.fg,
                                    format!("{} updates", total_updates),
                                )
                            } else {
                                let neutral = ui_theme::tint(self.theme, ui_theme::Hue::Neutral);
                                (neutral.bg, neutral.fg, "Up to date".to_string())
                            };
                            egui::Frame::new()
                                .fill(badge_fill)
                                .stroke(egui::Stroke::new(1.0_f32, ui_theme::separator(self.theme)))
                                .corner_radius(20)
                                .inner_margin(egui::Margin::symmetric(12, 6))
                                .show(ui, |ui| {
                                    ui.label(egui::RichText::new(badge_text).size(typography::pt(typography::CALLOUT) * 1.05).strong().color(badge_fg));
                                    if sources_with_updates > 0 {
                                        ui.label(
                                            egui::RichText::new(format!("{} sources", sources_with_updates))
                                                .size(typography::pt(typography::SUBHEADLINE) * 1.05)
                                                .color(badge_fg),
                                        );
                                    }
                                });
                            // Theme toggle — System → Light → Dark, persisted
                            ui.add_space(6.0);
                            let icon = theme_icon(self.theme_preference);
                            let tip = format!("Theme: {:?} — click to cycle (System follows macOS)", self.theme_preference);
                            if ui
                                .add(egui::Button::new(egui::RichText::new(icon).size(16.0)).frame(false))
                                .on_hover_text(tip)
                                .clicked()
                            {
                                let next = next_theme_preference(self.theme_preference);
                                self.theme_preference = next;
                                self.theme = resolve_theme(next);
                                save_theme_preference(next);
                                apply_macos_appearance(ctx, self.theme);
                                // re-apply progress tighter spacing if needed (kept in style)
                                ctx.request_repaint();
                            }
                        });
                    });

                    ui.add_space(14.0);

                    // Gate summary — compact pill when all ok, list when any fail
                    if all_gates_ok {
                        let ok_tint = ui_theme::tint(self.theme, ui_theme::Hue::Success);
                        egui::Frame::new()
                            .fill(ok_tint.bg)
                            .stroke(egui::Stroke::new(1.0_f32, ok_tint.fg.gamma_multiply(0.35)))
                            .corner_radius(8)
                            .inner_margin(egui::Margin::symmetric(10, 8))
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.label(
                                        egui::RichText::new("●")
                                            .size(9.0)
                                            .color(ok_tint.fg),
                                    );
                                    ui.label(
                                        egui::RichText::new("All system checks passed")
                                            .size(typography::pt(typography::CALLOUT) * 1.25)
                                            .strong()
                                            .color(ok_tint.fg),
                                    );
                                    ui.separator();
                                    // compact details inline, muted
                                    let details: Vec<String> = self
                                        .gate_status
                                        .iter()
                                        .filter(|g| g.name == "power" || g.name == "disk")
                                        .map(|g| g.reason.clone())
                                        .collect();
                                    ui.label(
                                        egui::RichText::new(details.join("  ·  "))
                                            .size(typography::pt(typography::SUBHEADLINE) * 1.15)
                                            .weak(),
                                    );
                                });
                            });
                    } else {
                        let warn_tint = ui_theme::tint(self.theme, ui_theme::Hue::Warning);
                        egui::Frame::new()
                            .fill(warn_tint.bg)
                            .stroke(egui::Stroke::new(1.0_f32, warn_tint.fg.gamma_multiply(0.35)))
                            .corner_radius(8)
                            .inner_margin(10)
                            .show(ui, |ui| {
                                ui.vertical(|ui| {
                                    ui.label(
                                        egui::RichText::new("Pre-flight checks")
                                            .size(typography::pt(typography::SUBHEADLINE))
                                            .strong()
                                            .color(warn_tint.fg),
                                    );
                                    ui.add_space(2.0);
                                    // Dots sit on the warning tint — use tint-fg variants for AA
                                    let ok_dot = ui_theme::tint(self.theme, ui_theme::Hue::Success).fg;
                                    let fail_dot = ui_theme::tint(self.theme, ui_theme::Hue::Danger).fg;
                                    for g in &self.gate_status {
                                        let dot_col = if g.ok { ok_dot } else { fail_dot };
                                        ui.horizontal(|ui| {
                                            ui.label(egui::RichText::new("●").size(8.0).color(dot_col));
                                            ui.label(
                                                egui::RichText::new(format!("{} — {}", g.name, g.reason))
                                                    .size(typography::pt(typography::SUBHEADLINE))
                                                    .color(if g.ok {
                                                        ui.visuals().weak_text_color()
                                                    } else {
                                                        ui_theme::tint(self.theme, ui_theme::Hue::Danger).fg
                                                    }),
                                            );
                                        });
                                    }
                                });
                            });
                    }

                    ui.add_space(12.0);
                    ui.separator();
                    ui.add_space(8.0);

                    // Section cards
                    for sec in &self.sections {
                        let has_items = !sec.items.is_empty();
                        let is_populated = sec.count > 0;
                        // Kit card: Fill Tertiary surface + hairline separator, no accent outline
                        let (card_fill, border_col) = ui_theme::card(self.theme);

                        egui::Frame::new()
                            .fill(card_fill)
                            .stroke(egui::Stroke::new(1.0_f32, border_col))
                            .corner_radius(10)
                            .inner_margin(egui::Margin { left: 12, right: 12, top: 10, bottom: 10 })
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.label(
                                        egui::RichText::new(&sec.title)
                                            .size(typography::pt(typography::BODY) * 1.05)
                                            .strong()
                                            .color(if is_populated {
                                                ui.visuals().text_color()
                                            } else {
                                                ui.visuals().weak_text_color()
                                            }),
                                    );
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        // count badge
                                        let (fill, fg) = if is_populated {
                                            (
                                                ui_theme::system(self.theme, ui_theme::Hue::Accent),
                                                egui::Color32::WHITE,
                                            )
                                        } else {
                                            (
                                                ui_theme::fill(self.theme, ui_theme::FillLevel::Quaternary),
                                                ui.visuals().weak_text_color(),
                                            )
                                        };
                                        egui::Frame::new()
                                            .fill(fill)
                                            .corner_radius(10)
                                            .inner_margin(egui::Margin::symmetric(8, 3))
                                            .show(ui, |ui| {
                                                ui.label(
                                                    egui::RichText::new(format!("{}", sec.count))
                                                        .size(typography::pt(typography::SUBHEADLINE))
                                                        .strong()
                                                        .color(fg),
                                                );
                                            });
                                    });
                                });

                                if has_items {
                                    ui.add_space(8.0);
                                    ui.separator();
                                    ui.add_space(6.0);
                                    // items list with subtle rows
                                    for item in &sec.items {
                                        // Split "name (old -> new)" to style arrow part muted
                                        let (name_part, ver_part) = if let Some(idx) = item.find(" (") {
                                            (&item[..idx], Some(&item[idx..]))
                                        } else {
                                            (item.as_str(), None)
                                        };
                                        egui::Frame::new()
                                            .fill(ui.visuals().faint_bg_color.gamma_multiply(0.0))
                                            .inner_margin(egui::Margin::symmetric(2, 2))
                                            .show(ui, |ui| {
                                                ui.horizontal(|ui| {
                                                    ui.label(
                                                        egui::RichText::new("·")
                                                            .size(typography::pt(typography::CALLOUT))
                                                            .color(ui.visuals().weak_text_color()),
                                                    );
                                                    ui.label(
                                                        egui::RichText::new(name_part)
                                                            .size(typography::pt(typography::CALLOUT) * 1.05)
                                                            .color(ui.visuals().text_color()),
                                                    );
                                                    if let Some(v) = ver_part {
                                                        ui.label(
                                                            egui::RichText::new(v)
                                                                .size(typography::pt(typography::SUBHEADLINE) * 1.05)
                                                                .color(ui.visuals().weak_text_color())
                                                                .monospace(),
                                                        );
                                                    }
                                                });
                                            });
                                    }
                                } else {
                                    ui.add_space(4.0);
                                    ui.label(
                                        egui::RichText::new(if sec.count == 0 {
                                            "No changes"
                                        } else {
                                            ""
                                        })
                                        .size(typography::pt(typography::SUBHEADLINE))
                                        .color(ui_theme::label(self.theme, ui_theme::Level::Tertiary))
                                        .italics(),
                                    );
                                }
                            });
                        ui.add_space(8.0);
                    }

                    if self.sections.is_empty() {
                        egui::Frame::new()
                            .fill(ui_theme::fill(self.theme, ui_theme::FillLevel::Tertiary))
                            .corner_radius(8)
                            .inner_margin(12)
                            .show(ui, |ui| {
                                ui.label(egui::RichText::new(&self.summary).size(typography::pt(typography::CALLOUT)).weak());
                            });
                    }

                    ui.add_space(8.0);
                });
        });

        // Global shortcuts
        if ctx.input(|i| i.key_pressed(egui::Key::Enter)) {
            if let Some(tx) = self.tx.take() {
                let _ = tx.send(Consent::Proceed);
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            if let Some(tx) = self.tx.take() {
                let _ = tx.send(Consent::Postpone);
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if let Some(tx) = self.tx.take() {
            let _ = tx.send(Consent::Postpone);
        }
    }
}

// ---------------------------------------------------------------------------
// Progress window (single window, 3 states: Progress -> Completion)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
enum StepStatus {
    Pending,
    Running,
    Success,
    Failed,
    Skipped,
}

#[derive(Debug, Clone)]
struct StepState {
    name: String,
    status: StepStatus,
    duration: i64,
    note: String,
}

#[derive(Debug, Clone)]
struct LogEntry {
    step: String,
    line: String,
}

/// Single authoritative rendering for a step row — used by the unified
/// side panel and the legacy progress sidebar. Icons:
/// ○ pending, spinner (animated) while updating, ✓ updated, ✗ errored, – skipped.
/// Rows are clickable: clicking focuses the log on that step's first output.
fn render_step_row(ui: &mut egui::Ui, theme: Theme, step: &StepState, selected: bool) -> egui::Response {
    let id = egui::Id::new(("dotfiles_step_row", step.name.clone()));
    // Hover state from the previous frame — the row is painted before the
    // interaction pass, so live hover styling must come from the cached response.
    let hovered = ui.ctx().read_response(id).is_some_and(|r| r.hovered());
    let bg = if selected {
        ui_theme::tint(theme, ui_theme::Hue::Accent).bg
    } else if step.status == StepStatus::Running {
        ui_theme::tint(theme, ui_theme::Hue::Warning).bg
    } else if hovered {
        ui_theme::fill(theme, ui_theme::FillLevel::Secondary)
    } else {
        egui::Color32::TRANSPARENT
    };
    let inner = egui::Frame::new()
        .fill(bg)
        .corner_radius(6)
        .inner_margin(egui::Margin::symmetric(8, 5))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                match step.status {
                    StepStatus::Pending => {
                        ui.label(
                            egui::RichText::new("○")
                                .size(typography::pt(typography::FOOTNOTE))
                                .color(ui_theme::status_dot(theme, ui_theme::Dot::Pending)),
                        );
                    }
                    StepStatus::Running => {
                        // Animated spinner for "currently being updated"
                        ui.spinner();
                    }
                    StepStatus::Success => {
                        ui.label(
                            egui::RichText::new("✓")
                                .size(typography::pt(typography::FOOTNOTE))
                                .color(ui_theme::status_dot(theme, ui_theme::Dot::Success)),
                        );
                    }
                    StepStatus::Failed => {
                        ui.label(
                            egui::RichText::new("✗")
                                .size(typography::pt(typography::FOOTNOTE))
                                .color(ui_theme::status_dot(theme, ui_theme::Dot::Failed)),
                        );
                    }
                    StepStatus::Skipped => {
                        ui.label(
                            egui::RichText::new("–")
                                .size(typography::pt(typography::FOOTNOTE))
                                .color(ui_theme::status_dot(theme, ui_theme::Dot::Skipped)),
                        );
                    }
                }
                ui.label(egui::RichText::new(&step.name).size(typography::pt(typography::CALLOUT)));
                if step.duration > 0 {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            egui::RichText::new(format!("{}s", step.duration))
                                .size(typography::pt(typography::FOOTNOTE))
                                .weak()
                                .monospace(),
                        );
                    });
                }
            });
            if !step.note.is_empty() {
                ui.label(egui::RichText::new(&step.note).size(typography::pt(typography::FOOTNOTE)).weak().italics());
            }
        });
    ui.interact(inner.response.rect, id, egui::Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text("Click to show this step's output in the log")
}

/// Log view — one label per line so step sections can be anchored. `anchors`
/// maps step index → log line index of the section start (the "▶ name"
/// header). `filter` is the selected step index; when `Some`, only that
/// step's stdout/stderr is shown (global lines like "Waiting…" are hidden).
fn render_log_view(
    ui: &mut egui::Ui,
    theme: Theme,
    entries: &[LogEntry],
    steps: &[StepState],
    filter: Option<usize>,
    auto_scroll: bool,
) {
    // Build filtered list — when a section is selected, show only its
    // stdout/stderr; otherwise show the combined view.
    let filtered: Vec<&LogEntry> = if let Some(idx) = filter {
        if let Some(step) = steps.get(idx) {
            entries
                .iter()
                .filter(|e| e.step == step.name)
                .collect()
        } else {
            vec![]
        }
    } else {
        entries.iter().collect()
    };
    let stick = filter.is_none() && auto_scroll;
    egui::ScrollArea::vertical()
        .auto_shrink([false; 2])
        .stick_to_bottom(stick)
        .show(ui, |ui| {
            egui::Frame::new()
                .fill(ui.visuals().code_bg_color)
                .corner_radius(6)
                .inner_margin(8)
                .show(ui, |ui| {
                    if filtered.is_empty() {
                        if filter.is_some() {
                            ui.label(
                                egui::RichText::new("No output yet for this step — it hasn't produced stdout/stderr, or is still pending.")
                                    .size(typography::pt(typography::SUBHEADLINE))
                                    .color(ui_theme::label(theme, ui_theme::Level::Tertiary))
                                    .italics(),
                            );
                        }
                        return;
                    }
                    for entry in filtered {
                        let line = &entry.line;
                        if line.is_empty() {
                            ui.add_space(4.0);
                            continue;
                        }
                        let marker = line.starts_with('▶') || line.starts_with('✓');
                        let text = egui::RichText::new(line).monospace().size(11.0);
                        let text = if marker {
                            text.strong().color(ui_theme::label(theme, ui_theme::Level::Secondary))
                        } else {
                            text.color(ui.visuals().text_color())
                        };
                        ui.add(egui::Label::new(text).wrap());
                    }
                });
        });
}

/// Push text into the log as individual display lines (blank lines preserved).
/// Each pushed line is tagged with `step` so section filtering can select it.
fn push_log_lines(log: &mut Vec<LogEntry>, step: &str, text: &str) {
    log.extend(
        text.split('\n')
            .map(|s| s.trim_end_matches('\r').to_string())
            .map(|line| LogEntry { step: step.to_string(), line }),
    );
}

#[allow(dead_code)]
pub fn run_progress(paths: &Paths, trigger: &str) -> anyhow::Result<()> {
    let paths = paths.clone();
    let trigger = trigger.to_string();

    crate::notify::notify("dotfiles", "System update started in the background");

    let result = eframe::run_native(
        "dotfiles — upgrading",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([700.0, 400.0])
                .with_min_inner_size([700.0, 400.0])
                .with_icon(dock_icon_data())
                .with_transparent(true)
                .with_has_shadow(true),
            ..Default::default()
        },
        Box::new({
            let trigger = trigger.clone();
            move |cc| {
                let pref = load_theme_preference();
                let theme = resolve_theme(pref);
                apply_macos_appearance(&cc.egui_ctx, theme);
                // progress wants a touch tighter spacing
                let mut style = (*cc.egui_ctx.style()).clone();
                style.spacing.item_spacing = egui::vec2(8.0, 6.0);
                cc.egui_ctx.set_style(style);
                let mut app = ProgressApp::new(paths, trigger.clone());
                app.theme_preference = pref;
                app.theme = theme;
                Ok(Box::new(app))
            }
        }),
    );

    if let Err(e) = result {
        eprintln!("egui progress failed: {}", e);
        let (_tx, _rx) = std::sync::mpsc::channel::<PipelineEvent>();
        let opts = dotfiles_core::pipeline::PipelineOptions {
            trigger: trigger.clone(),
            sudo_askpass: crate::upgrade::askpass_wrapper_path(),
            event_tx: None,
        };
        let _ = dotfiles_core::pipeline::run_pipeline(&Paths::detect(), opts);
    }
    Ok(())
}

#[allow(dead_code)]
struct ProgressApp {
    paths: Paths,
    trigger: String,
    steps: Vec<StepState>,
    log_lines: Vec<LogEntry>,
    log_receiver: Receiver<PipelineEvent>,
    log_sender: Sender<PipelineEvent>,
    sudo_receiver: Receiver<SudoRequest>,
    sudo_sender: Sender<SudoRequest>,
    sudo_prompt: Option<SudoPromptState>,
    finished: bool,
    report_path: Option<PathBuf>,
    overall_status: String,
    start_time: std::time::Instant,
    pipeline_handle: Option<thread::JoinHandle<anyhow::Result<()>>>,
    askpass_handle: Option<thread::JoinHandle<()>>,
    auto_scroll: bool,
    /// step index → log line index of the step's section start ("▶ name")
    step_anchors: Vec<Option<usize>>,
    /// set on StepStarted; consumed by the first non-empty output line
    pending_anchor: Option<usize>,
    /// step index to scroll the log to (set by clicking a step row)
    log_focus: Option<usize>,
    theme_preference: ThemePreference,
    theme: Theme,
}

#[derive(Debug, Clone)]
struct SudoRequest {
    command: String,
    reason: String,
    response_tx: Sender<SudoResponse>,
}

#[derive(Debug, Clone)]
enum SudoResponse {
    Password(String),
    Cancel,
}

struct SudoPromptState {
    command: String,
    reason: String,
    password: String,
    /// Askpass-socket flow (brew casks): answer delivered to the socket.
    response_tx: Option<Sender<SudoResponse>>,
    /// Stdin-capture flow (sudo -S / interactive prompts): answer written to
    /// the child's stdin by the pipeline; empty string = cancel (EOF).
    stdin_respond: Option<std::sync::mpsc::Sender<String>>,
    show_password: bool,
}

#[allow(dead_code)]
impl ProgressApp {
    fn new(paths: Paths, trigger: String) -> Self {
        let step_names = vec![
            "brew", "rtk-repatch", "mas", "rust", "php", "node-fn", "python-uv", "opencode",
            "neovim-plugins", "gem", "tmux-tpm", "macos",
        ];
        let steps = step_names
            .into_iter()
            .map(|n| StepState {
                name: n.to_string(),
                status: StepStatus::Pending,
                duration: 0,
                note: String::new(),
            })
            .collect();

        let (log_tx, log_rx) = unbounded::<PipelineEvent>();
        let (sudo_tx, sudo_rx) = unbounded::<SudoRequest>();

        let mut app = Self {
            paths: paths.clone(),
            trigger: trigger.clone(),
            steps,
            log_lines: vec![LogEntry { step: String::new(), line: "Waiting for log…".into() }],
            log_receiver: log_rx,
            log_sender: log_tx.clone(),
            sudo_receiver: sudo_rx,
            sudo_sender: sudo_tx.clone(),
            sudo_prompt: None,
            finished: false,
            report_path: None,
            overall_status: String::new(),
            start_time: std::time::Instant::now(),
            pipeline_handle: None,
            askpass_handle: None,
            auto_scroll: true,
            step_anchors: vec![None; 12],
            pending_anchor: None,
            log_focus: None,
            theme_preference: load_theme_preference(),
            theme: resolve_theme(load_theme_preference()),
        };

        app.spawn_pipeline();
        app.spawn_askpass_server();
        app
    }

    fn spawn_pipeline(&mut self) {
        let paths = self.paths.clone();
        let trigger = self.trigger.clone();
        let tx = self.log_sender.clone();
        let handle = thread::spawn(move || -> anyhow::Result<()> {
            let askpass = crate::upgrade::askpass_wrapper_path();
            let opts = dotfiles_core::pipeline::PipelineOptions {
                trigger,
                sudo_askpass: askpass,
                event_tx: Some({
                    let (std_tx, std_rx) = std::sync::mpsc::channel::<PipelineEvent>();
                    let tx2 = tx.clone();
                    thread::spawn(move || {
                        for ev in std_rx {
                            let _ = tx2.send(ev);
                        }
                    });
                    std_tx
                }),
            };
            let (_report, _path) = dotfiles_core::pipeline::run_pipeline(&paths, opts)?;
            Ok(())
        });
        self.pipeline_handle = Some(handle);
    }

    fn spawn_askpass_server(&mut self) {
        let socket_path = crate::upgrade::askpass_socket_path();
        let sudo_tx = self.sudo_sender.clone();
        let _ = std::fs::remove_file(&socket_path);
        if let Some(parent) = socket_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let handle = thread::spawn(move || {
            use std::os::unix::net::UnixListener;
            let listener = match UnixListener::bind(&socket_path) {
                Ok(l) => l,
                Err(_) => return,
            };
            let _ = listener.set_nonblocking(false);
            for stream in listener.incoming().flatten() {
                let tx = sudo_tx.clone();
                thread::spawn(move || handle_askpass_conn(stream, tx));
                if !socket_path.exists() {
                    break;
                }
            }
        });
        self.askpass_handle = Some(handle);
    }
}

fn handle_askpass_conn(stream: std::os::unix::net::UnixStream, sudo_tx: Sender<SudoRequest>) {
    use std::io::{BufRead, BufReader, Write};
    let mut reader = BufReader::new(&stream);
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() {
        return;
    }
    let v: serde_json::Value = match serde_json::from_str(&line) {
        Ok(v) => v,
        Err(_) => return,
    };
    let cmd = v.get("command").and_then(|x| x.as_str()).unwrap_or("sudo operation").to_string();
    let reason = v.get("reason").and_then(|x| x.as_str()).unwrap_or("").to_string();

    let (resp_tx, resp_rx) = unbounded::<SudoResponse>();
    let req = SudoRequest { command: cmd, reason, response_tx: resp_tx };
    let _ = sudo_tx.send(req);

    let resp = resp_rx.recv().unwrap_or(SudoResponse::Cancel);
    let mut writer = &stream;
    match resp {
        SudoResponse::Password(pw) => {
            let resp_json = serde_json::json!({"password": pw}).to_string() + "\n";
            let _ = writer.write_all(resp_json.as_bytes());
        }
        SudoResponse::Cancel => {
            let resp_json = serde_json::json!({"cancel": true}).to_string() + "\n";
            let _ = writer.write_all(resp_json.as_bytes());
        }
    }
    let _ = writer.flush();
}

impl eframe::App for ProgressApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        while let Ok(ev) = self.log_receiver.try_recv() {
            match ev {
                PipelineEvent::StepStarted { name, .. } => {
                    if let Some(pos) = self.steps.iter().position(|s| s.name == name) {
                        self.steps[pos].status = StepStatus::Running;
                        self.step_anchors[pos] = None;
                        self.pending_anchor = Some(pos);
                    }
                }
                PipelineEvent::LogLine { step, stream, line } => {
                    if stream != LogStream::Combined {
                        continue;
                    }
                    let clean = strip_ansi(&line);
                    // First output line of a just-started step anchors its section
                    if let Some(pos) = self.pending_anchor {
                        if !clean.trim().is_empty() && self.step_anchors.get(pos) == Some(&None) {
                            self.step_anchors[pos] = Some(self.log_lines.len());
                        }
                        self.pending_anchor = None;
                    }
                    self.log_lines.push(LogEntry { step, line: clean });
                    if self.log_lines.len() > 5000 {
                        self.log_lines.drain(0..1000);
                        for a in self.step_anchors.iter_mut() {
                            *a = a.map(|i| i.saturating_sub(1000));
                        }
                    }
                }
                PipelineEvent::StepFinished { report } => {
                    if let Some(s) = self.steps.iter_mut().find(|s| s.name == report.name) {
                        s.status = match report.status.as_str() {
                            "success" => StepStatus::Success,
                            "failed" => StepStatus::Failed,
                            "skipped" => StepStatus::Skipped,
                            _ => StepStatus::Pending,
                        };
                        s.duration = report.duration_seconds;
                        s.note = report.note.clone();
                    }
                }
                PipelineEvent::RunFinished { status, report_path } => {
                    self.finished = true;
                    self.overall_status = status;
                    self.report_path = Some(report_path);
                    push_log_lines(&mut self.log_lines, "", &format!("\n\n✓ Update finished — see {}", self.report_path.as_ref().unwrap().display()));
                    crate::notify::notify("dotfiles", &format!("Update finished ({})", self.overall_status));
                    let _ = std::fs::remove_file(crate::upgrade::askpass_socket_path());
                }
                PipelineEvent::SudoPrompt { command, reason, respond } => {
                    // Stdin-capture flow: the pipeline detected a password prompt
                    self.sudo_prompt = Some(SudoPromptState {
                        command,
                        reason,
                        password: String::new(),
                        response_tx: None,
                        stdin_respond: Some(respond),
                        show_password: false,
                    });
                }
            }
        }

        while let Ok(req) = self.sudo_receiver.try_recv() {
            self.sudo_prompt = Some(SudoPromptState {
                command: req.command,
                reason: req.reason,
                password: String::new(),
                response_tx: Some(req.response_tx),
                stdin_respond: None,
                show_password: false,
            });
        }

        egui::TopBottomPanel::top("top").show(ctx, |ui| {
            ui.horizontal(|ui| {
                let elapsed = self.start_time.elapsed().as_secs();
                ui.label(egui::RichText::new(format!("{}m {:02}s", elapsed / 60, elapsed % 60)).size(typography::pt(typography::CALLOUT)).monospace().weak());
                ui.separator();
                if self.finished {
                    let ok_tint = ui_theme::tint(self.theme, ui_theme::Hue::Success);
                    egui::Frame::new()
                        .fill(ok_tint.bg)
                        .stroke(egui::Stroke::new(1.0_f32, ok_tint.fg.gamma_multiply(0.35)))
                        .corner_radius(20)
                        .inner_margin(egui::Margin::symmetric(10, 4))
                        .show(ui, |ui| {
                            ui.label(
                                egui::RichText::new(format!("Finished — {}", self.overall_status))
                                    .size(typography::pt(typography::CALLOUT))
                                    .strong()
                                    .color(ok_tint.fg),
                            );
                        });
                } else {
                    ui.spinner();
                    ui.label(egui::RichText::new("Running").size(typography::pt(typography::CALLOUT)).weak());
                    let pct = self.steps.iter().filter(|s| s.status != StepStatus::Pending && s.status != StepStatus::Running).count() as f32
                        / self.steps.len() as f32;
                    egui::ProgressBar::new(pct)
                        .desired_width(120.0)
                        .show_percentage()
                        .ui(ui);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // Theme toggle — System → Light → Dark, persisted
                    let t_icon = theme_icon(self.theme_preference);
                    if ui
                        .add(egui::Button::new(egui::RichText::new(t_icon).size(14.0)).frame(false))
                        .on_hover_text(format!("Theme: {:?} — click to cycle (System follows macOS)", self.theme_preference))
                        .clicked()
                    {
                        let next = next_theme_preference(self.theme_preference);
                        self.theme_preference = next;
                        self.theme = resolve_theme(next);
                        save_theme_preference(next);
                        apply_macos_appearance(ctx, self.theme);
                        let mut style = (*ctx.style()).clone();
                        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
                        ctx.set_style(style);
                        ctx.request_repaint();
                    }
                    ui.separator();
                    if !self.finished {
                        if ui.button("Cancel").clicked() {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                    } else if let Some(path) = &self.report_path {
                        if ui.button("Open report").clicked() {
                            let _ = std::process::Command::new("open").arg(path).spawn();
                        }
                        let ps = ui_theme::primary_button(self.theme);
                        let done_btn = egui::Button::new(egui::RichText::new("Done").strong().color(ps.text))
                            .fill(ps.fill)
                            .stroke(ps.stroke)
                            .corner_radius(8);
                        if ui.add(done_btn).clicked() {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                    }
                });
            });
        });

        egui::SidePanel::left("steps")
            .exact_width(200.0)
            .resizable(false)
            .show(ctx, |ui| {
                ui.add_space(6.0);
                ui.label(egui::RichText::new("STEPS").size(typography::pt(typography::FOOTNOTE)).weak().strong().extra_letter_spacing(1.0));
                ui.add_space(4.0);
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for idx in 0..self.steps.len() {
                        let selected = self.log_focus == Some(idx);
                        let resp = render_step_row(ui, self.theme, &self.steps[idx], selected);
                        if resp.clicked() {
                            // Toggle filter: clicking the active section shows all
                            if selected {
                                self.log_focus = None;
                                self.auto_scroll = true;
                            } else {
                                self.auto_scroll = false;
                                self.log_focus = Some(idx);
                            }
                        }
                    }
                });
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            let title = if let Some(idx) = self.log_focus {
                if let Some(s) = self.steps.get(idx) {
                    format!("Live log — {}", s.name)
                } else {
                    "Live log".to_string()
                }
            } else {
                "Live log".to_string()
            };
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(title).size(typography::pt(typography::SUBHEADLINE)).strong().weak());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if self.log_focus.is_some() && ui.small_button("Show all").clicked() {
                        self.log_focus = None;
                        self.auto_scroll = true;
                    }
                    if ui.small_button("Copy").clicked() {
                        let text = if let Some(idx) = self.log_focus {
                            if let Some(s) = self.steps.get(idx) {
                                self.log_lines.iter().filter(|e| e.step == s.name).map(|e| e.line.as_str()).collect::<Vec<_>>().join("\n")
                            } else {
                                self.log_lines.iter().map(|e| e.line.as_str()).collect::<Vec<_>>().join("\n")
                            }
                        } else {
                            self.log_lines.iter().map(|e| e.line.as_str()).collect::<Vec<_>>().join("\n")
                        };
                        ui.ctx().copy_text(text);
                    }
                    if ui.small_button("Clear").clicked() {
                        self.log_lines.clear();
                        self.step_anchors.iter_mut().for_each(|a| *a = None);
                        self.log_focus = None;
                    }
                    ui.checkbox(&mut self.auto_scroll, "Auto-scroll");
                });
            });
            ui.separator();
            render_log_view(ui, self.theme, &self.log_lines, &self.steps, self.log_focus, self.auto_scroll);
        });

        // Sudo modal — polished card
        let mut sudo_close = false;
        let mut sudo_response: Option<SudoResponse> = None;
        if let Some(prompt) = &mut self.sudo_prompt {
            egui::Window::new("Sudo required")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .frame(
                    egui::Frame::window(&ctx.style())
                        .corner_radius(12)
                        .inner_margin(16),
                )
                .show(ctx, |ui| {
                    ui.vertical_centered(|ui| {
                        let danger_tint = ui_theme::tint(self.theme, ui_theme::Hue::Danger);
                        egui::Frame::new()
                            .fill(danger_tint.bg)
                            .corner_radius(20)
                            .inner_margin(8)
                            .show(ui, |ui| {
                                ui.label(egui::RichText::new("!").size(14.0).strong().color(danger_tint.fg));
                            });
                        ui.add_space(6.0);
                        ui.label(egui::RichText::new("Administrator password required").size(typography::pt(typography::BODY)).strong());
                    });
                    ui.add_space(8.0);
                    egui::Frame::new()
                        .fill(ui_theme::fill(self.theme, ui_theme::FillLevel::Secondary))
                        .corner_radius(8)
                        .inner_margin(10)
                        .stroke(egui::Stroke::new(1.0_f32, ui_theme::separator(self.theme)))
                        .show(ui, |ui| {
                            ui.label(egui::RichText::new(&prompt.command).monospace().size(typography::pt(typography::SUBHEADLINE)));
                            ui.add_space(4.0);
                            ui.label(egui::RichText::new(&prompt.reason).size(typography::pt(typography::SUBHEADLINE)).weak());
                        });
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("Password").size(typography::pt(typography::SUBHEADLINE)).weak());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.checkbox(&mut prompt.show_password, "Show");
                        });
                    });
                    let resp = egui::TextEdit::singleline(&mut prompt.password)
                        .password(!prompt.show_password)
                        .hint_text("Enter password")
                        .desired_width(f32::INFINITY)
                        .show(ui)
                        .response;
                    if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        sudo_response = Some(SudoResponse::Password(prompt.password.clone()));
                        sudo_close = true;
                    }
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if kit_button(ui, "Unlock", ui_theme::primary_button(self.theme), Some([96.0, 28.0])).clicked() {
                                sudo_response = Some(SudoResponse::Password(prompt.password.clone()));
                                sudo_close = true;
                            }
                            if ui.button("Cancel").clicked() {
                                sudo_response = Some(SudoResponse::Cancel);
                                sudo_close = true;
                            }
                        });
                    });
                    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                        sudo_response = Some(SudoResponse::Cancel);
                        sudo_close = true;
                    }
                });
        }
        if sudo_close {
            if let Some(prompt) = self.sudo_prompt.take() {
                let resp = sudo_response.unwrap_or(SudoResponse::Cancel);
                if let Some(tx) = prompt.response_tx {
                    let _ = tx.send(resp.clone());
                }
                if let Some(tx) = prompt.stdin_respond {
                    // Empty answer = cancel → the pipeline closes the child's stdin (EOF)
                    let pw = match resp {
                        SudoResponse::Password(p) if !p.is_empty() => p,
                        _ => String::new(),
                    };
                    let _ = tx.send(pw);
                }
            }
        }

        if !self.finished {
            ctx.request_repaint_after(std::time::Duration::from_millis(200));
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        let _ = std::fs::remove_file(crate::upgrade::askpass_socket_path());
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Unified single-window upgrade — consent → progress → done in one run_native
// Fixes macOS “Choose Application” + hang caused by two sequential
// eframe::run_native (winit EventLoop can only run once per process).
// ─────────────────────────────────────────────────────────────────────────────
fn has_gui_available() -> bool {
    #[cfg(target_os = "macos")]
    return true;
    #[cfg(not(target_os = "macos"))]
    return std::env::var("DISPLAY").is_ok() || std::env::var("WAYLAND_DISPLAY").is_ok();
}

pub fn run_upgrade_window(paths: &Paths, trigger: &str) -> anyhow::Result<()> {
    // Stamp last_dialog_at before showing — covers dismiss/locked-screen (same as old solicit_consent_gui)
    {
        let mut s = dotfiles_core::state::State::load(&paths.state_file).unwrap_or_default();
        s.last_dialog_at = Some(chrono::Utc::now().timestamp());
        let _ = s.save(&paths.state_file);
    }

    if !has_gui_available() {
        eprint!("Proceed? [Y/n] ");
        use std::io::{self, Write};
        let _ = io::stdout().flush();
        let mut line = String::new();
        let _ = io::stdin().read_line(&mut line);
        if line.trim().to_lowercase() == "n" {
            let mut s = dotfiles_core::state::State::load(&paths.state_file).unwrap_or_default();
            s.last_outcome = Some("postponed".into());
            s.last_dialog_at = Some(chrono::Utc::now().timestamp());
            let _ = s.save(&paths.state_file);
            eprintln!("postponed");
            return Ok(());
        }
        // Headless fallback — run pipeline directly
        let askpass = crate::upgrade::askpass_wrapper_path();
        let opts = dotfiles_core::pipeline::PipelineOptions {
            trigger: trigger.to_string(),
            sudo_askpass: askpass,
            event_tx: None,
        };
        let (_report, _path) = dotfiles_core::pipeline::run_pipeline(paths, opts)?;
        return Ok(());
    }

    let sections = dotfiles_core::probes::probe_all();
    let mut summary = dotfiles_core::probes::summary_text(&sections);
    if summary.trim().is_empty() {
        summary = "Nothing looks outdated; this will refresh indexes and caches.".into();
    }
    let gate_status = compute_gate_status();
    let pref = load_theme_preference();
    let theme = resolve_theme(pref);

    let result = eframe::run_native(
        "dotfiles — update available",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([560.0, 480.0])
                .with_min_inner_size([560.0, 480.0]),
            ..Default::default()
        },
        Box::new(move |cc| {
            apply_macos_appearance(&cc.egui_ctx, theme);
            Ok(Box::new(UnifiedApp::new(
                paths.clone(),
                trigger.to_string(),
                summary,
                sections,
                gate_status,
                pref,
                theme,
            )))
        }),
    );
    if let Err(e) = result {
        eprintln!("egui unified failed: {e} — falling back to headless");
        let askpass = crate::upgrade::askpass_wrapper_path();
        let opts = dotfiles_core::pipeline::PipelineOptions {
            trigger: trigger.to_string(),
            sudo_askpass: askpass,
            event_tx: None,
        };
        let _ = dotfiles_core::pipeline::run_pipeline(paths, opts);
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UnifiedMode {
    Consent,
    Progress,
    Done,
}

struct UnifiedApp {
    paths: Paths,
    trigger: String,
    summary: String,
    sections: Vec<Section>,
    gate_status: Vec<gates::GateResult>,
    mode: UnifiedMode,
    theme_preference: ThemePreference,
    theme: Theme,
    // Progress state (only used in Progress/Done)
    steps: Vec<StepState>,
    log_lines: Vec<LogEntry>,
    log_receiver: Option<Receiver<PipelineEvent>>,
    log_sender: Option<Sender<PipelineEvent>>,
    sudo_receiver: Option<Receiver<SudoRequest>>,
    sudo_sender: Option<Sender<SudoRequest>>,
    sudo_prompt: Option<SudoPromptState>,
    finished: bool,
    report_path: Option<PathBuf>,
    overall_status: String,
    start_time: Option<std::time::Instant>,
    pipeline_handle: Option<thread::JoinHandle<anyhow::Result<()>>>,
    askpass_handle: Option<thread::JoinHandle<()>>,
    auto_scroll: bool,
    /// step index → log line index of the step's section start ("▶ name")
    step_anchors: Vec<Option<usize>>,
    /// set on StepStarted; consumed by the first non-empty output line
    pending_anchor: Option<usize>,
    /// step index to scroll the log to (set by clicking a step row)
    log_focus: Option<usize>,
    current_step_index: Option<usize>,
    total_steps: usize,
}

impl UnifiedApp {
    fn new(
        paths: Paths,
        trigger: String,
        summary: String,
        sections: Vec<Section>,
        gate_status: Vec<gates::GateResult>,
        theme_preference: ThemePreference,
        theme: Theme,
    ) -> Self {
        let steps: Vec<StepState> = vec![
            "brew", "rtk-repatch", "mas", "rust", "php", "node-fn", "python-uv", "opencode",
            "neovim-plugins", "gem", "tmux-tpm", "macos",
        ]
        .into_iter()
        .map(|n| StepState {
            name: n.to_string(),
            status: StepStatus::Pending,
            duration: 0,
            note: String::new(),
        })
        .collect();
        let total_steps = steps.len();
        Self {
            paths,
            trigger,
            summary,
            sections,
            gate_status,
            mode: UnifiedMode::Consent,
            theme_preference,
            theme,
            steps,
            log_lines: vec![LogEntry { step: String::new(), line: "Waiting for log…".into() }],
            log_receiver: None,
            log_sender: None,
            sudo_receiver: None,
            sudo_sender: None,
            sudo_prompt: None,
            finished: false,
            report_path: None,
            overall_status: String::new(),
            start_time: None,
            pipeline_handle: None,
            askpass_handle: None,
            auto_scroll: true,
            step_anchors: vec![None; total_steps],
            pending_anchor: None,
            log_focus: None,
            current_step_index: None,
            total_steps,
        }
    }

    fn start_progress(&mut self, ctx: &egui::Context) {
        self.mode = UnifiedMode::Progress;
        self.finished = false;
        self.start_time = Some(std::time::Instant::now());
        self.log_lines = vec![LogEntry { step: String::new(), line: "Waiting for log…".into() }];
        // Reset progress tracking for the new run
        self.current_step_index = None;
        self.total_steps = self.steps.len();
        self.step_anchors = vec![None; self.steps.len()];
        self.pending_anchor = None;
        self.log_focus = None;
        for s in &mut self.steps {
            s.status = StepStatus::Pending;
            s.duration = 0;
            s.note.clear();
        }
        let (log_tx, log_rx) = unbounded::<PipelineEvent>();
        let (sudo_tx, sudo_rx) = unbounded::<SudoRequest>();
        self.log_receiver = Some(log_rx);
        self.log_sender = Some(log_tx.clone());
        self.sudo_receiver = Some(sudo_rx);
        self.sudo_sender = Some(sudo_tx.clone());
        // Notify
        crate::notify::notify("dotfiles", "System update started in the background");
        // Spawn askpass server
        let socket_path = crate::upgrade::askpass_socket_path();
        let _ = std::fs::remove_file(&socket_path);
        if let Some(parent) = socket_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let sudo_tx_clone = sudo_tx.clone();
        let ask_handle = thread::spawn(move || {
            use std::os::unix::net::UnixListener;
            let listener = match UnixListener::bind(&socket_path) {
                Ok(l) => l,
                Err(_) => return,
            };
            let _ = listener.set_nonblocking(false);
            for stream in listener.incoming().flatten() {
                let tx = sudo_tx_clone.clone();
                thread::spawn(move || handle_askpass_conn(stream, tx));
                if !socket_path.exists() {
                    break;
                }
            }
        });
        self.askpass_handle = Some(ask_handle);
        // Spawn pipeline
        let paths = self.paths.clone();
        let trigger = self.trigger.clone();
        let log_tx_clone = log_tx.clone();
        let handle = thread::spawn(move || -> anyhow::Result<()> {
            let askpass = crate::upgrade::askpass_wrapper_path();
            let opts = dotfiles_core::pipeline::PipelineOptions {
                trigger,
                sudo_askpass: askpass,
                event_tx: Some({
                    let (std_tx, std_rx) = std::sync::mpsc::channel::<PipelineEvent>();
                    let tx2 = log_tx_clone.clone();
                    thread::spawn(move || {
                        for ev in std_rx {
                            let _ = tx2.send(ev);
                        }
                    });
                    std_tx
                }),
            };
            let (_report, _path) = dotfiles_core::pipeline::run_pipeline(&paths, opts)?;
            Ok(())
        });
        self.pipeline_handle = Some(handle);
        ctx.send_viewport_cmd(egui::ViewportCommand::Title("dotfiles — upgrading".into()));
        ctx.request_repaint();
    }

    fn consent_ui(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        // Reuse the polished consent UI, but without closing the window on Proceed
        let total_updates: usize = self.sections.iter().map(|s| s.count).sum();
        let sources_with_updates = self.sections.iter().filter(|s| s.count > 0).count();
        let has_gate_fail = self
            .gate_status
            .iter()
            .any(|g| !g.ok && g.name != "schedule" && g.name != "dialog_cooldown");
        let all_gates_ok = !has_gate_fail;

        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let accent_tint = ui_theme::tint(self.theme, ui_theme::Hue::Accent);
            egui::Frame::new()
                .fill(accent_tint.bg)
                .stroke(egui::Stroke::new(1.0_f32, ui_theme::separator(self.theme)))
                .corner_radius(10)
                .inner_margin(6)
                .show(ui, |ui| {
                    ui.label(egui::RichText::new("↻").size(22.0).color(accent_tint.fg));
                });
            ui.add_space(8.0);
            ui.vertical(|ui| {
                ui.label(egui::RichText::new("Updates ready").size(typography::pt(typography::TITLE3)).strong().line_height(Some(20.0)));
                ui.label(egui::RichText::new("Review what will change — you choose when to run it").size(typography::pt(typography::SUBHEADLINE) * 1.25).weak());
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let success_tint = ui_theme::tint(self.theme, ui_theme::Hue::Success);
                let (badge_fill, badge_fg, badge_text) = if total_updates > 0 {
                    (success_tint.bg, success_tint.fg, format!("{} updates", total_updates))
                } else {
                    let neutral = ui_theme::tint(self.theme, ui_theme::Hue::Neutral);
                    (neutral.bg, neutral.fg, "Up to date".to_string())
                };
                egui::Frame::new()
                    .fill(badge_fill)
                    .stroke(egui::Stroke::new(1.0_f32, ui_theme::separator(self.theme)))
                    .corner_radius(20)
                    .inner_margin(egui::Margin::symmetric(12, 6))
                    .show(ui, |ui| {
                        ui.label(egui::RichText::new(badge_text).size(typography::pt(typography::CALLOUT) * 1.05).strong().color(badge_fg));
                        if sources_with_updates > 0 {
                            ui.label(egui::RichText::new(format!("{} sources", sources_with_updates)).size(typography::pt(typography::SUBHEADLINE) * 1.05).color(badge_fg));
                        }
                    });
                ui.add_space(6.0);
                let icon = theme_icon(self.theme_preference);
                if ui.add(egui::Button::new(egui::RichText::new(icon).size(16.0)).frame(false)).on_hover_text(format!("Theme: {:?} — click to cycle (System follows macOS)", self.theme_preference)).clicked() {
                    let next = next_theme_preference(self.theme_preference);
                    self.theme_preference = next;
                    self.theme = resolve_theme(next);
                    save_theme_preference(next);
                    apply_macos_appearance(ctx, self.theme);
                    ctx.request_repaint();
                }
            });
        });
        ui.add_space(14.0);
        if all_gates_ok {
            let ok_tint = ui_theme::tint(self.theme, ui_theme::Hue::Success);
            egui::Frame::new()
                .fill(ok_tint.bg)
                .stroke(egui::Stroke::new(1.0_f32, ok_tint.fg.gamma_multiply(0.35)))
                .corner_radius(8)
                .inner_margin(egui::Margin::symmetric(10, 8))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("●").size(9.0).color(ok_tint.fg));
                        ui.label(egui::RichText::new("All system checks passed").size(typography::pt(typography::CALLOUT) * 1.25).strong().color(ok_tint.fg));
                        ui.separator();
                        let details: Vec<String> = self.gate_status.iter().filter(|g| g.name == "power" || g.name == "disk").map(|g| g.reason.clone()).collect();
                        ui.label(egui::RichText::new(details.join("  ·  ")).size(typography::pt(typography::SUBHEADLINE) * 1.15).weak());
                    });
                });
        } else {
            let warn_tint = ui_theme::tint(self.theme, ui_theme::Hue::Warning);
            egui::Frame::new()
                .fill(warn_tint.bg)
                .stroke(egui::Stroke::new(1.0_f32, warn_tint.fg.gamma_multiply(0.35)))
                .corner_radius(8)
                .inner_margin(10)
                .show(ui, |ui| {
                    ui.vertical(|ui| {
                        ui.label(egui::RichText::new("Pre-flight checks").size(typography::pt(typography::SUBHEADLINE)).strong().color(warn_tint.fg));
                        ui.add_space(2.0);
                        // Dots sit on the warning tint — use tint-fg variants for AA
                        let ok_dot = ui_theme::tint(self.theme, ui_theme::Hue::Success).fg;
                        let fail_dot = ui_theme::tint(self.theme, ui_theme::Hue::Danger).fg;
                        for g in &self.gate_status {
                            let dot_col = if g.ok { ok_dot } else { fail_dot };
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new("●").size(8.0).color(dot_col));
                                ui.label(egui::RichText::new(format!("{} — {}", g.name, g.reason)).size(typography::pt(typography::SUBHEADLINE)).color(if g.ok { ui.visuals().weak_text_color() } else { ui_theme::tint(self.theme, ui_theme::Hue::Danger).fg }));
                            });
                        }
                    });
                });
        }
        ui.add_space(12.0);
        ui.separator();
        ui.add_space(8.0);
        for sec in &self.sections {
            let has_items = !sec.items.is_empty();
            let is_populated = sec.count > 0;
            // Kit card: Fill Tertiary surface + hairline separator, no accent outline
            let (card_fill, border_col) = ui_theme::card(self.theme);
            egui::Frame::new()
                .fill(card_fill)
                .stroke(egui::Stroke::new(1.0_f32, border_col))
                .corner_radius(10)
                .inner_margin(egui::Margin { left: 12, right: 12, top: 10, bottom: 10 })
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(&sec.title).size(typography::pt(typography::BODY) * 1.05).strong().color(if is_populated { ui.visuals().text_color() } else { ui.visuals().weak_text_color() }));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let (fill, fg) = if is_populated { (ui_theme::system(self.theme, ui_theme::Hue::Accent), egui::Color32::WHITE) } else { (ui_theme::fill(self.theme, ui_theme::FillLevel::Quaternary), ui.visuals().weak_text_color()) };
                            egui::Frame::new().fill(fill).corner_radius(10).inner_margin(egui::Margin::symmetric(8, 3)).show(ui, |ui| {
                                ui.label(egui::RichText::new(format!("{}", sec.count)).size(typography::pt(typography::SUBHEADLINE)).strong().color(fg));
                            });
                        });
                    });
                    if has_items {
                        ui.add_space(8.0);
                        ui.separator();
                        ui.add_space(6.0);
                        for item in &sec.items {
                            let (name_part, ver_part) = if let Some(idx) = item.find(" (") { (&item[..idx], Some(&item[idx..])) } else { (item.as_str(), None) };
                            egui::Frame::new().fill(ui.visuals().faint_bg_color.gamma_multiply(0.0)).inner_margin(egui::Margin::symmetric(2, 2)).show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new("·").size(typography::pt(typography::CALLOUT)).color(ui.visuals().weak_text_color()));
                                    ui.label(egui::RichText::new(name_part).size(typography::pt(typography::CALLOUT) * 1.05).color(ui.visuals().text_color()));
                                    if let Some(v) = ver_part {
                                        ui.label(egui::RichText::new(v).size(typography::pt(typography::SUBHEADLINE) * 1.05).color(ui.visuals().weak_text_color()).monospace());
                                    }
                                });
                            });
                        }
                    } else {
                        ui.add_space(4.0);
                        ui.label(egui::RichText::new(if sec.count == 0 { "No changes" } else { "" }).size(typography::pt(typography::SUBHEADLINE)).color(ui_theme::label(self.theme, ui_theme::Level::Tertiary)).italics());
                    }
                });
            ui.add_space(8.0);
        }
        if self.sections.is_empty() {
            egui::Frame::new().fill(ui_theme::fill(self.theme, ui_theme::FillLevel::Tertiary)).corner_radius(8).inner_margin(12).show(ui, |ui| {
                ui.label(egui::RichText::new(&self.summary).size(typography::pt(typography::CALLOUT)).weak());
            });
        }
    }

    fn progress_ui(&mut self, _ctx: &egui::Context, ui: &mut egui::Ui) {
        // Steps are shown in the left SidePanel; this central panel now
        // holds only the log controls and the live log — no duplication.
        let title = if let Some(idx) = self.log_focus {
            if let Some(s) = self.steps.get(idx) {
                format!("Live log — {}", s.name)
            } else {
                "Live log".to_string()
            }
        } else {
            "Live log".to_string()
        };
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(title).size(typography::pt(typography::SUBHEADLINE)).strong().weak());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if self.log_focus.is_some() && ui.small_button("Show all").clicked() {
                    self.log_focus = None;
                    self.auto_scroll = true;
                }
                if ui.small_button("Copy").clicked() {
                    let text = if let Some(idx) = self.log_focus {
                        if let Some(s) = self.steps.get(idx) {
                            self.log_lines.iter().filter(|e| e.step == s.name).map(|e| e.line.as_str()).collect::<Vec<_>>().join("\n")
                        } else {
                            self.log_lines.iter().map(|e| e.line.as_str()).collect::<Vec<_>>().join("\n")
                        }
                    } else {
                        self.log_lines.iter().map(|e| e.line.as_str()).collect::<Vec<_>>().join("\n")
                    };
                    ui.ctx().copy_text(text);
                }
                if ui.small_button("Clear").clicked() {
                    self.log_lines.clear();
                    self.step_anchors.iter_mut().for_each(|a| *a = None);
                    self.log_focus = None;
                }
                ui.checkbox(&mut self.auto_scroll, "Auto-scroll");
            });
        });
        ui.separator();
        render_log_view(ui, self.theme, &self.log_lines, &self.steps, self.log_focus, self.auto_scroll);
    }
}

impl eframe::App for UnifiedApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Poll pipeline events if in progress
        if self.mode == UnifiedMode::Progress {
            let mut finished = false;
            let mut report_path: Option<PathBuf> = None;
            let mut overall_status = String::new();
            if let Some(rx) = &self.log_receiver {
                while let Ok(ev) = rx.try_recv() {
                    match ev {
                        PipelineEvent::StepStarted { name, index, total } => {
                            self.current_step_index = Some(index);
                            self.total_steps = total;
                            // Use 1-based index for robust marking (fixes composer/php name mismatch)
                            let pos = if self.steps.get(index.saturating_sub(1)).is_some() {
                                index.saturating_sub(1)
                            } else {
                                self.steps.iter().position(|s| s.name == name).unwrap_or(usize::MAX)
                            };
                            if pos < self.steps.len() {
                                self.steps[pos].status = StepStatus::Running;
                                self.step_anchors[pos] = None;
                                self.pending_anchor = Some(pos);
                            }
                        }
                        PipelineEvent::LogLine { step, stream, line } => {
                            if stream != LogStream::Combined {
                                continue;
                            }
                            let clean = strip_ansi(&line);
                            // First output line of a just-started step anchors its section
                            if let Some(pos) = self.pending_anchor {
                                if !clean.trim().is_empty() && self.step_anchors.get(pos) == Some(&None) {
                                    self.step_anchors[pos] = Some(self.log_lines.len());
                                }
                                self.pending_anchor = None;
                            }
                            self.log_lines.push(LogEntry { step, line: clean });
                            if self.log_lines.len() > 5000 {
                                self.log_lines.drain(0..1000);
                                for a in self.step_anchors.iter_mut() {
                                    *a = a.map(|i| i.saturating_sub(1000));
                                }
                            }
                        }
                        PipelineEvent::StepFinished { report } => {
                            if let Some(s) = self.steps.iter_mut().find(|s| s.name == report.name) {
                                s.status = match report.status.as_str() {
                                    "success" => StepStatus::Success,
                                    "failed" => StepStatus::Failed,
                                    "skipped" => StepStatus::Skipped,
                                    _ => StepStatus::Pending,
                                };
                                s.duration = report.duration_seconds;
                                s.note = report.note.clone();
                            }
                        }
                        PipelineEvent::RunFinished { status, report_path: rp } => {
                            finished = true;
                            report_path = Some(rp.clone());
                            overall_status = status.clone();
                            push_log_lines(&mut self.log_lines, "", &format!("\n\n✓ Update finished — see {}", rp.display()));
                            crate::notify::notify("dotfiles", &format!("Update finished ({})", status));
                            let _ = std::fs::remove_file(crate::upgrade::askpass_socket_path());
                        }
                        PipelineEvent::SudoPrompt { command, reason, respond } => {
                            // Stdin-capture flow: the pipeline detected a password prompt
                            self.sudo_prompt = Some(SudoPromptState {
                                command,
                                reason,
                                password: String::new(),
                                response_tx: None,
                                stdin_respond: Some(respond),
                                show_password: false,
                            });
                        }
                    }
                }
            }
            if finished {
                self.finished = true;
                self.report_path = report_path;
                self.overall_status = overall_status;
                self.mode = UnifiedMode::Done;
                ctx.send_viewport_cmd(egui::ViewportCommand::Title("dotfiles — done".into()));
            }
            // Sudo prompts
            if let Some(sudo_rx) = &self.sudo_receiver {
                while let Ok(req) = sudo_rx.try_recv() {
                    // Askpass-socket flow (brew casks): answer delivered to the socket
                    self.sudo_prompt = Some(SudoPromptState {
                        command: req.command,
                        reason: req.reason,
                        password: String::new(),
                        response_tx: Some(req.response_tx),
                        stdin_respond: None,
                        show_password: false,
                    });
                }
            }
        }

        // ── Top bar for progress/done ──
        if self.mode != UnifiedMode::Consent {
            egui::TopBottomPanel::top("unified_top").show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if let Some(start) = self.start_time {
                        let elapsed = start.elapsed().as_secs();
                        ui.label(egui::RichText::new(format!("{}m {:02}s", elapsed/60, elapsed%60)).size(typography::pt(typography::CALLOUT)).monospace().weak());
                        ui.separator();
                    }
                    if self.finished {
                        let ok_tint = ui_theme::tint(self.theme, ui_theme::Hue::Success);
                        egui::Frame::new().fill(ok_tint.bg).stroke(egui::Stroke::new(1.0_f32, ok_tint.fg.gamma_multiply(0.35))).corner_radius(20).inner_margin(egui::Margin::symmetric(10, 4)).show(ui, |ui| {
                            ui.label(egui::RichText::new(format!("Finished — {}", self.overall_status)).size(typography::pt(typography::CALLOUT)).strong().color(ok_tint.fg));
                        });
                    } else {
                        ui.spinner();
                        ui.label(egui::RichText::new("Running").size(typography::pt(typography::CALLOUT)).weak());
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let icon = theme_icon(self.theme_preference);
                        if ui.add(egui::Button::new(egui::RichText::new(icon).size(14.0)).frame(false)).on_hover_text(format!("Theme: {:?} (click to cycle)", self.theme_preference)).clicked() {
                            let next = next_theme_preference(self.theme_preference);
                            self.theme_preference = next;
                            self.theme = resolve_theme(next);
                            save_theme_preference(next);
                            apply_macos_appearance(ctx, self.theme);
                        }
                        ui.separator();
                        if !self.finished {
                            if ui.button("Cancel").clicked() {
                                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                            }
                        } else if let Some(path) = &self.report_path {
                            if ui.button("Open report").clicked() {
                                let _ = std::process::Command::new("open").arg(path).spawn();
                            }
                            let ps = ui_theme::primary_button(self.theme);
                            let done_btn = egui::Button::new(egui::RichText::new("Done").strong().color(ps.text)).fill(ps.fill).stroke(ps.stroke).corner_radius(8);
                            if ui.add(done_btn).clicked() { ctx.send_viewport_cmd(egui::ViewportCommand::Close); }
                        }
                    });
                });
            });
            egui::SidePanel::left("unified_steps").exact_width(200.0).resizable(false).show(ctx, |ui| {
                ui.add_space(6.0);
                ui.label(egui::RichText::new("STEPS").size(typography::pt(typography::FOOTNOTE)).weak().strong().extra_letter_spacing(1.0));
                ui.add_space(4.0);
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for idx in 0..self.steps.len() {
                        let selected = self.log_focus == Some(idx);
                        let resp = render_step_row(ui, self.theme, &self.steps[idx], selected);
                        if resp.clicked() {
                            if selected {
                                self.log_focus = None;
                                self.auto_scroll = true;
                            } else {
                                self.auto_scroll = false;
                                self.log_focus = Some(idx);
                            }
                        }
                    }
                });
            });
            // ── Footer status bar: centered progress bar with in-bar label ──
            // Keeps the progress bar in the middle of the footer, with text like
            // "upgrading 3 out of 12 apps" inside the bar (current index / total).
            egui::TopBottomPanel::bottom("unified_footer")
                .frame(
                    egui::Frame::new()
                        .fill(ctx.style().visuals.panel_fill)
                        .inner_margin(egui::Margin::symmetric(16, 10))
                        .stroke(egui::Stroke::new(1.0_f32, ctx.style().visuals.widgets.noninteractive.bg_stroke.color)),
                )
                .show(ctx, |ui| {
                    let total = self.total_steps.max(1);
                    let completed = self
                        .steps
                        .iter()
                        .filter(|s| s.status != StepStatus::Pending && s.status != StepStatus::Running)
                        .count();
                    let pct = if self.finished { 1.0 } else { completed as f32 / total as f32 };
                    let label = if self.finished {
                        format!("upgraded {} out of {} apps", total, total)
                    } else if let Some(idx) = self.current_step_index {
                        format!("upgrading {} out of {} apps", idx, total)
                    } else {
                        format!("upgrading 0 out of {} apps", total)
                    };
                    // Center the bar horizontally in the footer
                    let bar_width = 320.0_f32;
                    ui.horizontal(|ui| {
                        let available = ui.available_width();
                        let pad = ((available - bar_width) / 2.0).max(0.0);
                        ui.add_space(pad);
                        egui::ProgressBar::new(pct)
                            .text(egui::RichText::new(label).size(typography::pt(typography::FOOTNOTE)))
                            .desired_width(bar_width)
                            .animate(!self.finished)
                            .ui(ui);
                    });
                });
        }

        match self.mode {
            UnifiedMode::Consent => {
                // Bottom bar
                let has_gate_fail = self.gate_status.iter().any(|g| !g.ok && g.name != "schedule" && g.name != "dialog_cooldown");
                egui::TopBottomPanel::bottom("consent_actions")
                    .frame(egui::Frame::new().fill(ctx.style().visuals.panel_fill).inner_margin(egui::Margin { left: 16, right: 16, top: 12, bottom: 12 }).stroke(egui::Stroke::new(1.0_f32, ctx.style().visuals.widgets.noninteractive.bg_stroke.color)))
                    .show(ctx, |ui| {
                        ui.horizontal(|ui| {
                            if has_gate_fail {
                                tint_frame(ui_theme::tint(self.theme, ui_theme::Hue::Warning)).inner_margin(egui::Margin::symmetric(10, 6)).show(ui, |ui| {
                                    ui.add(egui::Label::new(egui::RichText::new("Some checks failed — will still update if you proceed").size(typography::pt(typography::SUBHEADLINE)).color(ui_theme::tint(self.theme, ui_theme::Hue::Warning).fg)).wrap());
                                });
                            } else {
                                ui.add(egui::Label::new(egui::RichText::new("Press Enter to update • Esc to postpone").size(typography::pt(typography::SUBHEADLINE)).weak().italics()).wrap());
                            }
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if kit_button(ui, "Update Now", ui_theme::primary_button(self.theme), Some([124.0, 32.0])).clicked() {
                                    eprintln!("[dotfiles] Update Now clicked → starting progress in same window");
                                    self.start_progress(ctx);
                                }
                                if kit_button(ui, "Postpone", ui_theme::bordered_button(self.theme), Some([124.0, 32.0])).clicked() {
                                    eprintln!("[dotfiles] Postpone clicked");
                                    let mut s = dotfiles_core::state::State::load(&self.paths.state_file).unwrap_or_default();
                                    s.last_outcome = Some("postponed".into());
                                    s.last_dialog_at = Some(chrono::Utc::now().timestamp());
                                    let _ = s.save(&self.paths.state_file);
                                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                                }
                            });
                        });
                    });
                egui::CentralPanel::default().show(ctx, |ui| {
                    egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
                        self.consent_ui(ctx, ui);
                    });
                });
                if ctx.input(|i| i.key_pressed(egui::Key::Enter)) {
                    self.start_progress(ctx);
                }
                if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                    let mut s = dotfiles_core::state::State::load(&self.paths.state_file).unwrap_or_default();
                    s.last_outcome = Some("postponed".into());
                    s.last_dialog_at = Some(chrono::Utc::now().timestamp());
                    let _ = s.save(&self.paths.state_file);
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
            UnifiedMode::Progress | UnifiedMode::Done => {
                egui::CentralPanel::default().show(ctx, |ui| {
                    self.progress_ui(ctx, ui);
                });
            }
        }

        // Sudo modal (shared)
        let mut sudo_close = false;
        let mut sudo_response: Option<SudoResponse> = None;
        if let Some(prompt) = &mut self.sudo_prompt {
            egui::Window::new("Sudo required")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .frame(egui::Frame::window(&ctx.style()).corner_radius(12).inner_margin(16))
                .show(ctx, |ui| {
                    ui.vertical_centered(|ui| {
                        let danger_tint = ui_theme::tint(self.theme, ui_theme::Hue::Danger);
                        egui::Frame::new().fill(danger_tint.bg).corner_radius(20).inner_margin(8).show(ui, |ui| {
                            ui.label(egui::RichText::new("!").size(14.0).strong().color(danger_tint.fg));
                        });
                        ui.add_space(6.0);
                        ui.label(egui::RichText::new("Administrator password required").size(typography::pt(typography::BODY)).strong());
                    });
                    ui.add_space(8.0);
                    egui::Frame::new().fill(ui_theme::fill(self.theme, ui_theme::FillLevel::Secondary)).corner_radius(8).inner_margin(10).stroke(egui::Stroke::new(1.0_f32, ui_theme::separator(self.theme))).show(ui, |ui| {
                        ui.label(egui::RichText::new(&prompt.command).monospace().size(typography::pt(typography::SUBHEADLINE)));
                        ui.add_space(4.0);
                        ui.label(egui::RichText::new(&prompt.reason).size(typography::pt(typography::SUBHEADLINE)).weak());
                    });
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("Password").size(typography::pt(typography::SUBHEADLINE)).weak());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| { ui.checkbox(&mut prompt.show_password, "Show"); });
                    });
                    let resp = egui::TextEdit::singleline(&mut prompt.password).password(!prompt.show_password).hint_text("Enter password").desired_width(f32::INFINITY).show(ui).response;
                    if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        sudo_response = Some(SudoResponse::Password(prompt.password.clone()));
                        sudo_close = true;
                    }
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if kit_button(ui, "Unlock", ui_theme::primary_button(self.theme), Some([96.0, 28.0])).clicked() {
                                sudo_response = Some(SudoResponse::Password(prompt.password.clone()));
                                sudo_close = true;
                            }
                            if ui.button("Cancel").clicked() {
                                sudo_response = Some(SudoResponse::Cancel);
                                sudo_close = true;
                            }
                        });
                    });
                    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                        sudo_response = Some(SudoResponse::Cancel);
                        sudo_close = true;
                    }
                });
        }
        if sudo_close {
            if let Some(prompt) = self.sudo_prompt.take() {
                let resp = sudo_response.unwrap_or(SudoResponse::Cancel);
                if let Some(tx) = prompt.response_tx {
                    let _ = tx.send(resp.clone());
                }
                if let Some(tx) = prompt.stdin_respond {
                    // Empty answer = cancel → the pipeline closes the child's stdin (EOF)
                    let pw = match resp {
                        SudoResponse::Password(p) if !p.is_empty() => p,
                        _ => String::new(),
                    };
                    let _ = tx.send(pw);
                }
            }
        }

        if self.mode == UnifiedMode::Progress && !self.finished {
            ctx.request_repaint_after(std::time::Duration::from_millis(200));
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        let _ = std::fs::remove_file(crate::upgrade::askpass_socket_path());
        if self.mode == UnifiedMode::Consent {
            // Treat window close as Postpone (same as old on_exit)
            let mut s = dotfiles_core::state::State::load(&self.paths.state_file).unwrap_or_default();
            s.last_outcome = Some("postponed".into());
            let _ = s.save(&self.paths.state_file);
        }
    }
}

fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            if let Some(&next) = chars.peek() {
                if next == '[' {
                    chars.next();
                    for ch in chars.by_ref() {
                        if ch.is_ascii_alphabetic() { break; }
                    }
                    continue;
                } else if next == ']' {
                    chars.next();
                    for ch in chars.by_ref() {
                        if ch == '\x07' { break; }
                    }
                    continue;
                }
            }
            continue;
        }
        if c == '\r' { continue; }
        out.push(c);
    }
    out
}
