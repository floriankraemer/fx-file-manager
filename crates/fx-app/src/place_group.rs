use serde::{Deserialize, Serialize};

/// Sidebar section a place belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PlaceGroup {
    /// Home and its standard folders.
    Places,
    /// Folders the user pinned; they stay listed when the folder is missing.
    Bookmarks,
    /// Filesystem roots and mounts.
    Drives,
    /// Installed WSL distributions.
    Wsl,
}

impl PlaceGroup {
    pub const ALL: [PlaceGroup; 4] = [
        PlaceGroup::Places,
        PlaceGroup::Bookmarks,
        PlaceGroup::Drives,
        PlaceGroup::Wsl,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Self::Places => crate::t("places.places"),
            Self::Bookmarks => crate::t("places.bookmarks"),
            Self::Drives => crate::t("places.drives"),
            Self::Wsl => crate::t("places.wsl"),
        }
    }
}
