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
    OpenSettings,
    Refresh,
    /// Row of the active tab.
    Open(usize),
    OpenInNewTab(usize),
    CopyPath,
    SetSort(SortSpec),
    SetShowHidden(bool),
    SetDensity(Density),
}
