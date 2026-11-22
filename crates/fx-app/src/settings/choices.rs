use fx_core::{ConflictPolicy, GlobScope};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Density {
    #[default]
    Compact,
    Comfortable,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FolderOrder {
    #[default]
    Top,
    Mixed,
    Bottom,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DateFormat {
    #[default]
    Locale,
    Iso,
}

/// A modifier key that changes what a drop does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DragModifier {
    Ctrl,
    Shift,
    /// Option on macOS.
    Alt,
}

impl DragModifier {
    pub const ALL: [DragModifier; 3] = [DragModifier::Ctrl, DragModifier::Shift, DragModifier::Alt];

    /// In the current UI language.
    pub fn label(self) -> &'static str {
        crate::t(match self {
            DragModifier::Ctrl => "drag_modifier.ctrl",
            DragModifier::Shift => "drag_modifier.shift",
            DragModifier::Alt => "drag_modifier.alt",
        })
    }
}

/// The platform's file manager convention: Option in Finder, Ctrl elsewhere.
impl Default for DragModifier {
    fn default() -> Self {
        if cfg!(target_os = "macos") {
            DragModifier::Alt
        } else {
            DragModifier::Ctrl
        }
    }
}

/// Persisted form of [`ConflictPolicy`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ConflictChoice {
    #[default]
    Ask,
    Skip,
    Overwrite,
    KeepBoth,
    OverwriteIfNewer,
    SkipIdentical,
}

impl ConflictChoice {
    pub const ALL: [ConflictChoice; 6] = [
        ConflictChoice::Ask,
        ConflictChoice::Skip,
        ConflictChoice::Overwrite,
        ConflictChoice::KeepBoth,
        ConflictChoice::OverwriteIfNewer,
        ConflictChoice::SkipIdentical,
    ];

    /// In the current UI language.
    pub fn label(self) -> &'static str {
        crate::t(match self {
            ConflictChoice::Ask => "conflict_policy.ask",
            ConflictChoice::Skip => "conflict_policy.skip",
            ConflictChoice::Overwrite => "conflict_policy.overwrite",
            ConflictChoice::KeepBoth => "conflict_policy.keep_both",
            ConflictChoice::OverwriteIfNewer => "conflict_policy.overwrite_if_newer",
            ConflictChoice::SkipIdentical => "conflict_policy.skip_identical",
        })
    }
}

impl From<ConflictChoice> for ConflictPolicy {
    fn from(c: ConflictChoice) -> Self {
        match c {
            ConflictChoice::Ask => ConflictPolicy::Ask,
            ConflictChoice::Skip => ConflictPolicy::Skip,
            ConflictChoice::Overwrite => ConflictPolicy::Overwrite,
            ConflictChoice::KeepBoth => ConflictPolicy::KeepBoth,
            ConflictChoice::OverwriteIfNewer => ConflictPolicy::OverwriteIfNewer,
            ConflictChoice::SkipIdentical => ConflictPolicy::SkipIdentical,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GlobScopeChoice {
    #[default]
    Name,
    RelativePath,
}

impl GlobScopeChoice {
    pub const ALL: [GlobScopeChoice; 2] = [GlobScopeChoice::Name, GlobScopeChoice::RelativePath];

    /// In the current UI language.
    pub fn label(self) -> &'static str {
        crate::t(match self {
            GlobScopeChoice::Name => "search.scope.name",
            GlobScopeChoice::RelativePath => "search.scope.relative_path",
        })
    }
}

impl From<GlobScopeChoice> for GlobScope {
    fn from(c: GlobScopeChoice) -> Self {
        match c {
            GlobScopeChoice::Name => GlobScope::Name,
            GlobScopeChoice::RelativePath => GlobScope::RelativePath,
        }
    }
}
