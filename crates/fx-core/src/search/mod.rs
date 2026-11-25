//! Use case: streamed recursive name search on top of the parallel walk.

mod query;

use crate::list::ListError;
use crate::listing::{Entry, EntryKind, Listing};
use crate::walk::{WalkControl, WalkOptions, WalkVisitor, Walked, walk_cancellable};
pub use query::{GlobScope, SearchOptions, SearchQuery};
use std::io;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    pub path: PathBuf,
    pub kind: EntryKind,
    /// The entry is hidden itself, or sits below a folder whose name starts with a dot.
    pub hidden: bool,
}

/// A folder the search could not read, with the reason.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnreadableFolder {
    pub path: PathBuf,
    pub error: String,
}

#[derive(Debug)]
pub enum SearchEvent {
    /// Matches of one directory; never empty.
    Hits(Vec<Hit>),
    Unreadable(UnreadableFolder),
}

/// Streams matches and unreadable folders below `root` to `on_event`, from many threads.
/// Cancelling `control` stops the walk; the returned stats then cover the part that was visited,
/// and `on_event` is not called after `search` returns.
pub fn search<F>(
    root: &Path,
    query: &SearchQuery,
    opts: &WalkOptions,
    control: &WalkControl,
    on_event: F,
) -> Result<Walked, ListError>
where
    F: Fn(SearchEvent) + Sync,
{
    let visitor = SearchVisitor {
        root,
        query,
        on_event,
    };
    walk_cancellable(root, opts, control, &visitor)
}

struct SearchVisitor<'a, F> {
    root: &'a Path,
    query: &'a SearchQuery,
    on_event: F,
}

impl<F: Fn(SearchEvent) + Sync> WalkVisitor for SearchVisitor<'_, F> {
    fn directory(&self, dir: &Path, listing: &Listing) {
        let mut batch = Batch::new(self.root, dir, listing);
        if self.query.matches_relative() {
            relative_hits(self.query, &mut batch);
        } else {
            name_hits(self.query, &mut batch);
        }
        if !batch.hits.is_empty() {
            (self.on_event)(SearchEvent::Hits(batch.hits));
        }
    }

    fn unreadable(&self, dir: &Path, err: &io::Error) {
        (self.on_event)(SearchEvent::Unreadable(UnreadableFolder {
            path: dir.to_path_buf(),
            error: err.to_string(),
        }));
    }
}

/// Hits of one directory. Allocates nothing until the first hit, and the dot-ancestor check
/// runs only for directories that have one.
struct Batch<'a> {
    root: &'a Path,
    dir: &'a Path,
    listing: &'a Listing,
    in_hidden_dir: Option<bool>,
    hits: Vec<Hit>,
}

impl<'a> Batch<'a> {
    fn new(root: &'a Path, dir: &'a Path, listing: &'a Listing) -> Self {
        Self {
            root,
            dir,
            listing,
            in_hidden_dir: None,
            hits: Vec::new(),
        }
    }

    fn push(&mut self, e: &Entry) {
        if self.hits.capacity() == 0 {
            self.hits.reserve_exact(self.listing.len());
        }
        let in_hidden_dir = *self
            .in_hidden_dir
            .get_or_insert_with(|| below_dot_dir(self.root, self.dir));
        self.hits.push(Hit {
            path: child_path(self.dir, self.listing.name(e)),
            kind: e.kind,
            hidden: e.hidden || in_hidden_dir,
        });
    }
}

fn name_hits(query: &SearchQuery, batch: &mut Batch) {
    let listing = batch.listing;
    for e in &listing.entries {
        if query.matches_name(listing.name_bytes(e)) {
            batch.push(e);
        }
    }
}

fn relative_hits(query: &SearchQuery, batch: &mut Batch) {
    let listing = batch.listing;
    let mut haystack = relative_prefix(batch.root, batch.dir);
    let prefix_len = haystack.len();
    for e in &listing.entries {
        haystack.truncate(prefix_len);
        haystack.extend_from_slice(listing.name_bytes(e));
        if query.matches_name(&haystack) {
            batch.push(e);
        }
    }
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

/// Same path as `dir.join(name)` in a single allocation of the exact size.
fn child_path(dir: &Path, name: &std::ffi::OsStr) -> PathBuf {
    let mut path = PathBuf::with_capacity(dir.as_os_str().len() + 1 + name.len());
    path.push(dir);
    path.push(name);
    path
}

/// Some folder between `root` (exclusive) and `dir` starts with a dot.
/// Hidden attributes of ancestors are not tracked; only the dot convention is.
fn below_dot_dir(root: &Path, dir: &Path) -> bool {
    dir.strip_prefix(root).is_ok_and(|rel| {
        rel.components()
            .any(|c| c.as_os_str().as_encoded_bytes().first() == Some(&b'.'))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dot_ancestors_are_found_below_the_root_only() {
        let root = Path::new("/r/.cfg");
        assert!(!below_dot_dir(root, root));
        assert!(!below_dot_dir(root, &root.join("a")));
        assert!(below_dot_dir(root, &root.join("a").join(".git").join("b")));
    }

    #[test]
    fn relative_prefix_is_empty_at_root_and_slash_terminated_below() {
        let root = Path::new("/r");
        assert!(relative_prefix(root, root).is_empty());
        assert_eq!(relative_prefix(root, &root.join("a").join("b")), b"a/b/");
    }

    #[test]
    fn child_path_equals_join() {
        let name = std::ffi::OsStr::new("f.txt");
        for dir in ["/r", "/r/", "/r/a b"] {
            let dir = Path::new(dir);
            assert_eq!(child_path(dir, name), dir.join(name));
        }
    }
}
