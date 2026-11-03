//! Entry icons (Material icon theme, embedded and rasterised on demand) and
//! stroke-drawn command-bar glyphs.

use crate::theme;
use eframe::egui::{
    self, Color32, ColorImage, Context, Rect, Shape, Stroke, TextureHandle, TextureOptions, Ui,
    pos2, vec2,
};
use fx_core::EntryKind;
use icon_theme::{Appearance, IconAssets, IconError, IconPack, IconRenderer};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

mod embedded {
    include!(concat!(env!("OUT_DIR"), "/icons_embedded.rs"));
}

const ICON_PT: f32 = 16.0;

struct Embedded;

impl IconAssets for Embedded {
    fn read(&self, relative: &Path) -> Result<Vec<u8>, IconError> {
        let rel = relative.to_string_lossy().replace('\\', "/");
        embedded::asset(&rel)
            .map(<[u8]>::to_vec)
            .ok_or_else(|| IconError::UnreadableAsset {
                path: rel,
                message: "not embedded".into(),
            })
    }
}

struct Themed {
    pack: IconPack,
    renderer: IconRenderer,
    textures: HashMap<(String, u32), TextureHandle>,
}

type Shared = Arc<Mutex<Themed>>;

fn shared(ctx: &Context) -> Shared {
    ctx.data_mut(|d| {
        d.get_temp_mut_or_insert_with(egui::Id::unique("fx-icon-theme"), || {
            Arc::new(Mutex::new(Themed {
                pack: IconPack::from_toml_str(embedded::PACK_TOML).expect("embedded pack.toml"),
                renderer: IconRenderer::new(),
                textures: HashMap::new(),
            }))
        })
        .clone()
    })
}

fn appearance() -> Appearance {
    if theme::is_light() {
        Appearance::Light
    } else {
        Appearance::Dark
    }
}

/// Paints the icon chosen by `pick` into `rect`.
fn paint_themed(ui: &Ui, rect: Rect, pick: impl FnOnce(&IconPack, Appearance) -> String) {
    let px = (ICON_PT * ui.pixels_per_point()).round() as u32;
    let shared = shared(ui.ctx());
    let mut t = shared.lock().unwrap_or_else(|e| e.into_inner());
    let t = &mut *t;
    let id = pick(&t.pack, appearance());
    let key = (id, px);
    if !t.textures.contains_key(&key) {
        let Ok(icon) = t.renderer.render(&t.pack, &Embedded, &key.0, px) else {
            return;
        };
        let image = ColorImage::from_rgba_premultiplied(
            [icon.width as usize, icon.height as usize],
            &icon.pixels,
        );
        let tex = ui.ctx().load_texture(
            format!("icon:{}:{px}", key.0),
            image,
            TextureOptions::LINEAR,
        );
        t.textures.insert(key.clone(), tex);
    }
    let r = Rect::from_center_size(rect.center(), vec2(ICON_PT, ICON_PT));
    let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    ui.painter()
        .image(t.textures[&key].id(), r, uv, Color32::WHITE);
}

pub fn paint_file(ui: &Ui, rect: Rect, name: &str) {
    paint_themed(ui, rect, |p, a| p.file_icon(name, None, a).to_owned());
}

pub fn paint_folder(ui: &Ui, rect: Rect, name: &str, expanded: bool) {
    paint_themed(ui, rect, |p, a| p.folder_icon(name, expanded, a).to_owned());
}

pub fn entry(ui: &mut Ui, kind: EntryKind, name: &str) {
    let (rect, _) = ui.allocate_exact_size(vec2(ICON_PT, ICON_PT), egui::Sense::hover());
    match kind {
        EntryKind::Dir => paint_folder(ui, rect, name, false),
        EntryKind::Symlink => {
            paint_file(ui, rect, name);
            arrow(ui, rect);
        }
        EntryKind::File | EntryKind::Other => paint_file(ui, rect, name),
    }
}

/// Small up-right arrow in the icon's bottom-left corner.
fn arrow(ui: &Ui, r: Rect) {
    let c = r.left_bottom() + vec2(3.5, -3.5);
    let (a, b) = (c + vec2(-1.5, 1.5), c + vec2(1.5, -1.5));
    let p = ui.painter();
    p.circle_filled(c, 4.0, theme::bg().gamma_multiply(0.85));
    let stroke = Stroke::new(1.3, theme::accent());
    p.line_segment([a, b], stroke);
    p.line_segment([b + vec2(-2.5, 0.0), b], stroke);
    p.line_segment([b, b + vec2(0.0, 2.5)], stroke);
}

/// Command-bar glyphs, drawn from strokes so they render without any icon font.
#[derive(Clone, Copy)]
pub enum Glyph {
    NewFolder,
    Rename,
    Delete,
    Sort,
    More,
    Chevron,
    ChevronRight,
}

/// Paints `glyph` centered in `rect` (a 16x16 area).
pub fn glyph(ui: &Ui, rect: Rect, glyph: Glyph, color: Color32) {
    let p = ui.painter();
    let c = rect.center();
    let s = Stroke::new(1.3, color);
    let at = |x: f32, y: f32| c + vec2(x, y);
    match glyph {
        Glyph::NewFolder => {
            let body = Rect::from_min_max(at(-7.0, -4.0), at(5.0, 5.0));
            p.rect_stroke(body, 1.5, s, egui::StrokeKind::Inside);
            p.line_segment([at(-7.0, -4.0), at(-7.0, -6.0)], s);
            p.line_segment([at(-7.0, -6.0), at(-2.5, -6.0)], s);
            p.line_segment([at(-2.5, -6.0), at(-1.0, -4.0)], s);
            let plus = Stroke::new(1.5, theme::accent());
            p.line_segment([at(5.0, 0.0), at(5.0, 7.0)], plus);
            p.line_segment([at(1.5, 3.5), at(8.5, 3.5)], plus);
        }
        Glyph::Rename => {
            let field = Rect::from_min_max(at(-8.0, -4.5), at(8.0, 4.5));
            p.rect_stroke(field, 2.0, s, egui::StrokeKind::Inside);
            p.line_segment(
                [at(0.0, -3.0), at(0.0, 3.0)],
                Stroke::new(1.5, theme::accent()),
            );
            p.line_segment([at(-1.8, -3.0), at(1.8, -3.0)], s);
            p.line_segment([at(-1.8, 3.0), at(1.8, 3.0)], s);
        }
        Glyph::Delete => {
            p.line_segment([at(-6.0, -4.5), at(6.0, -4.5)], s);
            p.line_segment([at(-2.0, -4.5), at(-2.0, -6.5)], s);
            p.line_segment([at(2.0, -4.5), at(2.0, -6.5)], s);
            p.line_segment([at(-2.0, -6.5), at(2.0, -6.5)], s);
            let can = vec![at(-4.5, -4.5), at(4.5, -4.5), at(3.8, 6.5), at(-3.8, 6.5)];
            p.add(Shape::closed_line(can, s));
            p.line_segment([at(-1.5, -2.0), at(-1.5, 4.0)], s);
            p.line_segment([at(1.5, -2.0), at(1.5, 4.0)], s);
        }
        Glyph::Sort => {
            for (i, half) in [7.0, 5.0, 3.0].into_iter().enumerate() {
                let y = -4.5 + 4.5 * i as f32;
                p.line_segment([at(-8.0, y), at(-8.0 + half * 1.6, y)], s);
            }
            let x = 6.0;
            p.line_segment([at(x, -6.0), at(x, 5.0)], s);
            p.line_segment([at(x - 2.5, 2.5), at(x, 5.5)], s);
            p.line_segment([at(x + 2.5, 2.5), at(x, 5.5)], s);
        }
        Glyph::More => {
            for dx in [-5.0, 0.0, 5.0] {
                p.circle_filled(at(dx, 0.0), 1.5, color);
            }
        }
        Glyph::ChevronRight => {
            p.line_segment([at(-1.5, -3.0), at(1.5, 0.0)], s);
            p.line_segment([at(-1.5, 3.0), at(1.5, 0.0)], s);
        }
        Glyph::Chevron => {
            p.line_segment([at(-3.0, -1.5), at(0.0, 1.5)], s);
            p.line_segment([at(3.0, -1.5), at(0.0, 1.5)], s);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_pack_has_default_assets() {
        let pack = IconPack::from_toml_str(embedded::PACK_TOML).unwrap();
        for id in [
            pack.default_file_icon(Appearance::Dark),
            pack.folder_icon("", false, Appearance::Dark),
        ] {
            let rel = pack.asset_path(id);
            let rel = rel.to_string_lossy().replace('\\', "/");
            assert!(
                embedded::asset(&rel).is_some_and(|b| !b.is_empty()),
                "{rel}"
            );
        }
    }
}
