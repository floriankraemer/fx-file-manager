//! Use case: streamed recursive name search on top of the parallel walk.

use crate::list::ListError;
use crate::listing::{EntryKind, Listing};
use crate::walk::{WalkOptions, Walked, walk_cancellable};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

/// What a glob pattern is matched against. Substring queries always match the name.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GlobScope {
    #[default]
    Name,
    /// Path relative to the search root, with `/` as separator on every OS.
    RelativePath,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Pattern {
    Substring(Vec<u8>),
    Glob(Vec<u8>),
}

/// Case-insensitive (ASCII) query: a substring, or a glob when it contains `*` or `?`.
/// An empty query matches everything.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchQuery {
    pattern: Pattern,
    scope: GlobScope,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    pub path: PathBuf,
    pub kind: EntryKind,
}

impl SearchQuery {
    pub fn new(text: &str, scope: GlobScope) -> Self {
        let lowered = text.as_bytes().to_ascii_lowercase();
        let is_glob = lowered.iter().any(|&b| b == b'*' || b == b'?');
        let pattern = if is_glob {
            Pattern::Glob(lowered)
        } else {
            Pattern::Substring(lowered)
        };
        Self { pattern, scope }
    }

    fn matches_name(&self, name: &[u8]) -> bool {
        match &self.pattern {
            Pattern::Substring(needle) => contains_ignore_case(name, needle),
            Pattern::Glob(glob) => glob_match(glob, name),
        }
    }

    fn matches_relative(&self) -> bool {
        self.scope == GlobScope::RelativePath && matches!(self.pattern, Pattern::Glob(_))
    }
}

/// `needle` must already be ASCII-lowercase.
fn contains_ignore_case(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() {
        return true;
    }
    haystack.windows(needle.len()).any(|w| {
        w.iter()
            .zip(needle)
            .all(|(h, n)| h.to_ascii_lowercase() == *n)
    })
}

/// Length of the UTF-8 sequence led by `b`; 1 for invalid lead bytes so matching never stalls.
fn char_len(b: u8) -> usize {
    match b {
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        0xF0..=0xF7 => 4,
        _ => 1,
    }
}

/// Anchored glob match: `*` is any run, `?` one UTF-8 character. `glob` must be ASCII-lowercase.
/// Iterative with single-star backtracking: O(glob * text) worst case, no allocation.
fn glob_match(glob: &[u8], text: &[u8]) -> bool {
    let (mut g, mut t) = (0, 0);
    let mut resume: Option<(usize, usize)> = None;
    while t < text.len() {
        match glob.get(g) {
            Some(b'*') => {
                resume = Some((g + 1, t));
                g += 1;
            }
            Some(b'?') => {
                g += 1;
                t = (t + char_len(text[t])).min(text.len());
            }
            Some(&c) if c == text[t].to_ascii_lowercase() => {
                g += 1;
                t += 1;
            }
            _ => {
                let Some((rg, rt)) = resume else {
                    return false;
                };
                let next = (rt + char_len(text[rt])).min(text.len());
                resume = Some((rg, next));
                g = rg;
                t = next;
            }
        }
    }
    glob[g..].iter().all(|&c| c == b'*')
}

/// Streams matches below `root` to `on_hits`, one non-empty batch per directory, from many threads.
/// Setting `cancel` stops the walk; the returned stats then cover the part that was visited.
pub fn search<F>(
    root: &Path,
    query: &SearchQuery,
    opts: &WalkOptions,
    cancel: &AtomicBool,
    on_hits: F,
) -> Result<Walked, ListError>
where
    F: Fn(Vec<Hit>) + Sync,
{
    walk_cancellable(root, opts, cancel, |dir, listing| {
        let hits = if query.matches_relative() {
            relative_hits(root, dir, listing, query)
        } else {
            name_hits(dir, listing, query)
        };
        if !hits.is_empty() {
            on_hits(hits);
        }
    })
}

fn name_hits(dir: &Path, listing: &Listing, query: &SearchQuery) -> Vec<Hit> {
    listing
        .entries
        .iter()
        .filter(|e| query.matches_name(listing.name_bytes(e)))
        .map(|e| hit(dir, listing, e))
        .collect()
}

fn relative_hits(root: &Path, dir: &Path, listing: &Listing, query: &SearchQuery) -> Vec<Hit> {
    let mut haystack = relative_prefix(root, dir);
    let prefix_len = haystack.len();
    let mut hits = Vec::new();
    for e in &listing.entries {
        haystack.truncate(prefix_len);
        haystack.extend_from_slice(listing.name_bytes(e));
        if query.matches_name(&haystack) {
            hits.push(hit(dir, listing, e));
        }
    }
    hits
}

/// `dir` relative to `root` with a trailing `/`, or empty for the root itself.
fn relative_prefix(root: &Path, dir: &Path) -> Vec<u8> {
    let rel = dir.strip_prefix(root).unwrap_or(dir);
    let mut bytes = rel.as_os_str().as_encoded_bytes().to_vec();
    if cfg!(windows) {
        bytes
            .iter_mut()
            .filter(|b| **b == b'\\')
            .for_each(|b| *b = b'/');
    }
    if !bytes.is_empty() {
        bytes.push(b'/');
    }
    bytes
}

fn hit(dir: &Path, listing: &Listing, e: &crate::listing::Entry) -> Hit {
    Hit {
        path: dir.join(listing.name(e)),
        kind: e.kind,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(text: &str) -> SearchQuery {
        SearchQuery::new(text, GlobScope::Name)
    }

    #[test]
    fn substring_ignores_ascii_case() {
        assert!(q("read").matches_name(b"README.md"));
        assert!(q("ME.m").matches_name(b"readme.md"));
        assert!(!q("readme").matches_name(b"read.md"));
    }

    #[test]
    fn empty_query_matches_everything() {
        assert!(q("").matches_name(b"anything"));
        assert!(q("").matches_name(b""));
    }

    #[test]
    fn glob_star_and_question_mark() {
        assert!(q("*.rs").matches_name(b"main.RS"));
        assert!(!q("*.rs").matches_name(b"main.rs.bak"));
        assert!(q("a?c").matches_name(b"abc"));
        assert!(!q("a?c").matches_name(b"ac"));
        assert!(!q("a?c").matches_name(b"abbc"));
        assert!(q("a*b*c").matches_name(b"aXXbYYc"));
        assert!(q("*").matches_name(b""));
        assert!(!q("*.rs").matches_name(b"readme.md"));
    }

    #[test]
    fn question_mark_consumes_a_whole_utf8_character() {
        assert!(q("h?llo").matches_name("héllo".as_bytes()));
        assert!(q("*.txt").matches_name("日本語.txt".as_bytes()));
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_names_still_match() {
        let name = b"caf\xE9 notes.TXT";
        assert!(q("notes").matches_name(name));
        assert!(q("*.txt").matches_name(name));
        assert!(q("caf?*").matches_name(name));
    }

    #[test]
    fn relative_scope_applies_to_globs_only() {
        let glob = SearchQuery::new("src/*.rs", GlobScope::RelativePath);
        assert!(glob.matches_relative());
        assert!(!SearchQuery::new("src", GlobScope::RelativePath).matches_relative());
        assert!(glob.matches_name(b"src/main.rs"));
    }

    #[test]
    fn relative_prefix_is_empty_at_root_and_slash_terminated_below() {
        let root = Path::new("/r");
        assert!(relative_prefix(root, root).is_empty());
        assert_eq!(relative_prefix(root, &root.join("a").join("b")), b"a/b/");
    }
}
