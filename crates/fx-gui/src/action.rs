//! User intents raised by views (command bar, context menus, shortcuts) and executed by `App`.

use fx_app::{Density, SortSpec};

pub enum Action {
    NewFolder,
    Rename,
    Delete {
        permanent: bool,
    },
    /// Delete these paths (not rows), e.g. folders picked in the tree.
    DeletePaths(Vec<std::path::PathBuf>),
    SelectAll,
    Properties,
    NewWindow,
    OpenSettings,
    OpenCommandPalette,
    OpenAbout,
    Exit,
    /// Ask for a new label for the bookmark at this index.
    RenameBookmark(usize),
    Refresh,
    /// Row of the active tab.
    Open(usize),
    OpenInNewTab(usize),
    /// Let the user pick the app for the file at this row of the active tab.
    OpenWith(usize),
    CopyPath,
    Copy,
    Cut,
    /// Paste into the active folder.
    Paste,
    /// Paste into the folder at this row of the active tab.
    PasteInto(usize),
    /// Bookmarks the folder at this row, or removes its bookmark.
    ToggleBookmark(usize),
    Search(crate::views::search::SearchAction),
    SetSort(SortSpec),
    SetShowHidden(bool),
    SetDensity(Density),
}
