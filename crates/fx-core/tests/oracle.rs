//! Native vs Std backend must agree on every forced storage kind.
//! Trivial while Native falls back to std; meaningful once an OS backend lands.

use fx_core::{
    Backend, EntryKind, ListOptions, Listing, StorageKind, WalkOptions, WalkStats, list_dir, walk,
};
use std::ffi::OsString;
use std::fs;
use std::path::Path;
use std::time::SystemTime;

type Row = (
    OsString,
    EntryKind,
    Option<u64>,
    Option<SystemTime>,
    bool,
    u64,
);

fn build_tree(root: &Path) {
    for i in 0..5_000 {
        fs::write(root.join(format!("file{i:04}.txt")), vec![b'x'; i % 7]).unwrap();
    }
    for name in [
        "héllo wörld.txt",
        "日本語.txt",
        "with space.txt",
        ".hidden",
        ".hidden dir/inner",
    ] {
        let p = root.join(name);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, b"data").unwrap();
    }
    fs::write(root.join("empty"), b"").unwrap();
    fs::write(root.join("big.bin"), vec![0u8; 1 << 20]).unwrap();
    fs::create_dir_all(root.join("nested/deeper/deepest")).unwrap();
    fs::write(root.join("nested/deeper/leaf.txt"), b"leaf").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::{PermissionsExt, symlink};
        symlink("empty", root.join("link-to-file")).unwrap();
        symlink("nested", root.join("link-to-dir")).unwrap();
        symlink("missing-target", root.join("dangling")).unwrap();
        fs::create_dir(root.join("unreadable")).unwrap();
        fs::set_permissions(root.join("unreadable"), fs::Permissions::from_mode(0o000)).unwrap();
    }
}

/// Makes "unreadable" removable again even when an assertion fails first.
struct RestorePerms<'a>(&'a Path);

impl Drop for RestorePerms<'_> {
    fn drop(&mut self) {
        let _root = self.0; // only Unix has an unreadable dir to restore
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ =
                fs::set_permissions(self.0.join("unreadable"), fs::Permissions::from_mode(0o755));
        }
    }
}

fn rows(l: &Listing) -> Vec<Row> {
    let mut rows: Vec<Row> = l
        .entries
        .iter()
        .map(|e| {
            // The Windows std backend cannot report file ids (always 0), so only compare on Unix.
            let inode = if cfg!(unix) { e.inode } else { 0 };
            (
                l.name(e).to_owned(),
                e.kind,
                e.size,
                e.modified,
                e.hidden,
                inode,
            )
        })
        .collect();
    rows.sort_by(|a, b| a.0.cmp(&b.0));
    rows
}

fn list(dir: &Path, backend: Backend, storage: StorageKind, metadata: bool) -> Vec<Row> {
    let opts = ListOptions {
        metadata,
        sort: false,
        backend,
        storage: Some(storage),
    };
    rows(&list_dir(dir, &opts).unwrap().listing)
}

#[test]
fn native_matches_std_for_every_storage_kind() {
    let tmp = tempfile::tempdir().unwrap();
    build_tree(tmp.path());
    let _restore = RestorePerms(tmp.path());
    let dirs = [
        tmp.path().to_path_buf(),
        tmp.path().join("nested"),
        tmp.path().join("nested/deeper/deepest"),
    ];

    for kind in [
        StorageKind::Ssd,
        StorageKind::Hdd,
        StorageKind::Network,
        StorageKind::Unknown,
    ] {
        for dir in &dirs {
            for metadata in [true, false] {
                let native = list(dir, Backend::Native, kind, metadata);
                let std = list(dir, Backend::Std, kind, metadata);
                assert_eq!(
                    native,
                    std,
                    "{kind:?} {} metadata={metadata}",
                    dir.display()
                );
            }
        }
    }
    // Sanity: the oracle tree is actually listed.
    assert!(list(tmp.path(), Backend::Std, StorageKind::Ssd, true).len() >= 5_000);
}

#[test]
fn errors_carry_the_path() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("f");
    fs::write(&file, b"").unwrap();
    let missing = tmp.path().join("nope");
    for backend in [Backend::Native, Backend::Std] {
        let opts = ListOptions {
            backend,
            ..ListOptions::default()
        };
        let e = list_dir(&missing, &opts).unwrap_err();
        assert!(matches!(e, fx_core::ListError::NotFound(_)), "{e}");
        assert_eq!(e.path(), missing);
        let e = list_dir(&file, &opts).unwrap_err();
        assert!(matches!(e, fx_core::ListError::NotADirectory(_)), "{e}");
    }
}

/// Sequential std recursion; an unreadable dir counts as an error but is still a dir.
fn std_count(dir: &Path, s: &mut WalkStats) {
    let Ok(rd) = fs::read_dir(dir) else {
        s.errors += 1;
        return;
    };
    for e in rd.map(Result::unwrap) {
        let t = e.file_type().unwrap();
        if t.is_symlink() {
            s.symlinks += 1;
        } else if t.is_dir() {
            s.dirs += 1;
            std_count(&e.path(), s);
        } else if t.is_file() {
            s.files += 1;
        } else {
            s.others += 1;
        }
    }
}

#[test]
fn walk_counts_match_sequential_std() {
    let tmp = tempfile::tempdir().unwrap();
    build_tree(tmp.path());
    let _restore = RestorePerms(tmp.path());
    let mut expected = WalkStats::default();
    std_count(tmp.path(), &mut expected);
    assert!(
        expected.files >= 5_000 && expected.dirs >= 4 && expected.symlinks == cfg!(unix) as u64 * 3
    );

    // build_tree made "unreadable" mode 000; as root that is not enforced.
    let unreadable_enforced = cfg!(unix) && fs::read_dir(tmp.path().join("unreadable")).is_err();
    assert_eq!(expected.errors, u64::from(unreadable_enforced));

    for backend in [Backend::Native, Backend::Std] {
        for kind in [
            StorageKind::Ssd,
            StorageKind::Hdd,
            StorageKind::Network,
            StorageKind::Unknown,
        ] {
            let dirs_seen = std::sync::atomic::AtomicU64::new(0);
            let opts = WalkOptions {
                backend,
                storage: Some(kind),
            };
            let walked = walk(tmp.path(), &opts, |_, _| {
                dirs_seen.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            })
            .unwrap();
            assert_eq!(walked.stats, expected, "{backend:?} {kind:?}");
            // Every readable dir (root + dirs - failed ones) is reported once.
            assert_eq!(
                dirs_seen.into_inner(),
                1 + expected.dirs - expected.errors,
                "{backend:?} {kind:?}"
            );
        }
    }
}

#[test]
fn walk_root_errors_are_errors() {
    let tmp = tempfile::tempdir().unwrap();
    let e = walk(&tmp.path().join("nope"), &WalkOptions::default(), |_, _| {}).unwrap_err();
    assert!(matches!(e, fx_core::ListError::NotFound(_)), "{e}");
}
