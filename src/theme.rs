//! Palette, egui style and small reusable drawings (gradients, icon).

use egui::{epaint::RectShape, Color32, CornerRadius, FontFamily, FontId, Rect, Stroke, TextStyle, TextureHandle};

pub const ACCENT_A: Color32 = Color32::from_rgb(0xFF, 0x3D, 0x6E);
pub const ACCENT_B: Color32 = Color32::from_rgb(0xFF, 0x8A, 0x3D);
pub const SUCCESS: Color32 = Color32::from_rgb(0x2E, 0xCC, 0x8F);
pub const DANGER: Color32 = Color32::from_rgb(0xFF, 0x5A, 0x5F);
pub const WARNING: Color32 = Color32::from_rgb(0xFF, 0xB8, 0x3D);

#[derive(Clone, Copy)]
pub struct Palette {
    pub bg: Color32,
    pub sidebar: Color32,
    pub surface: Color32,
    pub surface_hover: Color32,
    pub border: Color32,
    pub text: Color32,
    pub muted: Color32,
}

impl Palette {
    pub fn get(dark: bool) -> Self {
        if dark {
            Self {
                bg: Color32::from_rgb(0x0E, 0x10, 0x16),
                sidebar: Color32::from_rgb(0x13, 0x15, 0x1D),
                surface: Color32::from_rgb(0x1A, 0x1D, 0x27),
                surface_hover: Color32::from_rgb(0x23, 0x27, 0x34),
                border: Color32::from_rgb(0x2A, 0x2E, 0x3B),
                text: Color32::from_rgb(0xEC, 0xEE, 0xF4),
                muted: Color32::from_rgb(0x8E, 0x94, 0xA8),
            }
        } else {
            Self {
                bg: Color32::from_rgb(0xF4, 0xF5, 0xF9),
                sidebar: Color32::from_rgb(0xFF, 0xFF, 0xFF),
                surface: Color32::from_rgb(0xFF, 0xFF, 0xFF),
                surface_hover: Color32::from_rgb(0xF0, 0xF1, 0xF6),
                border: Color32::from_rgb(0xE2, 0xE4, 0xEC),
                text: Color32::from_rgb(0x16, 0x18, 0x22),
                muted: Color32::from_rgb(0x6B, 0x70, 0x84),
            }
        }
    }
}

pub fn apply(ctx: &egui::Context, dark: bool) {
    let p = Palette::get(dark);
    let mut visuals = if dark { egui::Visuals::dark() } else { egui::Visuals::light() };
    visuals.panel_fill = p.bg;
    visuals.window_fill = p.surface;
    visuals.extreme_bg_color = if dark { Color32::from_rgb(0x0B, 0x0C, 0x11) } else { Color32::from_rgb(0xEE, 0xEF, 0xF5) };
    visuals.faint_bg_color = p.surface_hover;
    visuals.window_stroke = Stroke::new(1.0, p.border);
    visuals.window_corner_radius = CornerRadius::same(16);
    visuals.menu_corner_radius = CornerRadius::same(10);
    visuals.hyperlink_color = ACCENT_B;
    visuals.selection.bg_fill = ACCENT_A.gamma_multiply(0.85);
    visuals.selection.stroke = Stroke::new(1.0, Color32::WHITE);
    visuals.override_text_color = Some(p.text);

    let radius = CornerRadius::same(10);
    for w in [
        &mut visuals.widgets.noninteractive,
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
        &mut visuals.widgets.open,
    ] {
        w.corner_radius = radius;
    }
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
    visuals.widgets.inactive.weak_bg_fill = p.surface_hover;
    visuals.widgets.inactive.bg_fill = p.surface_hover;
    visuals.widgets.inactive.bg_stroke = Stroke::NONE;
    visuals.widgets.hovered.weak_bg_fill = p.border;
    visuals.widgets.hovered.bg_fill = p.border;
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, ACCENT_A.gamma_multiply(0.6));
    visuals.widgets.active.weak_bg_fill = ACCENT_A.gamma_multiply(0.35);
    visuals.widgets.active.bg_fill = ACCENT_A.gamma_multiply(0.35);
    ctx.set_visuals(visuals);

    ctx.global_style_mut(|style| {
        style.spacing.item_spacing = egui::vec2(10.0, 10.0);
        style.spacing.button_padding = egui::vec2(14.0, 8.0);
        style.spacing.interact_size.y = 34.0;
        style.spacing.slider_width = 220.0;
        style.text_styles = [
            (TextStyle::Heading, FontId::new(26.0, FontFamily::Proportional)),
            (TextStyle::Body, FontId::new(15.0, FontFamily::Proportional)),
            (TextStyle::Button, FontId::new(15.0, FontFamily::Proportional)),
            (TextStyle::Small, FontId::new(12.5, FontFamily::Proportional)),
            (TextStyle::Monospace, FontId::new(13.0, FontFamily::Monospace)),
        ]
        .into();
    });
}

/// 256×1 texture with the brand gradient, used to paint rounded rectangles.
pub fn gradient_texture(ctx: &egui::Context) -> TextureHandle {
    let w = 256;
    let pixels: Vec<Color32> = (0..w)
        .map(|x| lerp_color(ACCENT_A, ACCENT_B, x as f32 / (w - 1) as f32))
        .collect();
    let image = egui::ColorImage::new([w, 1], pixels);
    ctx.load_texture("fluxo-gradient", image, egui::TextureOptions::LINEAR)
}

pub fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

pub fn gradient_rect(painter: &egui::Painter, tex: &TextureHandle, rect: Rect, radius: impl Into<CornerRadius>) {
    let uv = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
    painter.add(RectShape::filled(rect, radius, Color32::WHITE).with_texture(tex.id(), uv));
}

/// App icon (gradient circle with a download arrow), generated in code.
pub fn app_icon(size: u32) -> egui::IconData {
    let mut rgba = vec![0u8; (size * size * 4) as usize];
    let s = size as f32;
    let c = s / 2.0;
    for y in 0..size {
        for x in 0..size {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            let r = ((fx - c).powi(2) + (fy - c).powi(2)).sqrt();
            let edge = (c - r).clamp(0.0, 1.0);
            if edge <= 0.0 {
                continue;
            }
            let t = ((fx + fy) / (2.0 * s)).clamp(0.0, 1.0);
            let mut col = lerp_color(ACCENT_A, ACCENT_B, t);
            // Arrow: stem + triangular head
            let (nx, ny) = ((fx - c) / s, (fy - c) / s);
            let stem = nx.abs() < 0.07 && ny > -0.28 && ny < 0.08;
            let head = ny >= 0.0 && ny < 0.22 && nx.abs() < (0.22 - ny) * 1.1;
            let base = ny > 0.27 && ny < 0.33 && nx.abs() < 0.24;
            if stem || head || base {
                col = Color32::WHITE;
            }
            let i = ((y * size + x) * 4) as usize;
            rgba[i..i + 4].copy_from_slice(&[col.r(), col.g(), col.b(), (edge * 255.0) as u8]);
        }
    }
    egui::IconData { rgba, width: size, height: size }
}

#[cfg(test)]
mod tests {
    /// Writes `assets/icon.png` (used by the menu shortcut and README): `cargo test -- --ignored export_icon`.
    #[test]
    #[ignore]
    fn export_icon() {
        let icon = super::app_icon(256);
        std::fs::create_dir_all("assets").unwrap();
        image::save_buffer("assets/icon.png", &icon.rgba, icon.width, icon.height, image::ExtendedColorType::Rgba8).unwrap();
    }
}
