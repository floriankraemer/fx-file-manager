//! The columns of the file list. Which are shown, their order and widths live in `Settings`.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ListColumn {
    Name,
    Ext,
    Size,
    Modified,
    Age,
}

impl ListColumn {
    pub const ALL: [ListColumn; 5] = [
        ListColumn::Name,
        ListColumn::Ext,
        ListColumn::Size,
        ListColumn::Modified,
        ListColumn::Age,
    ];

    /// In the current UI language.
    pub fn label(self) -> &'static str {
        crate::t(match self {
            Self::Name => "col.name",
            Self::Ext => "col.ext",
            Self::Size => "col.size",
            Self::Modified => "col.modified",
            Self::Age => "col.age",
        })
    }

    /// Header text; the extension column abbreviates to fit.
    pub fn header(self) -> &'static str {
        match self {
            Self::Ext => crate::t("col_short.ext"),
            _ => self.label(),
        }
    }

    /// Name is the only column the user cannot hide: a listing needs something to name its rows.
    pub fn hideable(self) -> bool {
        self != Self::Name
    }

    pub fn default_width(self) -> f32 {
        match self {
            Self::Name => 260.0,
            Self::Ext => 60.0,
            Self::Size => 80.0,
            Self::Modified => 140.0,
            Self::Age => 56.0,
        }
    }

    pub fn min_width(self) -> f32 {
        match self {
            Self::Name => 160.0,
            Self::Ext => 30.0,
            Self::Size => 50.0,
            Self::Modified => 80.0,
            Self::Age => 40.0,
        }
    }
}
