//! IDE-style settings dialog: searchable category list on the left, option rows on the right.
//!
//! Every option is declared once in [`ROWS`]; pages and search both derive from that registry.

mod keyboard;
mod layout;
mod rows;

use crate::command::{COMMANDS, Command, Def};
use crate::theme;
use eframe::egui::{
    self, Align, Context, Key, Layout, Modifiers, RichText, ScrollArea, TextEdit, Ui,
    ViewportBuilder, ViewportClass, ViewportCommand, ViewportId,
};
use fx_app::{Explorer, Settings, t};
use keyboard::keyboard_section;
use layout::{category_row, form, group_box, title};
pub use rows::Form;
use rows::{ROWS, Row};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Category {
    General,
    Appearance,
    Fonts,
    Listing,
    Behavior,
    Transfers,
    Keyboard,
}

/// Layout metrics, matching the IDE's settings dialog (4/8/12/16 px spacing, 26 px controls).
const SIDEBAR_WIDTH: f32 = 200.0;
const SIDEBAR_ROW: f32 = 26.0;
const CONTROL_HEIGHT: f32 = 26.0;
const GROUP_GAP: f32 = 12.0;
const ROW_GAP: f32 = 8.0;
const TABLE_ROW: f32 = 24.0;
/// Indent of a note under a checkbox: the box plus its gap.
const CHECK_INDENT: f32 = 24.0;
const COLUMN_GAP: f32 = 16.0;
const FIELD_WIDTH: f32 = 320.0;
const COMBO_WIDTH: f32 = 240.0;
const BUTTON_MIN_WIDTH: f32 = 84.0;

impl Category {
    pub const ALL: [Category; 7] = [
        Category::General,
        Category::Appearance,
        Category::Fonts,
        Category::Listing,
        Category::Behavior,
        Category::Transfers,
        Category::Keyboard,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Category::General => t("settings.cat.general"),
            Category::Appearance => t("settings.cat.appearance"),
            Category::Fonts => t("settings.cat.fonts"),
            Category::Listing => t("settings.cat.listing"),
            Category::Behavior => t("settings.cat.behavior"),
            Category::Transfers => t("settings.cat.transfers"),
            Category::Keyboard => t("settings.cat.keyboard"),
        }
    }
}

/// Rows whose label or category label contains `query` (case-insensitive substring).
fn search(query: &str) -> Vec<&'static Row> {
    let q = query.trim().to_lowercase();
    ROWS.iter()
        .filter(|r| {
            t(r.label).to_lowercase().contains(&q)
                || r.category.label().to_lowercase().contains(&q)
                || (r.label == "settings.theme"
                    && ["color", "colour", "dark", "light"].contains(&q.as_str()))
        })
        .collect()
}

/// Commands whose title, group or the Keyboard category label contains `query`.
fn search_commands(query: &str) -> Vec<&'static Def> {
    let q = query.trim().to_lowercase();
    let keyboard = Category::Keyboard.label().to_lowercase().contains(&q);
    COMMANDS
        .iter()
        .filter(|d| keyboard || t(d.title).to_lowercase().contains(&q))
        .collect()
}

/// Whether the search `query` finds anything in `category`.
fn category_matches(category: Category, query: &str) -> bool {
    if category == Category::Keyboard {
        !search_commands(query).is_empty()
    } else {
        search(query).iter().any(|r| r.category == category)
    }
}

pub struct State {
    pub draft: Settings,
    pub start_text: String,
    pub category: Category,
    pub query: String,
    /// The command selected in the Keyboard list.
    selected: Option<Command>,
    /// The command whose next key combination is being captured.
    recording: Option<Command>,
    /// What the live preview last applied.
    previewed: Settings,
    /// Rendered inside the main window (no native viewports available).
    embedded: bool,
}

impl State {
    pub fn new(saved: &Settings) -> Self {
        Self {
            draft: saved.clone(),
            start_text: saved
                .start_folder
                .as_ref()
                .map_or_else(String::new, |p| p.display().to_string()),
            category: Category::General,
            query: String::new(),
            selected: None,
            recording: None,
            previewed: saved.clone(),
            embedded: false,
        }
    }

    /// The draft as it would be saved: with the typed start folder.
    fn pending(&self) -> Settings {
        let mut settings = self.draft.clone();
        let folder = self.start_text.trim();
        settings.start_folder = (!folder.is_empty()).then(|| PathBuf::from(folder));
        settings
    }

    /// Whether the settings window is shown inside the main window rather than as its own.
    pub fn embedded(&self) -> bool {
        self.embedded
    }
}

fn look_changed(a: &Settings, b: &Settings) -> bool {
    a.theme != b.theme
        || a.font_family != b.font_family
        || a.list_font_size != b.list_font_size
        || a.tree_font_size != b.tree_font_size
}

/// The right-hand side: the selected category's page, or search results across all pages.
fn page(ui: &mut Ui, st: &mut State, current_folder: String) {
    let query = st.query.trim().to_lowercase();
    let searching = !query.is_empty();
    let rows: Vec<&Row> = if searching {
        search(&query)
    } else {
        ROWS.iter().filter(|r| r.category == st.category).collect()
    };
    let keyboard_page = !searching && st.category == Category::Keyboard;
    let commands: Vec<&Def> = if searching {
        search_commands(&query)
    } else if keyboard_page {
        COMMANDS.iter().collect()
    } else {
        Vec::new()
    };
    if !searching {
        title(ui, st.category.label());
    }
    if rows.is_empty() && commands.is_empty() {
        ui.add_space(8.0);
        ui.vertical_centered(|ui| {
            ui.label(RichText::new(t("settings.none")).color(theme::dim()));
        });
        return;
    }
    let mut f = Form {
        draft: &mut st.draft,
        start_text: &mut st.start_text,
        current_folder,
    };
    form(ui, &rows, &mut f, searching);
    if commands.is_empty() {
        return;
    }
    if keyboard_page {
        // Fill the page: list, then strip (26) + note (~20) + gaps.
        let height = (ui.available_height() - TABLE_ROW - CONTROL_HEIGHT - 48.0).max(120.0);
        keyboard_section(ui, st, &commands, height);
    } else {
        group_box(ui, Category::Keyboard.label(), |ui| {
            keyboard_section(ui, st, &commands, 220.0);
        });
    }
}

/// The sidebar: search field and the category list (dimmed when the search finds nothing there).
fn sidebar(ui: &mut Ui, st: &mut State) {
    ui.spacing_mut().item_spacing.y = 2.0;
    ui.add(
        TextEdit::singleline(&mut st.query)
            .hint_text(t("settings.search"))
            .desired_width(f32::INFINITY)
            .min_size(egui::vec2(0.0, CONTROL_HEIGHT))
            .align(egui::Align2::LEFT_CENTER),
    );
    ui.add_space(6.0);
    let searching = !st.query.trim().is_empty();
    for c in Category::ALL {
        let selected = st.category == c && !searching;
        let dimmed = searching && !category_matches(c, &st.query);
        if category_row(ui, c.label(), selected, dimmed).clicked() {
            st.category = c;
            st.query.clear();
        }
    }
}

/// The settings window's id, for focusing it from the main window.
pub fn viewport_id() -> ViewportId {
    ViewportId::from_hash_of("settings")
}

/// What the footer asked for.
#[derive(Default)]
struct Outcome {
    apply: bool,
    done: bool,
}

/// Window content: footer, sidebar and page. `ok`/`cancel` are signalled through the outcome.
fn content(ui: &mut Ui, explorer: &Explorer, st: &mut State, out: &mut Outcome) {
    let dirty = st.pending() != *explorer.settings();
    let style = ui.style().clone();
    egui::Panel::bottom("settings-footer")
        .frame(egui::Frame::side_top_panel(&style).inner_margin(12))
        .show(ui, |ui| footer(ui, dirty, out));
    egui::Panel::left("settings-sidebar")
        .resizable(false)
        .exact_size(SIDEBAR_WIDTH)
        .frame(egui::Frame::side_top_panel(&style).inner_margin(12))
        .show(ui, |ui| sidebar(ui, st));
    let current_folder = explorer.active().path().display().to_string();
    let margin = egui::Margin {
        left: 16,
        right: 12,
        top: 12,
        bottom: 12,
    };
    egui::CentralPanel::default()
        .frame(egui::Frame::central_panel(&style).inner_margin(margin))
        .show(ui, |ui| page_area(ui, st, current_folder));
}

/// The footer buttons, right-aligned: OK, Cancel and Apply (enabled when the draft differs from the saved settings).
fn footer(ui: &mut Ui, dirty: bool, out: &mut Outcome) {
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        let min = egui::vec2(BUTTON_MIN_WIDTH, CONTROL_HEIGHT);
        if ui
            .add_enabled(dirty, egui::Button::new(t("button.apply")).min_size(min))
            .clicked()
        {
            out.apply = true;
        }
        if ui
            .add(egui::Button::new(t("button.cancel")).min_size(min))
            .clicked()
        {
            out.done = true;
        }
        let ok = egui::Button::new(RichText::new(t("button.ok")).color(theme::on_accent()))
            .fill(theme::accent())
            .min_size(min);
        if ui.add(ok).clicked() {
            out.apply = true;
            out.done = true;
        }
    });
}

/// The page with the form spacing; it scrolls unless it is the Keyboard page, whose list scrolls itself.
fn page_area(ui: &mut Ui, st: &mut State, current_folder: String) {
    let spacing = ui.spacing_mut();
    spacing.item_spacing = egui::vec2(8.0, ROW_GAP);
    spacing.interact_size.y = CONTROL_HEIGHT;
    spacing.icon_spacing = 6.0;
    if st.query.trim().is_empty() && st.category == Category::Keyboard {
        page(ui, st, current_folder);
        return;
    }
    ScrollArea::vertical()
        .id_salt("settings-page")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.set_max_width(ui.available_width() - 12.0);
            page(ui, st, current_folder);
        });
}

/// Draws the settings window. Returns true when it should be closed.
pub fn show(ctx: &Context, explorer: &mut Explorer, st: &mut State) -> bool {
    let builder = ViewportBuilder::default()
        .with_title(t("settings.title"))
        .with_inner_size([960.0, 640.0])
        .with_min_inner_size([740.0, 510.0]);
    let mut out = Outcome::default();
    ctx.show_viewport_immediate(viewport_id(), builder, |ui, class| {
        window(ui, class, explorer, st, &mut out);
    });
    if out.apply {
        let pending = st.pending();
        explorer.set_settings(pending);
    }
    if out.done {
        theme::apply(ctx, explorer.settings());
    }
    out.done
}

/// One frame of the settings viewport: keys, live preview, then the content.
fn window(
    ui: &mut Ui,
    class: ViewportClass,
    explorer: &Explorer,
    st: &mut State,
    out: &mut Outcome,
) {
    let ctx = ui.ctx().clone();
    st.embedded = class == ViewportClass::EmbeddedWindow;
    if st.embedded {
        ui.set_min_size(egui::vec2(900.0, 560.0));
    }
    handle_escape(&ctx, st, out);
    if ctx.input(|i| i.viewport().close_requested()) {
        out.done = true;
    }
    if look_changed(&st.draft, &st.previewed) {
        theme::apply(&ctx, &st.draft);
        st.previewed = st.draft.clone();
    }
    content(ui, explorer, st, out);
}

/// Esc first stops a recording or clears a non-empty search, else cancels.
fn handle_escape(ctx: &Context, st: &mut State, out: &mut Outcome) {
    let esc_free = st.recording.is_none() && !egui::Popup::is_any_open(ctx);
    if !esc_free || !ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
        return;
    }
    if st.query.is_empty() {
        out.done = true;
    } else {
        st.query.clear();
    }
}

/// Brings the settings window to the front.
pub fn focus(ctx: &Context) {
    ctx.send_viewport_cmd_to(viewport_id(), ViewportCommand::Focus);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_matches_label_or_category_case_insensitively() {
        assert_eq!(search("").len(), ROWS.len());
        assert_eq!(search("THEME").len(), 1);
        assert_eq!(search("fonts").len(), 3);
        assert!(search("zzz").is_empty());
        assert!(search_commands("zzz").is_empty());
        assert!(category_matches(Category::Keyboard, "rename"));
        assert!(!category_matches(Category::Fonts, "rename"));
    }

    #[test]
    fn row_labels_are_catalog_keys() {
        for r in ROWS {
            assert_ne!(t(r.label), r.label, "missing text for {}", r.label);
            assert_ne!(t(r.group), r.group, "missing text for {}", r.group);
        }
        for c in Category::ALL {
            assert!(!c.label().starts_with("settings."), "{c:?}");
        }
    }

    #[test]
    fn command_groups_have_text() {
        for d in COMMANDS {
            assert_ne!(t(d.group), d.group, "missing text for {}", d.group);
        }
    }

    #[test]
    fn commands_are_ordered_by_group() {
        let mut seen: Vec<&str> = Vec::new();
        for d in COMMANDS {
            if seen.last() != Some(&d.group) {
                assert!(!seen.contains(&d.group), "{} split", d.group);
                seen.push(d.group);
            }
        }
    }

    #[test]
    fn groups_are_contiguous_within_a_category() {
        // `form` starts a new box whenever the group changes, so a group split in two would show twice.
        let mut seen = Vec::new();
        for (i, r) in ROWS.iter().enumerate() {
            if i == 0 || ROWS[i - 1].category != r.category || ROWS[i - 1].group != r.group {
                assert!(!seen.contains(&(r.category, r.group)), "{} split", r.group);
                seen.push((r.category, r.group));
            }
        }
    }

    #[test]
    fn every_category_has_rows() {
        // The Keyboard page is generated from the command table, not from ROWS.
        for c in Category::ALL
            .into_iter()
            .filter(|c| *c != Category::Keyboard)
        {
            assert!(ROWS.iter().any(|r| r.category == c));
        }
    }
}
