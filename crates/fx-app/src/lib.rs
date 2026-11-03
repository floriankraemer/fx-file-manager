//! fx-app: UI-agnostic view-model for a tabbed file explorer. No UI dependencies.

mod explorer;
mod format;
mod places;
mod tab;
mod view;

pub use explorer::Explorer;
pub use format::{AgeBucket, age, extension, format_millis, format_size, format_time};
pub use places::{Place, PlaceGroup, breadcrumbs, places};
pub use tab::{LoadState, Tab};
pub use view::{DirView, SortColumn, SortSpec};
