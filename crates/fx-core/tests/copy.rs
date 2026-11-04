//! Copy engine end to end on temp dirs: trees of many small files, every schedule and method of the host,
//! conflict policies, resume, cancel/pause and move safety.

use fx_core::{
    ConflictPolicy, Item, Method, Op, Order, Overrides, Request, Resolution, Schedule, Summary,
    TransferHandle,
};
use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, SystemTime};

/// Deterministic pseudo-random bytes (xorshift), so failures reproduce.
fn bytes(seed: u64, len: usize) -> Vec<u8> {
    let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x as u8
        })
        .collect()
}

/// `dirs` folders of `files` files each (0..6 KiB), two levels deep, with fixed old mtimes.
fn small_tree(root: &Path, dirs: usize, files: usize) {
    let old = SystemTime::UNIX_EPOCH + Duration::new(1_600_000_000, 123_456_789);
    for d in 0..dirs {
        let dir = root.join(format!("d{:02}", d % 7)).join(format!("sub{d}"));
        fs::create_dir_all(&dir).unwrap();
        for f in 0..files {
            let path = dir.join(format!("f{f}.bin"));
            fs::write(
                &path,
                bytes((d * 1000 + f) as u64, (d * 131 + f * 97) % 6000),
            )
            .unwrap();
            fs::File::options()
                .write(true)
                .open(&path)
                .unwrap()
                .set_modified(old)
                .unwrap();
        }
    }
    fs::write(root.join("top.txt"), b"top").unwrap();
}

/// Asserts `b` is a copy of `a`: same names, kinds, contents, symlink targets and mtimes (dirs included).
fn assert_same_tree(a: &Path, b: &Path) {
    let (ma, mb) = (
        fs::symlink_metadata(a).unwrap(),
        fs::symlink_metadata(b).unwrap(),
    );
    assert_eq!(
        ma.file_type().is_dir(),
        mb.file_type().is_dir(),
        "{}",
        b.display()
    );
    assert_eq!(
        ma.file_type().is_symlink(),
        mb.file_type().is_symlink(),
        "{}",
        b.display()
    );
    if ma.file_type().is_symlink() {
        assert_eq!(fs::read_link(a).unwrap(), fs::read_link(b).unwrap());
        return;
    }
    if ma.is_dir() {
        let names = |p: &Path| {
            let mut v: Vec<_> = fs::read_dir(p)
                .unwrap()
                .map(|e| e.unwrap().file_name())
                .collect();
            v.sort();
            v
        };
        let (na, nb) = (names(a), names(b));
        assert_eq!(na, nb, "entries of {}", b.display());
        for n in na {
            assert_same_tree(&a.join(&n), &b.join(&n));
        }
    } else {
        assert_eq!(
            fs::read(a).unwrap(),
            fs::read(b).unwrap(),
            "{}",
            b.display()
        );
    }
    assert_eq!(
        ma.modified().unwrap(),
        mb.modified().unwrap(),
        "mtime of {}",
        b.display()
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            ma.permissions().mode() & 0o7777,
            mb.permissions().mode() & 0o7777,
            "mode of {}",
            b.display()
        );
    }
}

fn request(items: Vec<Item>, op: Op, policy: ConflictPolicy, overrides: Overrides) -> Request {
    Request {
        items,
        op,
        policy,
        overrides,
    }
}

fn item(src: &Path, dst: &Path) -> Item {
    Item {
        src: src.to_path_buf(),
        dst: dst.to_path_buf(),
    }
}

fn run(req: &Request) -> Summary {
    TransferHandle::new(|| {}).run(req)
}

fn copy(src: &Path, dst: &Path, policy: ConflictPolicy, overrides: Overrides) -> Summary {
    run(&request(vec![item(src, dst)], Op::Copy, policy, overrides))
}

fn assert_clean(s: &Summary) {
    assert!(
        s.errors.iter().all(|e| e.warning),
        "errors: {:#?}",
        s.errors
    );
    assert!(!s.cancelled);
}

fn no_part_files(root: &Path) -> bool {
    fs::read_dir(root).unwrap().all(|e| {
        let e = e.unwrap();
        !e.file_name().to_string_lossy().ends_with(".fx-part")
            && (!e.file_type().unwrap().is_dir() || no_part_files(&e.path()))
    })
}

#[test]
fn many_small_files_streaming() {
    let t = tempfile::tempdir().unwrap();
    let src = t.path().join("src");
    small_tree(&src, 40, 30);
    let s = copy(
        &src,
        &t.path().join("dst"),
        ConflictPolicy::Skip,
        Overrides::default(),
    );
    assert_clean(&s);
    assert_eq!(s.files, 40 * 30 + 1);
    assert_eq!(s.items_done, 1);
    assert_same_tree(&src, &t.path().join("dst"));
}

#[test]
fn many_small_files_two_phase() {
    for order in [Order::Extent, Order::Inode] {
        let t = tempfile::tempdir().unwrap();
        let src = t.path().join("src");
        small_tree(&src, 12, 25);
        let overrides = Overrides {
            schedule: Some(Schedule::TwoPhase { order }),
            ..Overrides::default()
        };
        let s = copy(&src, &t.path().join("dst"), ConflictPolicy::Skip, overrides);
        assert_clean(&s);
        assert_eq!(s.files, 12 * 25 + 1);
        assert_same_tree(&src, &t.path().join("dst"));
    }
}

#[test]
fn every_host_method_copies_correctly() {
    let methods: &[Method] = if cfg!(target_os = "linux") {
        &[Method::CopyFileRange, Method::Sendfile, Method::ReadWrite]
    } else if cfg!(windows) {
        &[Method::CopyFile2, Method::ReadWrite]
    } else {
        &[Method::ReadWrite]
    };
    for &method in methods {
        let t = tempfile::tempdir().unwrap();
        let src = t.path().join("src");
        small_tree(&src, 3, 10);
        // A file larger than one 16 MiB chunk.
        fs::write(src.join("big.bin"), bytes(7, 17 << 20)).unwrap();
        let overrides = Overrides {
            method: Some(method),
            workers: Some(3),
            ..Overrides::default()
        };
        let s = copy(&src, &t.path().join("dst"), ConflictPolicy::Skip, overrides);
        assert_clean(&s);
        assert_same_tree(&src, &t.path().join("dst"));
    }
}

/// The default chain starts with a clone where the filesystem may support it; it must fall back cleanly.
#[test]
fn default_chain_falls_back_or_clones() {
    let t = tempfile::tempdir().unwrap();
    let src = t.path().join("src");
    small_tree(&src, 4, 20);
    let s = copy(
        &src,
        &t.path().join("dst"),
        ConflictPolicy::Skip,
        Overrides::default(),
    );
    assert_clean(&s);
    assert!(s.cloned == 0 || s.cloned <= s.files);
    assert_same_tree(&src, &t.path().join("dst"));
}

#[test]
fn copy_into_itself_is_refused_and_same_folder_keeps_both() {
    let t = tempfile::tempdir().unwrap();
    let dir = t.path().join("dir");
    fs::create_dir(&dir).unwrap();
    fs::write(dir.join("f.txt"), b"x").unwrap();
    let s = copy(
        &dir,
        &dir.join("inner"),
        ConflictPolicy::Overwrite,
        Overrides::default(),
    );
    assert_eq!(s.error_count(), 1);
    assert!(
        s.errors[0].message.contains("into itself"),
        "{:?}",
        s.errors
    );

    // Pasting next to itself duplicates, whatever the policy says.
    let s = copy(
        &dir.join("f.txt"),
        &dir.join("f.txt"),
        ConflictPolicy::Overwrite,
        Overrides::default(),
    );
    assert_clean(&s);
    assert_eq!(fs::read(dir.join("f (2).txt")).unwrap(), b"x");
    let s = copy(&dir, &dir, ConflictPolicy::Skip, Overrides::default());
    assert_clean(&s);
    assert_eq!(fs::read(t.path().join("dir (2)/f.txt")).unwrap(), b"x");
}

#[test]
fn conflict_policies() {
    let t = tempfile::tempdir().unwrap();
    let (src, dst) = (t.path().join("a.txt"), t.path().join("out"));
    fs::create_dir(&dst).unwrap();
    let target = dst.join("a.txt");
    let set_mtime = |p: &Path, secs: u64| {
        fs::File::options()
            .write(true)
            .open(p)
            .unwrap()
            .set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(secs))
            .unwrap();
    };
    let reset = || {
        fs::write(&src, b"new").unwrap();
        set_mtime(&src, 2_000_000_000);
        fs::write(&target, b"old content").unwrap();
        set_mtime(&target, 1_000_000_000);
    };

    reset();
    let s = copy(&src, &target, ConflictPolicy::Skip, Overrides::default());
    assert_eq!(
        (s.skipped, fs::read(&target).unwrap()),
        (1, b"old content".to_vec())
    );

    reset();
    let s = copy(
        &src,
        &target,
        ConflictPolicy::Overwrite,
        Overrides::default(),
    );
    assert_clean(&s);
    assert_eq!(fs::read(&target).unwrap(), b"new");
    assert!(no_part_files(&dst));

    reset();
    copy(
        &src,
        &target,
        ConflictPolicy::KeepBoth,
        Overrides::default(),
    );
    assert_eq!(fs::read(&target).unwrap(), b"old content");
    assert_eq!(fs::read(dst.join("a (2).txt")).unwrap(), b"new");

    reset();
    copy(
        &src,
        &target,
        ConflictPolicy::OverwriteIfNewer,
        Overrides::default(),
    );
    assert_eq!(fs::read(&target).unwrap(), b"new");
    reset();
    set_mtime(&src, 500_000_000);
    let s = copy(
        &src,
        &target,
        ConflictPolicy::OverwriteIfNewer,
        Overrides::default(),
    );
    assert_eq!(
        (s.skipped, fs::read(&target).unwrap()),
        (1, b"old content".to_vec())
    );

    // A file in the way of a folder is never replaced.
    let folder = t.path().join("folder");
    fs::create_dir(&folder).unwrap();
    fs::write(dst.join("folder"), b"file").unwrap();
    let s = copy(
        &folder,
        &dst.join("folder"),
        ConflictPolicy::Overwrite,
        Overrides::default(),
    );
    assert_eq!(s.error_count(), 1);
    assert_eq!(fs::read(dst.join("folder")).unwrap(), b"file");
}

#[test]
fn folders_merge_into_existing_folders() {
    let t = tempfile::tempdir().unwrap();
    let (src, dst) = (t.path().join("src"), t.path().join("dst"));
    fs::create_dir_all(src.join("sub")).unwrap();
    fs::write(src.join("sub/new.txt"), b"new").unwrap();
    fs::write(src.join("sub/both.txt"), b"src").unwrap();
    fs::create_dir_all(dst.join("sub")).unwrap();
    fs::write(dst.join("sub/old.txt"), b"old").unwrap();
    fs::write(dst.join("sub/both.txt"), b"dst").unwrap();
    let s = copy(&src, &dst, ConflictPolicy::Skip, Overrides::default());
    assert_eq!((s.files, s.skipped), (1, 1));
    assert_eq!(fs::read(dst.join("sub/new.txt")).unwrap(), b"new");
    assert_eq!(fs::read(dst.join("sub/old.txt")).unwrap(), b"old");
    assert_eq!(fs::read(dst.join("sub/both.txt")).unwrap(), b"dst");
}

#[test]
fn ask_waits_for_answers_and_apply_to_all() {
    let t = tempfile::tempdir().unwrap();
    let (src, dst) = (t.path().join("src"), t.path().join("dst"));
    fs::create_dir_all(&src).unwrap();
    fs::create_dir_all(&dst).unwrap();
    for i in 0..5 {
        fs::write(src.join(format!("{i}")), b"src").unwrap();
        fs::write(dst.join(format!("{i}")), b"dst").unwrap();
    }
    let asked = Arc::new(AtomicUsize::new(0));
    let handle = {
        let asked = Arc::clone(&asked);
        TransferHandle::new(move || {
            asked.fetch_add(1, Ordering::Relaxed);
        })
    };
    let req = request(
        vec![item(&src, &dst)],
        Op::Copy,
        ConflictPolicy::Ask,
        Overrides::default(),
    );
    let worker = {
        let handle = handle.clone();
        thread::spawn(move || handle.run(&req))
    };
    let first = loop {
        if let Some(c) = handle.conflicts().into_iter().next() {
            break c;
        }
        thread::yield_now();
    };
    assert!(first.dst.starts_with(&dst) && !first.src_is_dir && first.src_size == 3);
    handle.answer_conflict(first.id, Resolution::Overwrite, false);
    let second = loop {
        if let Some(c) = handle.conflicts().into_iter().find(|c| c.id != first.id) {
            break c;
        }
        thread::yield_now();
    };
    handle.answer_conflict(second.id, Resolution::Skip, true);
    let s = worker.join().unwrap();
    assert!(asked.load(Ordering::Relaxed) >= 2);
    assert_eq!((s.files, s.skipped), (1, 4));
    let srcs = (0..5).filter(|i| fs::read(dst.join(format!("{i}"))).unwrap() == b"src");
    assert_eq!(srcs.count(), 1);
}

#[test]
fn skip_identical_resumes_an_interrupted_copy() {
    let t = tempfile::tempdir().unwrap();
    let (src, dst) = (t.path().join("src"), t.path().join("dst"));
    small_tree(&src, 5, 10);
    assert_clean(&copy(
        &src,
        &dst,
        ConflictPolicy::Skip,
        Overrides::default(),
    ));
    // What a crash leaves behind: one truncated file with a fresh mtime, one file never written.
    let partial = dst.join("d00/sub0/f3.bin");
    fs::write(&partial, b"tr").unwrap();
    fs::remove_file(dst.join("d01/sub1/f4.bin")).unwrap();
    let s = copy(
        &src,
        &dst,
        ConflictPolicy::SkipIdentical,
        Overrides::default(),
    );
    assert_clean(&s);
    assert_eq!((s.files, s.skipped), (2, 5 * 10 + 1 - 2));
    assert_same_tree(&src, &dst);
}

#[test]
fn pause_holds_and_resume_finishes() {
    let t = tempfile::tempdir().unwrap();
    let src = t.path().join("src");
    small_tree(&src, 4, 10);
    let handle = TransferHandle::new(|| {});
    handle.pause();
    let req = request(
        vec![item(&src, &t.path().join("dst"))],
        Op::Copy,
        ConflictPolicy::Skip,
        Overrides::default(),
    );
    let worker = {
        let handle = handle.clone();
        thread::spawn(move || handle.run(&req))
    };
    thread::sleep(Duration::from_millis(100));
    assert_eq!(
        handle.snapshot().files_done,
        0,
        "nothing is copied while paused"
    );
    handle.resume();
    let s = worker.join().unwrap();
    assert_clean(&s);
    assert_same_tree(&src, &t.path().join("dst"));
}

#[test]
fn cancel_keeps_finished_files_and_leaves_no_partial_ones() {
    let t = tempfile::tempdir().unwrap();
    let (src, dst) = (t.path().join("src"), t.path().join("dst"));
    small_tree(&src, 40, 50);
    let handle = TransferHandle::new(|| {});
    let req = request(
        vec![item(&src, &dst)],
        Op::Copy,
        ConflictPolicy::Skip,
        Overrides::default(),
    );
    let worker = {
        let handle = handle.clone();
        thread::spawn(move || handle.run(&req))
    };
    while handle.snapshot().files_done < 50 && !worker.is_finished() {
        thread::yield_now();
    }
    handle.cancel();
    let s = worker.join().unwrap();
    assert!(s.cancelled);
    assert_eq!(s.items_done, 0);
    // Every file that exists is complete.
    fn check(src: &Path, dst: &Path) {
        for e in fs::read_dir(dst).unwrap() {
            let e = e.unwrap();
            let (s, d) = (src.join(e.file_name()), e.path());
            if e.file_type().unwrap().is_dir() {
                check(&s, &d);
            } else {
                assert_eq!(
                    fs::read(&s).unwrap(),
                    fs::read(&d).unwrap(),
                    "{}",
                    d.display()
                );
            }
        }
    }
    check(&src, &dst);
    assert!(no_part_files(&dst));
}

#[test]
fn move_on_one_volume_renames_and_merges() {
    let t = tempfile::tempdir().unwrap();
    let (src, out) = (t.path().join("src"), t.path().join("out"));
    small_tree(&src, 3, 5);
    fs::create_dir_all(out.join("src/d00")).unwrap();
    fs::write(out.join("src/keep.txt"), b"keep").unwrap();
    let s = run(&request(
        vec![item(&src, &out.join("src"))],
        Op::Move,
        ConflictPolicy::Skip,
        Overrides::default(),
    ));
    assert_clean(&s);
    assert!(s.plans.is_empty(), "no data was copied: {:?}", s.plans);
    assert!(!src.exists(), "a fully merged source folder is removed");
    assert_eq!(fs::read(out.join("src/keep.txt")).unwrap(), b"keep");
    assert!(out.join("src/d00/sub0/f1.bin").is_file());
    assert!(out.join("src/top.txt").is_file());
}

#[test]
fn copied_move_deletes_the_source_only_when_complete() {
    let t = tempfile::tempdir().unwrap();
    let (src, dst) = (t.path().join("src"), t.path().join("dst"));
    small_tree(&src, 3, 5);
    let reference = t.path().join("reference");
    assert_clean(&copy(
        &src,
        &reference,
        ConflictPolicy::Skip,
        Overrides::default(),
    ));
    // A forced method skips the rename, which exercises the cross-volume path on one volume.
    let forced = Overrides {
        method: Some(Method::ReadWrite),
        ..Overrides::default()
    };
    let s = run(&request(
        vec![item(&src, &dst)],
        Op::Move,
        ConflictPolicy::Skip,
        forced,
    ));
    assert_clean(&s);
    assert_eq!(s.items_done, 1);
    assert!(!src.exists());
    assert_same_tree(&reference, &dst);

    // An item with a skipped file keeps its source.
    let src2 = t.path().join("src2");
    small_tree(&src2, 1, 3);
    let dst2 = t.path().join("dst2");
    fs::create_dir_all(dst2.join("d00/sub0")).unwrap();
    fs::write(dst2.join("d00/sub0/f1.bin"), b"in the way").unwrap();
    let s = run(&request(
        vec![item(&src2, &dst2)],
        Op::Move,
        ConflictPolicy::Skip,
        forced,
    ));
    assert_eq!(s.kept_sources, vec![src2.clone()]);
    assert!(src2.join("d00/sub0/f1.bin").is_file() && src2.join("top.txt").is_file());
}

#[test]
fn keep_both_move_verifies_the_renamed_copy_and_keeps_the_existing_file() {
    let t = tempfile::tempdir().unwrap();
    let (src, dst) = (t.path().join("src"), t.path().join("dst"));
    fs::create_dir_all(&src).unwrap();
    fs::create_dir_all(&dst).unwrap();
    // Same size and mtime as the source, different content: only the name (2) copy may justify deleting.
    fs::write(src.join("x"), b"new!").unwrap();
    fs::write(dst.join("x"), b"old!").unwrap();
    let when = fs::metadata(src.join("x")).unwrap().modified().unwrap();
    fs::File::options()
        .write(true)
        .open(dst.join("x"))
        .unwrap()
        .set_modified(when)
        .unwrap();
    let forced = Overrides {
        method: Some(Method::ReadWrite),
        ..Overrides::default()
    };
    let s = run(&request(
        vec![item(&src, &dst)],
        Op::Move,
        ConflictPolicy::KeepBoth,
        forced,
    ));
    assert_clean(&s);
    assert!(s.kept_sources.is_empty(), "{:?}", s.kept_sources);
    assert!(!src.exists());
    assert_eq!(fs::read(dst.join("x")).unwrap(), b"old!");
    assert_eq!(fs::read(dst.join("x (2)")).unwrap(), b"new!");
}

#[cfg(unix)]
mod unix {
    use super::*;
    use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};

    fn is_root() -> bool {
        rustix::process::geteuid().is_root()
    }

    #[test]
    fn symlinks_hardlinks_modes_and_read_only_folders() {
        let t = tempfile::tempdir().unwrap();
        let src = t.path().join("src");
        small_tree(&src, 2, 3);
        symlink("top.txt", src.join("link")).unwrap();
        symlink("/nonexistent/target", src.join("dangling")).unwrap();
        fs::hard_link(src.join("top.txt"), src.join("hard")).unwrap();
        fs::set_permissions(src.join("top.txt"), fs::Permissions::from_mode(0o640)).unwrap();
        let ro = src.join("readonly");
        fs::create_dir(&ro).unwrap();
        fs::write(ro.join("inside"), b"x").unwrap();
        fs::set_permissions(&ro, fs::Permissions::from_mode(0o555)).unwrap();

        let dst = t.path().join("dst");
        let s = copy(&src, &dst, ConflictPolicy::Skip, Overrides::default());
        assert_clean(&s);
        let a = fs::metadata(dst.join("top.txt")).unwrap();
        let b = fs::metadata(dst.join("hard")).unwrap();
        assert_eq!(a.ino(), b.ino(), "hardlinks stay linked");
        assert_eq!(a.permissions().mode() & 0o777, 0o640);
        let ro_copy = dst.join("readonly");
        assert_eq!(
            fs::metadata(&ro_copy).unwrap().permissions().mode() & 0o777,
            0o555
        );
        assert_eq!(fs::read(ro_copy.join("inside")).unwrap(), b"x");
        assert_eq!(
            fs::read_link(dst.join("dangling")).unwrap(),
            Path::new("/nonexistent/target")
        );
        assert_same_tree(&src, &dst);
        // Let the temp dir be removed.
        for d in [&ro, &ro_copy] {
            fs::set_permissions(d, fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    #[test]
    fn merged_folders_keep_their_own_mode() {
        let t = tempfile::tempdir().unwrap();
        let (src, dst) = (t.path().join("src"), t.path().join("dst"));
        fs::create_dir_all(src.join("d")).unwrap();
        fs::write(src.join("d/f"), b"x").unwrap();
        fs::set_permissions(src.join("d"), fs::Permissions::from_mode(0o755)).unwrap();
        fs::create_dir_all(dst.join("d")).unwrap();
        fs::set_permissions(dst.join("d"), fs::Permissions::from_mode(0o700)).unwrap();
        assert_clean(&copy(
            &src,
            &dst,
            ConflictPolicy::Skip,
            Overrides::default(),
        ));
        assert_eq!(fs::read(dst.join("d/f")).unwrap(), b"x");
        let mode = fs::metadata(dst.join("d")).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700, "an existing folder is not changed by a merge");
    }

    #[test]
    fn sparse_files_stay_sparse() {
        let t = tempfile::tempdir().unwrap();
        let src = t.path().join("sparse.img");
        let f = fs::File::create(&src).unwrap();
        f.set_len(32 << 20).unwrap();
        std::os::unix::fs::FileExt::write_all_at(&f, &bytes(3, 4096), 16 << 20).unwrap();
        drop(f);
        let src_blocks = fs::metadata(&src).unwrap().blocks();
        let dst = t.path().join("copy.img");
        for method in [None, Some(Method::ReadWrite)] {
            let _ = fs::remove_file(&dst);
            let overrides = Overrides {
                method,
                ..Overrides::default()
            };
            assert_clean(&copy(&src, &dst, ConflictPolicy::Skip, overrides));
            assert_eq!(fs::read(&src).unwrap(), fs::read(&dst).unwrap());
            if src_blocks * 512 < 32 << 20 {
                // The filesystem decides how much of the source is allocated (APFS keeps more than
                // the written block); the copy must not allocate more than that, plus one MiB.
                let blocks = fs::metadata(&dst).unwrap().blocks();
                assert!(
                    blocks <= src_blocks + 2048,
                    "{blocks} blocks vs {src_blocks} in the source: holes were filled ({method:?})"
                );
            }
        }
    }

    #[test]
    fn overwrite_failure_keeps_the_old_file() {
        if is_root() {
            return; // root ignores the read-only folder that makes the overwrite fail.
        }
        let t = tempfile::tempdir().unwrap();
        let (src, dir) = (t.path().join("a"), t.path().join("dir"));
        fs::write(&src, b"new").unwrap();
        fs::create_dir(&dir).unwrap();
        fs::write(dir.join("a"), b"old").unwrap();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o555)).unwrap();
        let s = copy(
            &src,
            &dir.join("a"),
            ConflictPolicy::Overwrite,
            Overrides::default(),
        );
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(s.error_count(), 1);
        assert_eq!(fs::read(dir.join("a")).unwrap(), b"old");
        assert!(no_part_files(&dir));
    }

    #[test]
    fn move_keeps_the_whole_source_on_any_error() {
        if is_root() {
            return; // root can read the unreadable file.
        }
        let t = tempfile::tempdir().unwrap();
        let src = t.path().join("src");
        small_tree(&src, 2, 4);
        let secret = src.join("d00/sub0/f2.bin");
        fs::set_permissions(&secret, fs::Permissions::from_mode(0o000)).unwrap();
        let forced = Overrides {
            method: Some(Method::ReadWrite),
            ..Overrides::default()
        };
        let s = run(&request(
            vec![item(&src, &t.path().join("dst"))],
            Op::Move,
            ConflictPolicy::Skip,
            forced,
        ));
        fs::set_permissions(&secret, fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(s.error_count(), 1, "{:?}", s.errors);
        assert_eq!(s.kept_sources, vec![src.clone()]);
        assert!(src.join("top.txt").is_file() && secret.is_file());
    }

    #[test]
    fn fifos_are_recreated() {
        let t = tempfile::tempdir().unwrap();
        let src = t.path().join("src");
        fs::create_dir(&src).unwrap();
        let fifo = src.join("pipe");
        if rustix_mkfifo(&fifo).is_err() {
            return;
        }
        let s = copy(
            &src,
            &t.path().join("dst"),
            ConflictPolicy::Skip,
            Overrides::default(),
        );
        assert_clean(&s);
        use std::os::unix::fs::FileTypeExt;
        assert!(
            fs::symlink_metadata(t.path().join("dst/pipe"))
                .unwrap()
                .file_type()
                .is_fifo()
        );
    }

    #[cfg(target_os = "linux")]
    fn rustix_mkfifo(p: &Path) -> std::io::Result<()> {
        use rustix::fs::{CWD, FileType, Mode, mknodat};
        Ok(mknodat(
            CWD,
            p,
            FileType::Fifo,
            Mode::from_raw_mode(0o644),
            0,
        )?)
    }

    #[cfg(not(target_os = "linux"))]
    fn rustix_mkfifo(p: &Path) -> std::io::Result<()> {
        let status = std::process::Command::new("mkfifo").arg(p).status()?;
        status
            .success()
            .then_some(())
            .ok_or_else(|| std::io::Error::other("mkfifo failed"))
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn user_xattrs_are_copied() {
        use rustix::fs::{XattrFlags, getxattr, setxattr};
        let t = tempfile::tempdir().unwrap();
        let src = t.path().join("x");
        fs::write(&src, b"x").unwrap();
        if setxattr(&src, "user.fx", b"value", XattrFlags::empty()).is_err() {
            return; // the temp filesystem has no user xattrs
        }
        let dst = t.path().join("y");
        assert_clean(&copy(
            &src,
            &dst,
            ConflictPolicy::Skip,
            Overrides::default(),
        ));
        let mut buf = [0u8; 16];
        let n = getxattr(&dst, "user.fx", &mut buf[..]).unwrap();
        assert_eq!(&buf[..n], b"value");
    }

    #[test]
    fn many_items_into_one_folder() {
        let t = tempfile::tempdir().unwrap();
        let (a, b, out) = (t.path().join("a"), t.path().join("b"), t.path().join("out"));
        small_tree(&a, 2, 3);
        fs::write(&b, b"b").unwrap();
        fs::create_dir(&out).unwrap();
        let req = Request::into_dir(
            &[a.clone(), b.clone()],
            &out,
            Op::Copy,
            ConflictPolicy::Skip,
        );
        let s = run(&req);
        assert_clean(&s);
        assert_eq!((s.items, s.items_done), (2, 2));
        assert_same_tree(&a, &out.join("a"));
        assert_eq!(fs::read(out.join("b")).unwrap(), b"b");
    }
}

/// Every experimental method must leave the same tree as the default path: content, mtimes and modes.
/// Methods that do not apply to the host or to the file sizes fall back, so this holds everywhere.
#[test]
fn experimental_methods_copy_like_the_default_path() {
    let t = tempfile::tempdir().unwrap();
    let src = t.path().join("src");
    small_tree(&src, 3, 12);
    fs::write(src.join("mid.bin"), bytes(3, 300_000)).unwrap();
    fs::write(src.join("big.bin"), bytes(9, 17 << 20)).unwrap();
    let read_only = src.join("ro.txt");
    fs::write(&read_only, b"read only").unwrap();
    let mut perms = fs::metadata(&read_only).unwrap().permissions();
    perms.set_readonly(true);
    fs::set_permissions(&read_only, perms).unwrap();

    let default = t.path().join("default");
    assert_clean(&copy(
        &src,
        &default,
        ConflictPolicy::Skip,
        Overrides::default(),
    ));
    for method in Method::EXPERIMENTAL {
        let dst = t.path().join(method.name());
        let overrides = Overrides {
            experiment: Some(method),
            workers: Some(3),
            ..Overrides::default()
        };
        let s = copy(&src, &dst, ConflictPolicy::Skip, overrides);
        assert_clean(&s);
        assert_same_tree(&src, &dst);
        assert_same_tree(&default, &dst);
        assert!(
            fs::metadata(dst.join("ro.txt"))
                .unwrap()
                .permissions()
                .readonly(),
            "{}: read-only bit",
            method.name()
        );
    }
}

#[test]
fn verified_copies_are_complete_and_report_no_errors() {
    for schedule in [
        Schedule::Streaming,
        Schedule::TwoPhase {
            order: Order::Inode,
        },
    ] {
        let t = tempfile::tempdir().unwrap();
        let src = t.path().join("src");
        small_tree(&src, 3, 12);
        fs::write(src.join("big.bin"), bytes(5, 17 << 20)).unwrap();
        let dst = t.path().join("dst");
        let overrides = Overrides {
            verify: true,
            schedule: Some(schedule),
            ..Overrides::default()
        };
        let s = copy(&src, &dst, ConflictPolicy::Skip, overrides);
        assert_clean(&s);
        assert_eq!(s.error_count(), 0);
        assert_same_tree(&src, &dst);
        assert!(s.plans.iter().all(|p| p.verify));
    }
}

#[cfg(windows)]
mod windows {
    use super::*;
    use std::io::Write;
    use std::process::Command;

    /// Whether `a` and `b` are names of one file: a write through one shows in the other.
    fn same_file(a: &Path, b: &Path) -> bool {
        let before = fs::read(b).unwrap();
        let mut f = fs::OpenOptions::new().append(true).open(a).unwrap();
        f.write_all(b"+linked").unwrap();
        drop(f);
        let linked = fs::read(b).unwrap() != before;
        fs::write(a, &before).unwrap();
        linked
    }

    #[test]
    fn hardlink_groups_survive_copy_and_move() {
        for (op, forced) in [
            (Op::Copy, Overrides::default()),
            // A forced method skips the same-volume rename, so the move copies and deletes.
            (
                Op::Move,
                Overrides {
                    method: Some(Method::ReadWrite),
                    ..Overrides::default()
                },
            ),
        ] {
            let t = tempfile::tempdir().unwrap();
            let src = t.path().join("src");
            fs::create_dir_all(src.join("sub")).unwrap();
            fs::write(src.join("a.txt"), b"shared").unwrap();
            fs::hard_link(src.join("a.txt"), src.join("sub/b.txt")).unwrap();
            fs::hard_link(src.join("a.txt"), src.join("c.txt")).unwrap();
            fs::write(src.join("alone.txt"), b"shared").unwrap();
            fs::write(src.join("other.txt"), b"other").unwrap();
            fs::hard_link(src.join("other.txt"), src.join("other2.txt")).unwrap();

            let dst = t.path().join("dst");
            let s = run(&request(
                vec![item(&src, &dst)],
                op,
                ConflictPolicy::Skip,
                forced,
            ));
            assert_clean(&s);
            assert_eq!(s.error_count(), 0, "{:#?}", s.errors);
            assert!(
                same_file(&dst.join("a.txt"), &dst.join("sub/b.txt")),
                "{op:?}"
            );
            assert!(same_file(&dst.join("a.txt"), &dst.join("c.txt")), "{op:?}");
            assert!(same_file(&dst.join("other.txt"), &dst.join("other2.txt")));
            // Same content, but never linked to the group.
            assert!(
                !same_file(&dst.join("a.txt"), &dst.join("alone.txt")),
                "{op:?}"
            );
            assert_eq!(src.exists(), op == Op::Copy);
        }
    }

    /// The `dir /AL` line of the reparse point `name` in `parent`: `<JUNCTION> name [target]`.
    fn listed_link(parent: &Path, name: &str) -> String {
        let out = Command::new("cmd")
            .args(["/C", "dir", "/AL"])
            .arg(parent)
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .find(|l| l.contains(&format!(" {name} [")))
            .unwrap_or_else(|| panic!("no link {name} in {}", parent.display()))
            .to_lowercase()
    }

    fn junction(link: &Path, target: &Path) {
        let status = Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .output()
            .unwrap()
            .status;
        assert!(status.success(), "mklink /J failed");
    }

    #[test]
    fn junctions_are_recreated_with_targets_inside_the_tree_rewritten() {
        for op in [Op::Copy, Op::Move] {
            let t = tempfile::tempdir().unwrap();
            let root = fs::canonicalize(t.path()).unwrap();
            let root = root
                .to_string_lossy()
                .trim_start_matches(r"\\?\")
                .to_string();
            let root = Path::new(&root);
            let (src, other, dst) = (root.join("src"), root.join("other"), root.join("dst"));
            fs::create_dir_all(src.join("real")).unwrap();
            fs::create_dir_all(&other).unwrap();
            fs::write(src.join("real/file.txt"), b"data").unwrap();
            junction(&src.join("inside"), &src.join("real"));
            junction(&src.join("outside"), &other);

            let forced = Overrides {
                method: Some(Method::ReadWrite),
                ..Overrides::default()
            };
            let s = run(&request(
                vec![item(&src, &dst)],
                op,
                ConflictPolicy::Skip,
                forced,
            ));
            assert_clean(&s);
            assert_eq!(s.error_count(), 0, "{:#?}", s.errors);

            let inside = listed_link(&dst, "inside");
            assert!(inside.contains("<junction>"), "{inside}");
            let moved_real = dst.join("real").to_string_lossy().to_lowercase();
            assert!(inside.contains(&format!("[{moved_real}]")), "{inside}");
            let outside = listed_link(&dst, "outside");
            assert!(outside.contains("<junction>"), "{outside}");
            let other = other.to_string_lossy().to_lowercase();
            assert!(outside.contains(&format!("[{other}]")), "{outside}");
            assert_eq!(fs::read(dst.join("inside/file.txt")).unwrap(), b"data");
            // A move deletes the source only when every entry, rewritten junction included, matched.
            assert_eq!(src.exists(), op == Op::Copy, "{op:?}: {:?}", s.kept_sources);
        }
    }
}
