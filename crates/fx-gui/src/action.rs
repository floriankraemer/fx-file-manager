//! User intents raised by views (command bar, context menus, shortcuts) and executed by `App`.

use fx_app::{Density, SortSpec};

pub enum Action {
    NewFolder,
    Rename,
    Delete {
        permanent: bool,
    },
    SelectAll,
    Properties,
    NewWindow,
    OpenSettings,
    OpenAbout,
    Exit,
    Refresh,
    /// Row of the active tab.
    Open(usize),
    OpenInNewTab(usize),
    /// Let the user pick the app for the file at this row of the active tab.
    #[cfg(windows)]
    OpenWith(usize),
    CopyPath,
    Copy,
    Cut,
    /// Paste into the active folder.
    Paste,
    /// Paste into the folder at this row of the active tab.
    PasteInto(usize),
    SetSort(SortSpec),
    SetShowHidden(bool),
    SetDensity(Density),
}
