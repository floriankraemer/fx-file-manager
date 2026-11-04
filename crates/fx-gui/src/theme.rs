//! Themes (built-in and user TOML files), compact spacing, and the age-bucket color mapping.
//! Colors are functions because they follow the active theme.

use eframe::egui::{self, Color32, CornerRadius, FontId, Stroke, TextStyle};
use fx_app::{AgeBucket, DEFAULT_THEME, Settings};
use serde::Deserialize;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock, RwLock};

/// Declares `Palette` (resolved colors) and `PaletteFile` (optional hex strings) from one field list.
macro_rules! palette {
    ($($f:ident),*) => {
        #[derive(Clone, Copy, Debug)]
        pub struct Palette {
            $(pub $f: Color32),*
        }

        #[derive(Deserialize)]
        struct PaletteFile {
            $($f: Option<String>),*
        }

        impl Palette {
            const BLANK: Palette = Palette { $($f: Color32::BLACK),* };

            /// File values over `base`; a malformed color is an error.
            fn from_file(file: &PaletteFile, base: Palette) -> Result<Palette, String> {
                Ok(Palette {
                    $($f: match &file.$f {
                        Some(hex) => parse_hex(hex)
                            .ok_or_else(|| format!("invalid color for {}: {hex}", stringify!($f)))?,
                        None => base.$f,
                    }),*
                })
            }
        }
    };
}
palette!(
    bg,
    panel,
    panel_hi,
    border,
    text,
    dim,
    accent,
    header_text,
    selected,
    error,
    faint,
    separator,
    selection_bg,
    age_minutes,
    age_hours,
    age_days,
    age_weeks,
    age_months,
    age_years
);

fn parse_hex(s: &str) -> Option<Color32> {
    let h = s.trim().trim_start_matches('#');
    if h.len() != 6 || !h.is_ascii() {
        return None;
    }
    let v = u32::from_str_radix(h, 16).ok()?;
    Some(Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8))
}

/// Which egui `Visuals` a theme starts from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Base {
    Dark,
    Light,
}

#[derive(Debug)]
pub struct Theme {
    pub id: String,
    pub name: String,
    pub base: Base,
    pub palette: Palette,
}

#[derive(Deserialize)]
struct ThemeFile {
    name: String,
    base: Base,
    #[serde(flatten)]
    colors: PaletteFile,
}

fn parse_theme(
    id: &str,
    text: &str,
    base_palette: impl Fn(Base) -> Palette,
) -> Result<Theme, String> {
    let file: ThemeFile = toml::from_str(text).map_err(|e| e.to_string())?;
    Ok(Theme {
        id: id.to_owned(),
        name: file.name,
        base: file.base,
        palette: Palette::from_file(&file.colors, base_palette(file.base))?,
    })
}

const BUILTIN: &[(&str, &str)] = &[
    ("nord-dark", include_str!("../themes/nord-dark.toml")),
    ("nord-light", include_str!("../themes/nord-light.toml")),
    (
        "solarized-dark",
        include_str!("../themes/solarized-dark.toml"),
    ),
    (
        "solarized-light",
        include_str!("../themes/solarized-light.toml"),
    ),
    ("dracula", include_str!("../themes/dracula.toml")),
    ("gruvbox-dark", include_str!("../themes/gruvbox-dark.toml")),
    (
        "high-contrast",
        include_str!("../themes/high-contrast.toml"),
    ),
];

fn builtins() -> Vec<Arc<Theme>> {
    BUILTIN
        .iter()
        .map(|(id, text)| {
            Arc::new(parse_theme(id, text, |_| Palette::BLANK).expect("built-in theme is valid"))
        })
        .collect()
}

/// Reads every `*.toml` in `dir`; broken files are reported on stderr and skipped.
/// Colors a file leaves out come from Nord Dark or Nord Light, matching its `base`.
fn load_user_themes(dir: &Path, builtin: &[Arc<Theme>]) -> Vec<Arc<Theme>> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let fallback = |base| {
        let id = if base == Base::Light {
            "nord-light"
        } else {
            DEFAULT_THEME
        };
        builtin
            .iter()
            .find(|t| t.id == id)
            .map_or(Palette::BLANK, |t| t.palette)
    };
    let mut out: Vec<Arc<Theme>> = Vec::new();
    for path in entries.flatten().map(|e| e.path()) {
        if path.extension().and_then(|e| e.to_str()) != Some("toml") {
            continue;
        }
        let Some(id) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        if builtin.iter().any(|t| t.id == id) {
            eprintln!(
                "fx: ignoring theme {}: id clashes with a built-in theme",
                path.display()
            );
            continue;
        }
        match std::fs::read_to_string(&path)
            .map_err(|e| e.to_string())
            .and_then(|text| parse_theme(id, &text, fallback))
        {
            Ok(theme) => out.push(Arc::new(theme)),
            Err(err) => eprintln!("fx: ignoring theme {}: {err}", path.display()),
        }
    }
    out.sort_by_key(|t| t.name.to_lowercase());
    out
}

fn registry() -> &'static RwLock<Vec<Arc<Theme>>> {
    static THEMES: OnceLock<RwLock<Vec<Arc<Theme>>>> = OnceLock::new();
    THEMES.get_or_init(|| RwLock::new(scan()))
}

fn scan() -> Vec<Arc<Theme>> {
    let mut all = builtins();
    if let Some(dir) = Settings::themes_dir() {
        let user = load_user_themes(&dir, &all);
        all.extend(user);
    }
    all
}

/// All themes: built-ins first, then user themes by name.
pub fn themes() -> Vec<Arc<Theme>> {
    registry().read().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Rescans the user themes folder.
pub fn reload() {
    let all = scan();
    *registry().write().unwrap_or_else(|e| e.into_inner()) = all;
}

/// The theme with `id`, or the default theme when there is none.
fn find(id: &str) -> Arc<Theme> {
    let all = themes();
    all.iter()
        .find(|t| t.id == id)
        .or_else(|| all.iter().find(|t| t.id == DEFAULT_THEME))
        .cloned()
        .expect("default theme exists")
}

fn active() -> &'static RwLock<Arc<Theme>> {
    static ACTIVE: OnceLock<RwLock<Arc<Theme>>> = OnceLock::new();
    ACTIVE.get_or_init(|| RwLock::new(find(DEFAULT_THEME)))
}

fn palette() -> Palette {
    active().read().unwrap_or_else(|e| e.into_inner()).palette
}

pub fn is_light() -> bool {
    active().read().unwrap_or_else(|e| e.into_inner()).base == Base::Light
}

macro_rules! colors {
    ($($name:ident),*) => {$(
        pub fn $name() -> Color32 {
            palette().$name
        }
    )*};
}
colors!(
    bg,
    panel_hi,
    border,
    text,
    dim,
    accent,
    header_text,
    selected,
    error
);

pub fn age_color(bucket: AgeBucket) -> Color32 {
    let p = palette();
    match bucket {
        AgeBucket::Minutes => p.age_minutes,
        AgeBucket::Hours => p.age_hours,
        AgeBucket::Days => p.age_days,
        AgeBucket::Weeks => p.age_weeks,
        AgeBucket::Months => p.age_months,
        AgeBucket::Years => p.age_years,
    }
}

pub fn list_style() -> TextStyle {
    TextStyle::Name("List".into())
}

pub fn tree_style() -> TextStyle {
    TextStyle::Name("Tree".into())
}

/// Row height for a text size: at least `min`, and the font size plus padding.
pub fn row_height(min: f32, font_size: f32) -> f32 {
    min.max(font_size + 6.0)
}

/// Sorted, deduplicated installed font family names (scanned once).
pub fn system_font_families() -> &'static [String] {
    static FAMILIES: OnceLock<Vec<String>> = OnceLock::new();
    FAMILIES.get_or_init(|| {
        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        let mut names: Vec<String> = db
            .faces()
            .flat_map(|f| f.families.iter().map(|(n, _)| n.clone()))
            .collect();
        names.sort_by_key(|n| n.to_lowercase());
        names.dedup();
        names
    })
}

/// Loads the first installed family of `families`.
fn load_font(families: &[&str]) -> Option<egui::FontData> {
    let mut db = fontdb::Database::new();
    db.load_system_fonts();
    families.iter().find_map(|family| {
        let query = fontdb::Query {
            families: &[fontdb::Family::Name(family)],
            ..Default::default()
        };
        let id = db.query(&query)?;
        db.with_face_data(id, |data, index| {
            let mut font = egui::FontData::from_owned(data.to_vec());
            font.index = index;
            font
        })
    })
}

/// System fonts with Simplified Chinese glyphs on Linux, Windows and macOS. Nothing is bundled to keep the binary small;
/// without one of these installed, Chinese text shows as boxes.
const CJK_FONTS: &[&str] = &[
    "Noto Sans CJK SC",
    "Noto Sans SC",
    "Source Han Sans SC",
    "WenQuanYi Micro Hei",
    "WenQuanYi Zen Hei",
    "Droid Sans Fallback",
    "Microsoft YaHei",
    "SimSun",
    "PingFang SC",
    "Hiragino Sans GB",
    "Heiti SC",
];

/// Installs `family` as the first proportional font and, for Chinese, a CJK fallback last.
/// Only touches egui when one of them changed.
fn apply_font(ctx: &egui::Context, family: Option<&str>, cjk: bool) {
    type Applied = (Option<String>, bool);
    static LAST: Mutex<Option<Applied>> = Mutex::new(None);
    let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
    let wanted = (family.map(str::to_owned), cjk);
    if last.as_ref() == Some(&wanted) {
        return;
    }
    let mut fonts = egui::FontDefinitions::default();
    if let Some(data) = family.and_then(|f| load_font(&[f])) {
        fonts.font_data.insert("user".to_owned(), data.into());
        if let Some(list) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
            list.insert(0, "user".to_owned());
        }
    }
    if cjk && let Some(data) = load_font(CJK_FONTS) {
        fonts.font_data.insert("cjk".to_owned(), data.into());
        for list in fonts.families.values_mut() {
            list.push("cjk".to_owned());
        }
    }
    ctx.set_fonts(fonts);
    *last = Some(wanted);
}

/// Display name of a theme: translated for built-in themes, the file's `name` otherwise.
pub fn display_name(theme: &Theme) -> String {
    let key = format!("theme.{}", theme.id);
    fx_app::i18n::find_in(fx_app::i18n::current(), &key)
        .map_or_else(|| theme.name.clone(), str::to_owned)
}

pub fn apply(ctx: &egui::Context, settings: &Settings) {
    apply_font(
        ctx,
        settings.font_family.as_deref(),
        settings.language.needs_cjk(),
    );
    let chosen = find(&settings.theme);
    let p = &chosen.palette;
    let (theme, base) = match chosen.base {
        Base::Light => (egui::Theme::Light, egui::Visuals::light()),
        Base::Dark => (egui::Theme::Dark, egui::Visuals::dark()),
    };
    let mut style = (*ctx.style_of(theme)).clone();
    style.text_styles = [
        (TextStyle::Small, FontId::proportional(11.0)),
        (TextStyle::Body, FontId::proportional(13.5)),
        (TextStyle::Button, FontId::proportional(13.5)),
        (TextStyle::Heading, FontId::proportional(16.0)),
        (TextStyle::Monospace, FontId::monospace(13.0)),
        (list_style(), FontId::proportional(settings.list_font_size)),
        (tree_style(), FontId::proportional(settings.tree_font_size)),
    ]
    .into();
    style.spacing.item_spacing = egui::vec2(6.0, 3.0);
    style.spacing.button_padding = egui::vec2(6.0, 2.0);
    style.spacing.interact_size.y = 20.0;

    let v = &mut style.visuals;
    *v = base;
    v.panel_fill = p.panel;
    v.window_fill = p.panel;
    v.extreme_bg_color = p.bg;
    v.faint_bg_color = p.faint;
    v.override_text_color = Some(p.text);
    v.hyperlink_color = p.accent;
    v.selection.bg_fill = p.selection_bg;
    v.selection.stroke = Stroke::new(1.0, p.accent);
    v.window_stroke = Stroke::new(1.0, p.border);
    let radius = CornerRadius::same(3);
    for w in [
        &mut v.widgets.noninteractive,
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        w.corner_radius = radius;
    }
    // Subtle: also drives the table's column separator lines.
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, p.separator);
    v.widgets.inactive.weak_bg_fill = p.panel_hi;
    v.widgets.inactive.bg_fill = p.panel_hi;
    v.widgets.hovered.weak_bg_fill = p.border;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, p.accent);
    v.widgets.active.weak_bg_fill = p.border;
    v.widgets.open.weak_bg_fill = p.border;
    *active().write().unwrap_or_else(|e| e.into_inner()) = chosen.clone();
    ctx.set_theme(theme);
    ctx.set_style_of(theme, style);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn luminance(c: Color32) -> f64 {
        let lin = |v: u8| {
            let v = f64::from(v) / 255.0;
            if v <= 0.03928 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * lin(c.r()) + 0.7152 * lin(c.g()) + 0.0722 * lin(c.b())
    }

    fn contrast(a: Color32, b: Color32) -> f64 {
        let (x, y) = (luminance(a), luminance(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    #[test]
    fn row_height_grows_with_font() {
        assert_eq!(row_height(20.0, 13.0), 20.0);
        assert_eq!(row_height(20.0, 32.0), 38.0);
        assert_eq!(row_height(28.0, 22.0), 28.0);
    }

    #[test]
    fn builtin_themes_are_readable() {
        let all = builtins();
        assert!(all.len() >= 4);
        for t in all {
            let p = t.palette;
            assert!(contrast(p.text, p.bg) >= 4.5, "{} text/bg", t.id);
            assert!(contrast(p.text, p.panel) >= 4.5, "{} text/panel", t.id);
            assert!(contrast(p.dim, p.bg) >= 3.0, "{} dim/bg", t.id);
        }
    }

    #[test]
    fn builtin_ids_are_unique_and_include_default() {
        let mut ids: Vec<_> = BUILTIN.iter().map(|(id, _)| *id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), BUILTIN.len());
        assert!(ids.contains(&DEFAULT_THEME));
    }

    #[test]
    fn user_theme_falls_back_to_base_and_broken_files_are_skipped() {
        let dir = std::env::temp_dir().join(format!("fx-themes-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let write = |name: &str, text: &str| std::fs::write(dir.join(name), text).unwrap();
        write(
            "mine.toml",
            "name = \"Mine\"\nbase = \"light\"\naccent = \"#ff0000\"\n",
        );
        write(
            "bad.toml",
            "name = \"Bad\"\nbase = \"light\"\nbg = \"nope\"\n",
        );
        write("junk.toml", "not toml [");
        write("nord-dark.toml", "name = \"Clash\"\nbase = \"dark\"\n");
        let all = builtins();
        let user = load_user_themes(&dir, &all);
        std::fs::remove_dir_all(&dir).ok();
        assert_eq!(user.len(), 1);
        let nord_light = all.iter().find(|t| t.id == "nord-light").unwrap();
        assert_eq!(user[0].palette.accent, Color32::from_rgb(255, 0, 0));
        assert_eq!(user[0].palette.bg, nord_light.palette.bg);
    }

    #[test]
    fn unknown_theme_id_falls_back_to_default() {
        assert_eq!(find("does-not-exist").id, DEFAULT_THEME);
    }
}
