//! fx-app: UI-agnostic view-model for a tabbed file explorer. No UI dependencies.

mod explorer;
mod format;
mod ops;
mod places;
mod settings;
mod tab;
mod tree;
mod view;

pub use explorer::Explorer;
pub use format::{AgeBucket, age, extension, format_millis, format_size, format_time, stem_chars};
pub use ops::{EntryDetails, NameError, Notice, NoticeKind, validate_name};
pub use places::{Place, PlaceGroup, breadcrumbs, places};
pub use settings::{Density, Settings, ThemeChoice, start_dir};
pub use tab::{LoadState, Tab};
pub use tree::{Children, FolderTree, TreeNode};
pub use view::{DirView, SortColumn, SortSpec};
