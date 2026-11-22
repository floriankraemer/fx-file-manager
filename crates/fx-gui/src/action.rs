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
    /// Show the search bar and focus it.
    OpenSearch,
    /// The bar's text changed: (re)start the search, or clear it when blank.
    StartSearch(String),
    /// Stop walking and keep the results.
    StopSearch,
    /// Drop the results and hide the bar.
    CloseSearch,
    /// Esc: stop a running search, otherwise close it.
    EscapeSearch,
    /// Result row of the active tab's search.
    OpenSearchHit(usize),
    RevealSearchHit(usize),
    CopySearchHitPath(usize),
    SetSort(SortSpec),
    SetShowHidden(bool),
    SetDensity(Density),
}
