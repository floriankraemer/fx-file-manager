use super::MIN_RESULT_CAP;
use super::choices::GlobScopeChoice;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SearchSettings {
    /// Most matches a search delivers before it stops; at least `MIN_RESULT_CAP` after loading.
    pub result_cap: usize,
    /// What a glob query is matched against.
    pub glob_scope: GlobScopeChoice,
    /// Match case: `a` no longer finds `A`.
    pub case_sensitive: bool,
    /// Show a notice before searching from a filesystem root.
    pub warn_root: bool,
}

impl Default for SearchSettings {
    fn default() -> Self {
        Self {
            result_cap: 100_000,
            glob_scope: GlobScopeChoice::default(),
            case_sensitive: false,
            warn_root: true,
        }
    }
}

impl SearchSettings {
    pub(super) fn normalize(&mut self) {
        self.result_cap = self.result_cap.max(MIN_RESULT_CAP);
    }
}
