//! One explorer tab: location, history, load state, the visible row order, and the selection.

use crate::view::{DirView, SortColumn, SortSpec, build_order};
use std::collections::BTreeSet;
use std::ffi::{OsStr, OsString};
use std::mem;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Clone, Debug)]
pub enum LoadState {
    Loading,
    Loaded(Arc<DirView>),
    Failed(String),
}

#[derive(Debug)]
pub struct Tab {
    id: u64,
    path: PathBuf,
    state: LoadState,
    /// Bumped per load; results carrying an older value are stale and dropped.
    generation: u64,
    order: Vec<u32>,
    /// Selected rows (indices into `order`), sorted.
    rows: BTreeSet<usize>,
    /// Focus row; also the anchor for shift-click ranges.
    focus: Option<usize>,
    /// Names behind `rows`. Kept across loads so a refresh, resort or filter restores the selection by name.
    selected_names: BTreeSet<OsString>,
    focus_name: Option<OsString>,
    sort: SortSpec,
    show_hidden: bool,
    filter: String,
    back: Vec<PathBuf>,
    forward: Vec<PathBuf>,
}

impl Tab {
    pub(crate) fn new(id: u64, path: PathBuf, show_hidden: bool) -> Self {
        Self {
            id,
            path,
            state: LoadState::Loading,
            generation: 0,
            order: Vec::new(),
            rows: BTreeSet::new(),
            focus: None,
            selected_names: BTreeSet::new(),
            focus_name: None,
            sort: SortSpec::default(),
            show_hidden,
            filter: String::new(),
            back: Vec::new(),
            forward: Vec::new(),
        }
    }

    pub fn id(&self) -> u64 {
        self.id
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Last path component, or the full path for a root such as `/` or `C:\`.
    pub fn title(&self) -> String {
        self.path.file_name().map_or_else(
            || self.path.display().to_string(),
            |name| name.to_string_lossy().into_owned(),
        )
    }

    pub fn state(&self) -> &LoadState {
        &self.state
    }

    /// Visible rows: indices into the loaded listing's entries.
    pub fn order(&self) -> &[u32] {
        &self.order
    }

    /// The focus row (index into `order()`), which is also the shift-click anchor. It may be unselected.
    pub fn selected(&self) -> Option<usize> {
        self.focus
    }

    /// Selected rows (indices into `order()`), sorted.
    pub fn selection(&self) -> Vec<usize> {
        self.rows.iter().copied().collect()
    }

    pub fn is_selected(&self, row: usize) -> bool {
        self.rows.contains(&row)
    }

    /// Replaces the selection with one row. Out-of-range rows clear the selection.
    pub fn select(&mut self, row: Option<usize>) {
        let row = row.filter(|&r| r < self.order.len());
        self.set_selection(row.into_iter().collect(), row);
    }

    /// Windows-style click: plain replaces the selection, ctrl toggles one row, shift selects the range from the anchor.
    /// Shift with ctrl adds the range to the existing selection. Shift without an anchor acts as a plain click.
    /// Out-of-range rows are ignored.
    pub fn click(&mut self, row: usize, ctrl: bool, shift: bool) {
        if row >= self.order.len() {
            return;
        }
        match (shift, self.focus) {
            (true, Some(anchor)) => {
                let mut rows = if ctrl {
                    self.rows.clone()
                } else {
                    BTreeSet::new()
                };
                rows.extend(anchor.min(row)..=anchor.max(row));
                self.set_selection(rows, Some(anchor));
            }
            _ if ctrl => {
                let mut rows = self.rows.clone();
                if !rows.remove(&row) {
                    rows.insert(row);
                }
                self.set_selection(rows, Some(row));
            }
            _ => self.set_selection(BTreeSet::from([row]), Some(row)),
        }
    }

    /// Selects every visible row. The focus stays where it is, or moves to the top if there was none.
    pub fn select_all(&mut self) {
        let focus = self.focus.or_else(|| (!self.order.is_empty()).then_some(0));
        self.set_selection((0..self.order.len()).collect(), focus);
    }

    /// Moves the focus by `delta` rows and selects only that row, clamped to the visible rows.
    /// With no selection, a positive delta starts at the top and a negative one at the bottom.
    pub fn move_selection(&mut self, delta: isize) {
        let Some(last) = self.order.len().checked_sub(1) else {
            return self.select(None);
        };
        let current = self
            .focus
            .map_or(if delta < 0 { last as isize + 1 } else { -1 }, |row| {
                row as isize
            });
        let row = (current + delta).clamp(0, last as isize) as usize;
        self.select(Some(row));
    }

    /// Selects the entry called `name` once the next load of this tab completes.
    pub(crate) fn select_after_load(&mut self, name: OsString) {
        self.selected_names = BTreeSet::from([name.clone()]);
        self.focus_name = Some(name);
    }

    pub fn sort(&self) -> SortSpec {
        self.sort
    }

    /// Selecting the active column flips its direction; a new column starts ascending.
    pub fn set_sort_column(&mut self, column: SortColumn) {
        self.sort = if self.sort.column == column {
            SortSpec {
                ascending: !self.sort.ascending,
                ..self.sort
            }
        } else {
            SortSpec {
                column,
                ascending: true,
            }
        };
        self.rebuild_order();
    }

    /// Sets column and direction explicitly, e.g. from a sort menu.
    pub fn set_sort(&mut self, sort: SortSpec) {
        self.sort = sort;
        self.rebuild_order();
    }

    pub fn show_hidden(&self) -> bool {
        self.show_hidden
    }

    pub fn set_show_hidden(&mut self, show: bool) {
        self.show_hidden = show;
        self.rebuild_order();
    }

    pub fn filter(&self) -> &str {
        &self.filter
    }

    /// Case-insensitive substring match on the entry name; empty shows everything.
    pub fn set_filter(&mut self, filter: &str) {
        filter.clone_into(&mut self.filter);
        self.rebuild_order();
    }

    pub fn can_back(&self) -> bool {
        !self.back.is_empty()
    }

    pub fn can_forward(&self) -> bool {
        !self.forward.is_empty()
    }

    pub fn can_up(&self) -> bool {
        self.path.parent().is_some()
    }

    /// Pushes the current path onto history and clears forward history.
    pub(crate) fn navigate_to(&mut self, path: PathBuf) {
        if path == self.path {
            return;
        }
        let previous = mem::replace(&mut self.path, path);
        self.back.push(previous);
        self.forward.clear();
        self.clear_selection();
    }

    pub(crate) fn go_back(&mut self) -> bool {
        let Some(previous) = self.back.pop() else {
            return false;
        };
        let current = mem::replace(&mut self.path, previous);
        self.forward.push(current);
        self.clear_selection();
        true
    }

    pub(crate) fn go_forward(&mut self) -> bool {
        let Some(next) = self.forward.pop() else {
            return false;
        };
        let current = mem::replace(&mut self.path, next);
        self.back.push(current);
        self.clear_selection();
        true
    }

    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }

    /// Marks the tab as loading and returns the generation the new result must carry.
    /// The selected names are kept, so a refresh can restore the selection.
    pub(crate) fn begin_load(&mut self) -> u64 {
        self.generation += 1;
        self.state = LoadState::Loading;
        self.order.clear();
        self.rows.clear();
        self.focus = None;
        self.generation
    }

    /// Applies a finished load. Returns false (and changes nothing) for a stale generation.
    pub(crate) fn apply_load(&mut self, generation: u64, result: Result<DirView, String>) -> bool {
        if generation != self.generation {
            return false;
        }
        self.state = match result {
            Ok(view) => LoadState::Loaded(Arc::new(view)),
            Err(message) => LoadState::Failed(message),
        };
        self.rebuild_order();
        true
    }

    /// Entry name of a visible row, if the listing is loaded.
    pub(crate) fn name_at(&self, row: usize) -> Option<&OsStr> {
        self.name_of(*self.order.get(row)?)
    }

    fn clear_selection(&mut self) {
        self.rows.clear();
        self.focus = None;
        self.selected_names.clear();
        self.focus_name = None;
    }

    /// Sets the selection and derives the remembered names from it.
    fn set_selection(&mut self, rows: BTreeSet<usize>, focus: Option<usize>) {
        self.selected_names = rows
            .iter()
            .filter_map(|&row| self.name_at(row))
            .map(OsStr::to_os_string)
            .collect();
        self.focus_name = focus
            .and_then(|row| self.name_at(row))
            .map(OsStr::to_os_string);
        self.rows = rows;
        self.focus = focus;
    }

    /// Rebuilds the visible order and maps the remembered names onto it.
    /// Names that are no longer visible (deleted, hidden by the filter) drop out of the selection.
    fn rebuild_order(&mut self) {
        let previous_focus = self.focus;
        self.order = match &self.state {
            LoadState::Loaded(view) => {
                build_order(&view.listing, self.show_hidden, &self.filter, self.sort)
            }
            LoadState::Loading | LoadState::Failed(_) => Vec::new(),
        };
        let rows = (0..self.order.len())
            .filter(|&row| {
                self.name_at(row)
                    .is_some_and(|name| self.selected_names.contains(name))
            })
            .collect();
        // Prefer the same file as focus; otherwise keep the old row position, clamped to the new list.
        let focus = self.row_of_focus_name().or_else(|| {
            let last = self.order.len().checked_sub(1)?;
            previous_focus.map(|row| row.min(last))
        });
        self.set_selection(rows, focus);
    }

    fn row_of_focus_name(&self) -> Option<usize> {
        let name = self.focus_name.as_deref()?;
        (0..self.order.len()).find(|&row| self.name_at(row) == Some(name))
    }

    fn name_of(&self, index: u32) -> Option<&OsStr> {
        match &self.state {
            LoadState::Loaded(view) => {
                Some(view.listing.name(&view.listing.entries[index as usize]))
            }
            LoadState::Loading | LoadState::Failed(_) => None,
        }
    }
}
