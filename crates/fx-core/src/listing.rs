//! Domain: a directory listing whose names live in one byte arena.

use std::ffi::OsStr;
use std::time::SystemTime;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EntryKind {
    File,
    Dir,
    /// Symlinks, and Windows reparse points (junctions).
    Symlink,
    Other,
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub(crate) name_off: u32,
    pub(crate) name_len: u32,
    pub kind: EntryKind,
    pub size: Option<u64>,
    pub modified: Option<SystemTime>,
    pub hidden: bool,
    /// Inode number; the file id on Windows; 0 if unknown.
    pub inode: u64,
}

/// Names are encoded OsStr bytes (WTF-8 on Windows), concatenated without separators.
#[derive(Default, Debug)]
pub struct Listing {
    names: Vec<u8>,
    pub entries: Vec<Entry>,
}

impl Listing {
    /// Appends an entry with no metadata; backends fill `size`/`modified` afterwards.
    pub fn push(&mut self, name: &OsStr, kind: EntryKind, hidden: bool, inode: u64) {
        self.push_encoded(name.as_encoded_bytes(), kind, hidden, inode);
    }

    /// Like [`push`](Self::push) from bytes already in `OsStr` encoded form
    /// (raw bytes on Unix, WTF-8 on Windows, see [`push_wtf8_from_utf16`]).
    pub fn push_encoded(&mut self, name: &[u8], kind: EntryKind, hidden: bool, inode: u64) {
        let name_off = u32::try_from(self.names.len()).expect("name arena exceeds 4 GiB");
        let name_len = u32::try_from(name.len()).expect("name exceeds 4 GiB");
        self.names.extend_from_slice(name);
        self.entries.push(Entry {
            name_off,
            name_len,
            kind,
            size: None,
            modified: None,
            hidden,
            inode,
        });
    }

    pub fn name_bytes(&self, e: &Entry) -> &[u8] {
        let start = e.name_off as usize;
        &self.names[start..start + e.name_len as usize]
    }

    pub fn name(&self, e: &Entry) -> &OsStr {
        // SAFETY: the arena only receives bytes from `OsStr::as_encoded_bytes`
        // or `push_wtf8_from_utf16`, both valid `OsStr` encodings, sliced on entry bounds.
        unsafe { OsStr::from_encoded_bytes_unchecked(self.name_bytes(e)) }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Disjoint borrows of the arena and entries, for sorting.
    pub(crate) fn names_and_entries_mut(&mut self) -> (&[u8], &mut Vec<Entry>) {
        (&self.names, &mut self.entries)
    }
}

/// Appends UTF-16 `units` to `out` as WTF-8, keeping lone surrogates (as Windows names may contain).
#[cfg_attr(not(windows), allow(dead_code))]
pub fn push_wtf8_from_utf16(out: &mut Vec<u8>, units: &[u16]) {
    for decoded in char::decode_utf16(units.iter().copied()) {
        match decoded {
            Ok(c) => out.extend_from_slice(c.encode_utf8(&mut [0; 4]).as_bytes()),
            Err(e) => {
                // Lone surrogate U+D800..=U+DFFF: generalized 3-byte UTF-8.
                let u = u32::from(e.unpaired_surrogate());
                out.extend_from_slice(&[
                    0xE0 | (u >> 12) as u8,
                    0x80 | ((u >> 6) & 0x3F) as u8,
                    0x80 | (u & 0x3F) as u8,
                ]);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wtf8_handles_pairs_and_lone_surrogates() {
        let mut out = Vec::new();
        // "a", U+1F600 as a surrogate pair, two lone high surrogates, "é"
        push_wtf8_from_utf16(&mut out, &[0x61, 0xD83D, 0xDE00, 0xD800, 0xDC00 - 1, 0xE9]);
        let mut expected = b"a".to_vec();
        expected.extend_from_slice("\u{1F600}".as_bytes());
        expected.extend_from_slice(&[0xED, 0xA0, 0x80]); // U+D800
        expected.extend_from_slice(&[0xED, 0xAF, 0xBF]); // U+DBFF
        expected.extend_from_slice("é".as_bytes());
        assert_eq!(out, expected);
    }

    #[test]
    fn arena_round_trips_names() {
        let mut l = Listing::default();
        l.push(OsStr::new("a"), EntryKind::File, false, 1);
        l.push(OsStr::new("bc"), EntryKind::Dir, true, 2);
        assert_eq!(l.name(&l.entries[0]), "a");
        assert_eq!(l.name(&l.entries[1]), "bc");
    }
}
