//! One loaded directory (immutable, shared) and the pure rules that turn it into a visible, sorted row order.

use crate::format::extension;
use crate::settings::FolderOrder;
use fx_core::{Entry, EntryKind, Listed, Listing, StorageKind, natural_cmp};
use std::cmp::Ordering;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

#[derive(Debug)]
pub struct DirView {
    pub path: PathBuf,
    pub listing: Listing,
    pub storage: StorageKind,
    /// Wall time of the listing, measured on the loader thread.
    pub elapsed: Duration,
    /// Entries of kind `File`.
    pub files: usize,
    /// Entries of kind `Dir`.
    pub dirs: usize,
    /// Entries of kind `Symlink`.
    pub symlinks: usize,
    /// Entries of kind `Other` (sockets, devices, ...).
    pub others: usize,
    /// Sum of sizes of `File` entries.
    pub total_file_bytes: u64,
    /// Directory mtime, stat'd before the listing started so a change during listing is seen later.
    /// `None` when the filesystem does not report it.
    pub mtime: Option<SystemTime>,
    /// Wall-clock time when the listing started; compared with `mtime` to spot racy stamps.
    pub listed_at: SystemTime,
}

/// A directory mtime this close to the listing start may have been shared by a change we missed
/// (coarse timestamps, e.g. FAT/SMB), so the listing cannot be trusted as current. Same idea as git's racy index entries.
pub(crate) const RACY_WINDOW: Duration = Duration::from_secs(2);

/// Whether a folder listed with (`mtime`, `listed_at`) must be listed again, given its `current` mtime.
pub(crate) fn needs_relist(
    mtime: Option<SystemTime>,
    listed_at: SystemTime,
    current: Option<SystemTime>,
) -> bool {
    if mtime != current {
        return true;
    }
    mtime.is_some_and(|m| {
        listed_at
            .duration_since(m)
            .map_or(true, |age| age < RACY_WINDOW)
    })
}

impl DirView {
    pub(crate) fn new(
        path: PathBuf,
        listed: Listed,
        elapsed: Duration,
        mtime: Option<SystemTime>,
        listed_at: SystemTime,
    ) -> Self {
        let Listed {
            listing, storage, ..
        } = listed;
        let (mut files, mut dirs, mut symlinks, mut others, mut total_file_bytes) = (0, 0, 0, 0, 0);
        for entry in &listing.entries {
            match entry.kind {
                EntryKind::File => {
                    files += 1;
                    total_file_bytes += entry.size.unwrap_or(0);
                }
                EntryKind::Dir => dirs += 1,
                EntryKind::Symlink => symlinks += 1,
                EntryKind::Other => others += 1,
            }
        }
        Self {
            path,
            listing,
            storage,
            elapsed,
            files,
            dirs,
            symlinks,
            others,
            total_file_bytes,
            mtime,
            listed_at,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SortColumn {
    Name,
    Ext,
    Size,
    Modified,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SortSpec {
    pub column: SortColumn,
    pub ascending: bool,
}

impl Default for SortSpec {
    fn default() -> Self {
        Self {
            column: SortColumn::Name,
            ascending: true,
        }
    }
}

/// Indices into `listing.entries` that pass the hidden and name filters, sorted by `sort`.
/// `folders` places directories; `sort.ascending` applies within each group and never moves the groups.
pub(crate) fn build_order(
    listing: &Listing,
    show_hidden: bool,
    folders: FolderOrder,
    filter: &str,
    sort: SortSpec,
) -> Vec<u32> {
    let needle = filter.to_lowercase();
    let mut order: Vec<u32> = listing
        .entries
        .iter()
        .enumerate()
        .filter(|(_, e)| show_hidden || !e.hidden)
        .filter(|(_, e)| {
            needle.is_empty()
                || listing
                    .name(e)
                    .to_string_lossy()
                    .to_lowercase()
                    .contains(&needle)
        })
        .map(|(i, _)| i as u32)
        .collect();
    order.sort_unstable_by(|&a, &b| {
        compare(
            listing,
            &listing.entries[a as usize],
            &listing.entries[b as usize],
            sort,
            folders,
        )
    });
    order
}

fn compare(
    listing: &Listing,
    a: &Entry,
    b: &Entry,
    sort: SortSpec,
    folders: FolderOrder,
) -> Ordering {
    let is_dir = |e: &Entry| e.kind == EntryKind::Dir;
    let by_column = match sort.column {
        SortColumn::Name => directed(name_cmp(listing, a, b), sort.ascending),
        SortColumn::Ext => directed(
            ext_cmp(listing, a, b).then_with(|| name_cmp(listing, a, b)),
            sort.ascending,
        ),
        SortColumn::Size => {
            none_last(a.size, b.size, sort.ascending).then_with(|| name_cmp(listing, a, b))
        }
        SortColumn::Modified => {
            none_last(a.modified, b.modified, sort.ascending).then_with(|| name_cmp(listing, a, b))
        }
    };
    let group = match folders {
        FolderOrder::Top => is_dir(b).cmp(&is_dir(a)),
        FolderOrder::Bottom => is_dir(a).cmp(&is_dir(b)),
        FolderOrder::Mixed => Ordering::Equal,
    };
    group.then(by_column)
}

fn name_cmp(listing: &Listing, a: &Entry, b: &Entry) -> Ordering {
    let (an, bn) = (listing.name_bytes(a), listing.name_bytes(b));
    natural_cmp(an, bn).then_with(|| an.cmp(bn))
}

/// Compares ASCII-lowercased extensions without allocating; directories have none, so they tie and fall through to the name.
fn ext_cmp(listing: &Listing, a: &Entry, b: &Entry) -> Ordering {
    let (an, bn) = (
        listing.name(a).to_string_lossy(),
        listing.name(b).to_string_lossy(),
    );
    lower_bytes(a, &an).cmp(lower_bytes(b, &bn))
}

fn lower_bytes<'s>(e: &Entry, name: &'s str) -> impl Iterator<Item = u8> + 's {
    let ext = if e.kind == EntryKind::Dir {
        ""
    } else {
        extension(name)
    };
    ext.bytes().map(|c| c.to_ascii_lowercase())
}

fn directed(ord: Ordering, ascending: bool) -> Ordering {
    if ascending { ord } else { ord.reverse() }
}

/// Missing values sort last in both directions.
fn none_last<T: Ord>(a: Option<T>, b: Option<T>, ascending: bool) -> Ordering {
    match (a, b) {
        (Some(x), Some(y)) => directed(x.cmp(&y), ascending),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;

    fn listing_of(names: &[(&str, EntryKind)]) -> Listing {
        let mut l = Listing::default();
        for (name, kind) in names {
            l.push(OsStr::new(name), *kind, false, 0);
        }
        l
    }

    #[test]
    fn counts_every_kind() {
        let l = listing_of(&[
            ("a", EntryKind::File),
            ("d", EntryKind::Dir),
            ("l", EntryKind::Symlink),
            ("m", EntryKind::Symlink),
            ("s", EntryKind::Other),
        ]);
        let listed = Listed {
            listing: l,
            storage: StorageKind::Unknown,
            strategy: fx_core::Strategy::for_kind(StorageKind::Unknown, 1),
        };
        let v = DirView::new(
            PathBuf::new(),
            listed,
            Duration::ZERO,
            None,
            SystemTime::now(),
        );
        assert_eq!((v.files, v.dirs, v.symlinks, v.others), (1, 1, 2, 1));
    }

    #[test]
    fn ext_sort_is_case_insensitive_and_dirs_stay_first() {
        let l = listing_of(&[
            ("b.TXT", EntryKind::File),
            ("a.rs", EntryKind::File),
            ("z.d", EntryKind::Dir),
            ("c.txt", EntryKind::File),
        ]);
        let sort = SortSpec {
            column: SortColumn::Ext,
            ascending: true,
        };
        let names: Vec<_> = build_order(&l, false, FolderOrder::Top, "", sort)
            .iter()
            .map(|&i| l.name(&l.entries[i as usize]).to_owned())
            .collect();
        assert_eq!(names, ["z.d", "a.rs", "b.TXT", "c.txt"]);
    }

    #[test]
    fn folder_order_modes_by_column_and_direction() {
        use FolderOrder::{Bottom, Mixed, Top};
        let mut l = listing_of(&[
            ("a", EntryKind::File),
            ("b", EntryKind::Dir),
            ("c", EntryKind::File),
            ("d", EntryKind::Dir),
            ("e", EntryKind::File),
        ]);
        let at = |s| SystemTime::UNIX_EPOCH + Duration::from_secs(s);
        for (e, (size, secs)) in l.entries.iter_mut().zip([
            (Some(30), 30),
            (None, 25),
            (Some(10), 10),
            (None, 5),
            (Some(20), 20),
        ]) {
            (e.size, e.modified) = (size, Some(at(secs)));
        }
        let cases = [
            (SortColumn::Name, true, Top, "bdace"),
            (SortColumn::Name, true, Bottom, "acebd"),
            (SortColumn::Name, true, Mixed, "abcde"),
            (SortColumn::Name, false, Top, "dbeca"),
            (SortColumn::Name, false, Bottom, "ecadb"),
            (SortColumn::Name, false, Mixed, "edcba"),
            (SortColumn::Modified, true, Top, "dbcea"),
            (SortColumn::Modified, true, Bottom, "ceadb"),
            (SortColumn::Modified, true, Mixed, "dceba"),
            (SortColumn::Modified, false, Top, "bdaec"),
            (SortColumn::Modified, false, Bottom, "aecbd"),
            (SortColumn::Modified, false, Mixed, "abecd"),
            (SortColumn::Size, true, Top, "bdcea"),
            (SortColumn::Size, true, Bottom, "ceabd"),
            (SortColumn::Size, true, Mixed, "ceabd"),
            (SortColumn::Size, false, Top, "bdaec"),
            (SortColumn::Size, false, Bottom, "aecbd"),
            (SortColumn::Size, false, Mixed, "aecbd"),
        ];
        for (column, ascending, folders, want) in cases {
            let sort = SortSpec { column, ascending };
            let got: String = build_order(&l, false, folders, "", sort)
                .iter()
                .map(|&i| {
                    l.name(&l.entries[i as usize])
                        .to_string_lossy()
                        .into_owned()
                })
                .collect();
            assert_eq!(got, want, "{column:?} asc={ascending} {folders:?}");
        }
    }

    #[test]
    fn relist_decision_handles_changed_and_racy_stamps() {
        let at = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        let old = at - Duration::from_secs(60);
        let recent = at - Duration::from_millis(500);
        let later = at + Duration::from_secs(5);
        // unchanged and old: trust the listing
        assert!(!needs_relist(Some(old), at, Some(old)));
        // mtime moved either way: relist
        assert!(needs_relist(Some(old), at, Some(later)));
        assert!(needs_relist(
            Some(old),
            at,
            Some(old - Duration::from_secs(1))
        ));
        // unchanged but within the racy window (or in the future): relist
        assert!(needs_relist(Some(recent), at, Some(recent)));
        assert!(needs_relist(Some(later), at, Some(later)));
        // exactly at the window edge is safe
        let edge = at - RACY_WINDOW;
        assert!(!needs_relist(Some(edge), at, Some(edge)));
        // no mtime support: never poll-relist, but appearing/disappearing stamps do
        assert!(!needs_relist(None, at, None));
        assert!(needs_relist(None, at, Some(old)));
        assert!(needs_relist(Some(old), at, None));
    }
}
