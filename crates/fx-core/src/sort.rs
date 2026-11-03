//! Explorer-like order: dirs first, then natural, ASCII-case-insensitive names.

use crate::listing::{EntryKind, Listing};
use rayon::slice::ParallelSliceMut;
use std::cmp::Ordering;

const PARALLEL_SORT_THRESHOLD: usize = 10_000;

/// Digit runs compare numerically ("a2" < "a10"); other bytes compare ASCII-case-insensitively.
// ponytail: non-ASCII letters compare by byte; add Unicode case folding if Explorer parity matters.
pub fn natural_cmp(a: &[u8], b: &[u8]) -> Ordering {
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        if a[i].is_ascii_digit() && b[j].is_ascii_digit() {
            let (ei, ej) = (digits_end(a, i), digits_end(b, j));
            let (da, db) = (trim_zeros(&a[i..ei]), trim_zeros(&b[j..ej]));
            let ord = da.len().cmp(&db.len()).then_with(|| da.cmp(db));
            if ord.is_ne() {
                return ord;
            }
            (i, j) = (ei, ej);
        } else {
            let ord = a[i].to_ascii_lowercase().cmp(&b[j].to_ascii_lowercase());
            if ord.is_ne() {
                return ord;
            }
            i += 1;
            j += 1;
        }
    }
    (a.len() - i).cmp(&(b.len() - j))
}

fn digits_end(s: &[u8], from: usize) -> usize {
    from + s[from..].iter().take_while(|b| b.is_ascii_digit()).count()
}

fn trim_zeros(s: &[u8]) -> &[u8] {
    let zeros = s.iter().take_while(|&&b| b == b'0').count();
    &s[zeros..]
}

pub(crate) fn sort(listing: &mut Listing) {
    let (names, entries) = listing.names_and_entries_mut();
    let cmp = |a: &crate::Entry, b: &crate::Entry| {
        let (a_dir, b_dir) = (a.kind == EntryKind::Dir, b.kind == EntryKind::Dir);
        b_dir.cmp(&a_dir).then_with(|| {
            let name = |e: &crate::Entry| {
                let start = e.name_off as usize;
                &names[start..start + e.name_len as usize]
            };
            let (an, bn) = (name(a), name(b));
            natural_cmp(an, bn).then_with(|| an.cmp(bn))
        })
    };
    if entries.len() > PARALLEL_SORT_THRESHOLD {
        entries.par_sort_unstable_by(cmp);
    } else {
        entries.sort_unstable_by(cmp);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;

    #[test]
    fn natural_and_case_insensitive() {
        assert_eq!(natural_cmp(b"a2", b"a10"), Ordering::Less);
        assert_eq!(natural_cmp(b"B", b"a"), Ordering::Greater);
        assert_eq!(natural_cmp(b"file01", b"file1"), Ordering::Equal);
        assert_eq!(natural_cmp(b"a", b"ab"), Ordering::Less);
    }

    #[test]
    fn dirs_first() {
        let mut l = Listing::default();
        l.push(OsStr::new("a10"), EntryKind::File, false, 0);
        l.push(OsStr::new("z"), EntryKind::Dir, false, 0);
        l.push(OsStr::new("A2"), EntryKind::File, false, 0);
        sort(&mut l);
        let order: Vec<_> = l.entries.iter().map(|e| l.name(e).to_owned()).collect();
        assert_eq!(order, ["z", "A2", "a10"]);
    }
}
