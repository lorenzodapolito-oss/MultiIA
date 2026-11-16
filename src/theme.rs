//! Palette pastello (niente bianco né grigio) e stile dell'interfaccia.

use eframe::egui::{self, Color32, CornerRadius, Stroke};

pub const BG: Color32 = Color32::from_rgb(0xD2, 0xE4, 0xFA);
pub const SURFACE: Color32 = Color32::from_rgb(0xE0, 0xEC, 0xFC);
pub const STRIPE: Color32 = Color32::from_rgb(0xC8, 0xDC, 0xF6);
pub const PRIMARY: Color32 = Color32::from_rgb(0x4F, 0x7C, 0xD6);
pub const PRIMARY_LIGHT: Color32 = Color32::from_rgb(0xB4, 0xD0, 0xF4);
pub const TEXT: Color32 = Color32::from_rgb(0x22, 0x31, 0x5E);
pub const MUTED: Color32 = Color32::from_rgb(0x58, 0x68, 0x98);
pub const BORDER: Color32 = Color32::from_rgb(0x8F, 0xBC, 0xE0);
pub const ERR: Color32 = Color32::from_rgb(0xBC, 0x3C, 0x58);
pub const BTN_BLUE: Color32 = Color32::from_rgb(0x9B, 0xBF, 0xF4);
pub const BTN_MINT: Color32 = Color32::from_rgb(0x9C, 0xDD, 0xC1);
pub const BTN_PEACH: Color32 = Color32::from_rgb(0xF6, 0xBF, 0x9C);
pub const BTN_LILAC: Color32 = Color32::from_rgb(0xCF, 0xBF, 0xF0);
pub const BTN_PINK: Color32 = Color32::from_rgb(0xF2, 0xBB, 0xD0);
pub const BTN_YELLOW: Color32 = Color32::from_rgb(0xF3, 0xE0, 0x9A);
pub const ON_COLOR: Color32 = Color32::from_rgb(0x1B, 0x28, 0x55);
pub const PLOT_PURITY: Color32 = Color32::from_rgb(0x3F, 0x7F, 0xD8);
pub const PLOT_DENSITY: Color32 = Color32::from_rgb(0x2E, 0xA0, 0x7A);

pub fn visuals() -> egui::Visuals {
    let mut v = egui::Visuals::light();
    v.panel_fill = BG;
    v.window_fill = SURFACE;
    v.extreme_bg_color = SURFACE;
    v.faint_bg_color = STRIPE;
    v.code_bg_color = SURFACE;
    v.override_text_color = Some(TEXT);
    v.weak_text_color = Some(MUTED);
    v.selection.bg_fill = PRIMARY_LIGHT;
    v.selection.stroke = Stroke::new(1.0, PRIMARY);
    v.hyperlink_color = PRIMARY;
    v.window_stroke = Stroke::new(1.0, BORDER);
    let shadow = Color32::from_rgba_unmultiplied(0x2B, 0x4A, 0x7A, 60);
    v.window_shadow.color = shadow;
    v.popup_shadow.color = shadow;
    v.text_cursor.stroke.color = PRIMARY;
    v.text_edit_bg_color = Some(SURFACE);
    let hover = Color32::from_rgb(0xA8, 0xC6, 0xF2);
    let press = Color32::from_rgb(0x96, 0xB8, 0xEE);
    for (w, fill) in [
        (&mut v.widgets.noninteractive, SURFACE),
        (&mut v.widgets.inactive, PRIMARY_LIGHT),
        (&mut v.widgets.hovered, hover),
        (&mut v.widgets.active, press),
        (&mut v.widgets.open, PRIMARY_LIGHT),
    ] {
        w.corner_radius = CornerRadius::same(9);
        w.bg_fill = fill;
        w.weak_bg_fill = fill;
        w.bg_stroke = Stroke::new(1.0, BORDER);
        w.fg_stroke.color = TEXT;
    }
    v.widgets.hovered.bg_stroke = Stroke::new(1.5, PRIMARY);
    v.widgets.active.bg_stroke = Stroke::new(1.5, PRIMARY);
    v
}

pub fn apply(ctx: &egui::Context) {
    let v = visuals();
    ctx.set_theme(egui::ThemePreference::Light);
    ctx.set_visuals_of(egui::Theme::Light, v.clone());
    ctx.set_visuals_of(egui::Theme::Dark, v);
    ctx.all_styles_mut(|s| {
        s.spacing.item_spacing = egui::vec2(8.0, 6.0);
        s.spacing.button_padding = egui::vec2(12.0, 6.0);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn neutral(c: Color32) -> bool {
        let [r, g, b, a] = c.to_srgba_unmultiplied().map(i32::from);
        a > 0 && r.max(g).max(b) - r.min(g).min(b) < 24
    }

    #[test]
    fn no_white_or_gray_anywhere() {
        let v = visuals();
        let mut c = vec![
            v.panel_fill,
            v.window_fill,
            v.extreme_bg_color,
            v.faint_bg_color,
            v.code_bg_color,
            v.hyperlink_color,
            v.selection.bg_fill,
            v.selection.stroke.color,
            v.window_stroke.color,
            v.window_shadow.color,
            v.popup_shadow.color,
            v.text_cursor.stroke.color,
        ];
        c.extend(v.override_text_color);
        c.extend(v.weak_text_color);
        c.extend(v.text_edit_bg_color);
        for w in [
            v.widgets.noninteractive,
            v.widgets.inactive,
            v.widgets.hovered,
            v.widgets.active,
            v.widgets.open,
        ] {
            c.extend([
                w.bg_fill,
                w.weak_bg_fill,
                w.bg_stroke.color,
                w.fg_stroke.color,
            ]);
        }
        c.extend([
            BG,
            SURFACE,
            STRIPE,
            PRIMARY,
            PRIMARY_LIGHT,
            TEXT,
            MUTED,
            BORDER,
            ERR,
            BTN_BLUE,
            BTN_MINT,
            BTN_PEACH,
            BTN_LILAC,
            BTN_PINK,
            BTN_YELLOW,
            ON_COLOR,
            PLOT_PURITY,
            PLOT_DENSITY,
        ]);
        for (i, col) in c.iter().enumerate() {
            assert!(!neutral(*col), "colore {i}: {col:?}");
        }
    }
}
