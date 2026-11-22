//! fx-app: UI-agnostic view-model for a tabbed file explorer. No UI dependencies.

mod columns;
mod explorer;
mod format;
pub mod i18n;
mod ops;
mod place_group;
mod places;
mod settings;
mod state;
mod tab;
mod transfers;
mod tree;
mod view;
#[cfg(windows)]
mod wsl;

pub use columns::ListColumn;
pub use explorer::{Explorer, REVALIDATE_INTERVAL, WindowId, open_with_system};
pub use format::{
    AgeBucket, age, extension, format_millis, format_number, format_size, format_time,
    set_date_format, stem_chars,
};
pub use i18n::{Language, t, tn, tr};
pub use ops::{EntryDetails, NameError, Notice, NoticeKind, validate_name};
pub use place_group::PlaceGroup;
pub use places::{Place, bookmark_places, breadcrumbs, places};
pub use settings::{
    ConflictChoice, DEFAULT_THEME, DateFormat, Density, DragModifier, FolderOrder, GlobScopeChoice,
    MAX_FONT_SIZE, MIN_FONT_SIZE, MIN_RESULT_CAP, SearchSettings, Settings, start_dir,
};
pub use state::{Bookmark, State, WindowState};
pub use tab::{LoadState, Tab};
pub use transfers::{TransferInfo, TransferState};
pub use tree::{Children, FolderTree, TreeNode};
pub use view::{DirView, SortColumn, SortSpec};
