//! fx-app: UI-agnostic view-model for a tabbed file explorer. No UI dependencies.

mod explorer;
mod format;
pub mod i18n;
mod ops;
mod places;
mod settings;
mod tab;
mod transfers;
mod tree;
mod view;

pub use explorer::{Explorer, REVALIDATE_INTERVAL, open_with_system};
pub use format::{AgeBucket, age, extension, format_millis, format_size, format_time, stem_chars};
pub use i18n::{Language, t, tn, tr};
pub use ops::{EntryDetails, NameError, Notice, NoticeKind, validate_name};
pub use places::{Place, PlaceGroup, breadcrumbs, places};
pub use settings::{
    ConflictChoice, DEFAULT_THEME, Density, FolderOrder, MAX_FONT_SIZE, MIN_FONT_SIZE, Settings,
    start_dir,
};
pub use tab::{LoadState, Tab};
pub use transfers::{TransferInfo, TransferState};
pub use tree::{Children, FolderTree, TreeNode};
pub use view::{DirView, SortColumn, SortSpec};
