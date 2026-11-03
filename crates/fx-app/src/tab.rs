//! One explorer tab: location, history, load state, and the visible row order.

use crate::view::{DirView, SortColumn, SortSpec, build_order};
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
    selected: Option<usize>,
    /// Keeps the selection on the same file across re-sorts, filters and refreshes.
    selected_name: Option<OsString>,
    sort: SortSpec,
    show_hidden: bool,
    filter: String,
    back: Vec<PathBuf>,
    forward: Vec<PathBuf>,
}

impl Tab {
    pub(crate) fn new(id: u64, path: PathBuf) -> Self {
        Self {
            id,
            path,
            state: LoadState::Loading,
            generation: 0,
            order: Vec::new(),
            selected: None,
            selected_name: None,
            sort: SortSpec::default(),
            show_hidden: false,
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

    /// Index into `order()`.
    pub fn selected(&self) -> Option<usize> {
        self.selected
    }

    /// Out-of-range rows clear the selection.
    pub fn select(&mut self, row: Option<usize>) {
        let row = row.filter(|&r| r < self.order.len());
        self.selected_name = row.and_then(|r| self.name_at(r)).map(OsStr::to_os_string);
        self.selected = row;
    }

    /// Moves the selection by `delta` rows, clamped to the visible rows.
    /// With no selection, a positive delta starts at the top and a negative one at the bottom.
    pub fn move_selection(&mut self, delta: isize) {
        let Some(last) = self.order.len().checked_sub(1) else {
            return self.select(None);
        };
        let current = self
            .selected
            .map_or(if delta < 0 { last as isize + 1 } else { -1 }, |row| {
                row as isize
            });
        let row = (current + delta).clamp(0, last as isize) as usize;
        self.select(Some(row));
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

    /// Marks the tab as loading and returns the generation the new result must carry.
    /// The previous selection's name is kept, so a refresh can restore it.
    pub(crate) fn begin_load(&mut self) -> u64 {
        self.generation += 1;
        self.state = LoadState::Loading;
        self.order.clear();
        self.selected = None;
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

    fn clear_selection(&mut self) {
        self.selected = None;
        self.selected_name = None;
    }

    fn rebuild_order(&mut self) {
        let previous = self.selected;
        self.order = match &self.state {
            LoadState::Loaded(view) => {
                build_order(&view.listing, self.show_hidden, &self.filter, self.sort)
            }
            LoadState::Loading | LoadState::Failed(_) => Vec::new(),
        };
        // Prefer the same file; otherwise keep the old row position, clamped to the new list.
        let row = self.row_of_selected_name().or_else(|| {
            let last = self.order.len().checked_sub(1)?;
            previous.map(|row| row.min(last))
        });
        self.select(row);
    }

    fn row_of_selected_name(&self) -> Option<usize> {
        let name = self.selected_name.as_deref()?;
        self.order
            .iter()
            .position(|&index| self.name_of(index) == Some(name))
    }

    fn name_at(&self, row: usize) -> Option<&OsStr> {
        self.name_of(*self.order.get(row)?)
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
