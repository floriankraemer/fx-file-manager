use super::Settings;
use super::sidebar::move_to_slot_of;
use crate::columns::ListColumn;
use serde::Deserialize;

impl Settings {
    pub fn column_shown(&self, column: ListColumn) -> bool {
        self.list_columns.contains(&column)
    }

    /// Hiding `Name` is ignored. A column shown again goes last.
    pub fn set_column_shown(&mut self, column: ListColumn, shown: bool) {
        if shown == self.column_shown(column) || (!shown && !column.hideable()) {
            return;
        }
        if !shown {
            self.list_columns.retain(|c| *c != column);
            return;
        }
        self.list_columns.push(column);
    }

    /// Moves `column` to the slot `target` occupies. Both must be shown.
    pub fn move_column(&mut self, column: ListColumn, target: ListColumn) {
        if self.column_shown(column) && self.column_shown(target) {
            move_to_slot_of(&mut self.list_columns, column, target);
        }
    }

    /// The saved width of `column`, never below its minimum.
    pub fn column_width(&self, column: ListColumn) -> f32 {
        let saved = self.list_column_widths.get(&column).copied();
        saved
            .filter(|w| w.is_finite())
            .unwrap_or(column.default_width())
            .max(column.min_width())
    }

    /// Drops repeats and makes sure `Name` is present, as a hand-edited file may break either.
    pub(super) fn normalize_columns(&mut self) {
        let mut seen = Vec::new();
        for c in &self.list_columns {
            if !seen.contains(c) {
                seen.push(*c);
            }
        }
        if !seen.contains(&ListColumn::Name) {
            seen.insert(0, ListColumn::Name);
        }
        self.list_columns = seen;
    }
}

/// The two flags that preceded `list_columns`.
#[derive(Deserialize)]
pub(super) struct LegacyColumns {
    list_columns: Option<serde::de::IgnoredAny>,
    show_ext_column: Option<bool>,
    show_age_column: Option<bool>,
}

impl LegacyColumns {
    /// The column list the old flags describe; `None` when the file already has `list_columns` or sets no flag.
    pub(super) fn migrate(self) -> Option<Vec<ListColumn>> {
        if self.list_columns.is_some()
            || (self.show_ext_column, self.show_age_column) == (None, None)
        {
            return None;
        }
        let mut columns = vec![ListColumn::Name];
        let optional = [
            (ListColumn::Ext, self.show_ext_column),
            (ListColumn::Size, Some(true)),
            (ListColumn::Modified, Some(true)),
            (ListColumn::Age, self.show_age_column),
        ];
        columns.extend(
            optional
                .into_iter()
                .filter_map(|(c, shown)| shown.unwrap_or(true).then_some(c)),
        );
        Some(columns)
    }
}
