//! `search` must agree with a naive recursive std::fs filter.

use fx_core::{
    Backend, EntryKind, GlobScope, SearchEvent, SearchOptions, SearchQuery, WalkControl,
    WalkOptions, search,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

fn build_tree(root: &Path) {
    for name in [
        "a/b/Report.TXT",
        "a/report-final.md",
        "a/b/c/abc",
        "a/b/c/axc.rs",
        "top.rs",
        "日本語/メモ.txt",
        "empty/.keep",
    ] {
        let p = root.join(name);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, b"x").unwrap();
    }
    fs::create_dir_all(root.join("reports")).unwrap();
}

/// Windows hides by attribute, not by leading dot; ancestors are covered by the dot-dir rule.
fn hide_on_windows(path: &Path) {
    if cfg!(windows) {
        let status = std::process::Command::new("attrib")
            .arg("+H")
            .arg(path)
            .status()
            .unwrap();
        assert!(status.success());
    }
}

type Keep = Box<dyn Fn(&str, &str) -> bool>;

fn kind_of(ft: fs::FileType) -> EntryKind {
    if ft.is_dir() {
        EntryKind::Dir
    } else if ft.is_file() {
        EntryKind::File
    } else if ft.is_symlink() {
        EntryKind::Symlink
    } else {
        EntryKind::Other
    }
}

/// Walks with std::fs only; `keep(lowercase name, lowercase relative path)` is the expected semantics.
fn naive(
    dir: &Path,
    root: &Path,
    keep: &dyn Fn(&str, &str) -> bool,
    out: &mut Vec<(PathBuf, EntryKind)>,
) {
    for entry in fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        let kind = kind_of(entry.file_type().unwrap());
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_lowercase();
        let rel = path.strip_prefix(root).unwrap();
        let rel = rel.to_string_lossy().replace('\\', "/").to_lowercase();
        if keep(&name, &rel) {
            out.push((path.clone(), kind));
        }
        if kind == EntryKind::Dir {
            naive(&path, root, keep, out);
        }
    }
}

fn found(root: &Path, query: &SearchQuery, backend: Backend) -> Vec<(PathBuf, EntryKind)> {
    let hits = Mutex::new(Vec::new());
    let opts = WalkOptions {
        backend,
        storage: None,
    };
    search(root, query, &opts, &WalkControl::default(), |event| {
        if let SearchEvent::Hits(batch) = event {
            hits.lock()
                .unwrap()
                .extend(batch.into_iter().map(|h| (h.path, h.kind)));
        }
    })
    .unwrap();
    let mut hits = hits.into_inner().unwrap();
    hits.sort_by(|a, b| a.0.cmp(&b.0));
    hits
}

#[test]
fn search_equals_naive_filter() {
    let tmp = tempfile::tempdir().unwrap();
    build_tree(tmp.path());
    let root = tmp.path();
    let cases: Vec<(&str, GlobScope, Keep)> = vec![
        (
            "report",
            GlobScope::Name,
            Box::new(|n, _| n.contains("report")),
        ),
        (
            "REPORT",
            GlobScope::Name,
            Box::new(|n, _| n.contains("report")),
        ),
        ("*.rs", GlobScope::Name, Box::new(|n, _| n.ends_with(".rs"))),
        (
            "a?c*",
            GlobScope::Name,
            Box::new(|n, _| n.starts_with('a') && n.chars().nth(2) == Some('c')),
        ),
        (
            "a/*.md",
            GlobScope::RelativePath,
            Box::new(|_, r| r.starts_with("a/") && r.ends_with(".md")),
        ),
        (
            "*.txt",
            GlobScope::Name,
            Box::new(|n, _| n.ends_with(".txt")),
        ),
        ("nomatch", GlobScope::Name, Box::new(|_, _| false)),
        ("", GlobScope::Name, Box::new(|_, _| true)),
    ];
    for (text, scope, keep) in &cases {
        let query = SearchQuery::new(
            text,
            SearchOptions {
                scope: *scope,
                ..SearchOptions::default()
            },
        );
        let mut expected = Vec::new();
        naive(root, root, keep.as_ref(), &mut expected);
        expected.sort_by(|a, b| a.0.cmp(&b.0));
        for backend in [Backend::Native, Backend::Std] {
            assert_eq!(
                found(root, &query, backend),
                expected,
                "{text:?} {backend:?}"
            );
        }
    }
}

#[test]
fn hidden_flag_covers_dot_entries_and_dot_ancestors() {
    let tmp = tempfile::tempdir().unwrap();
    for name in [".git/config", "src/.env", "src/main.rs"] {
        let p = tmp.path().join(name);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, b"x").unwrap();
    }
    hide_on_windows(&tmp.path().join(".git"));
    hide_on_windows(&tmp.path().join("src").join(".env"));
    let hits = Mutex::new(Vec::new());
    let control = WalkControl::default();
    let query = SearchQuery::new("", SearchOptions::default());
    search(
        tmp.path(),
        &query,
        &WalkOptions::default(),
        &control,
        |event| {
            if let SearchEvent::Hits(batch) = event {
                hits.lock().unwrap().extend(batch);
            }
        },
    )
    .unwrap();
    let mut hidden: Vec<String> = hits
        .into_inner()
        .unwrap()
        .into_iter()
        .filter(|h| h.hidden)
        .map(|h| {
            h.path
                .strip_prefix(tmp.path())
                .unwrap()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    hidden.sort();
    assert_eq!(
        hidden,
        [".git", ".git/config", "src/.env"].map(|p| p.replace('/', std::path::MAIN_SEPARATOR_STR))
    );
    assert_eq!(control.dirs_read(), 3);
}

fn names_found(root: &Path, query: &SearchQuery) -> Vec<String> {
    let mut names: Vec<String> = found(root, query, Backend::Native)
        .into_iter()
        .map(|(p, _)| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn case_sensitive_option_applies_to_substrings_and_globs() {
    let tmp = tempfile::tempdir().unwrap();
    for name in ["Report.TXT", "report.txt"] {
        fs::write(tmp.path().join(name), b"x").unwrap();
    }
    let exact = SearchOptions {
        case_sensitive: true,
        ..SearchOptions::default()
    };
    let query = |text| SearchQuery::new(text, exact);
    assert_eq!(names_found(tmp.path(), &query("Report")), ["Report.TXT"]);
    assert_eq!(names_found(tmp.path(), &query("*.txt")), ["report.txt"]);
    assert_eq!(names_found(tmp.path(), &query("report")), ["report.txt"]);
    let folded = SearchQuery::new("report", SearchOptions::default());
    assert_eq!(names_found(tmp.path(), &folded).len(), 2);
}

#[test]
fn cancelling_mid_walk_stops_the_search_and_silences_the_callback() {
    let tmp = tempfile::tempdir().unwrap();
    for i in 0..300 {
        let dir = tmp.path().join(format!("d{i}/sub"));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("hit.txt"), b"x").unwrap();
    }
    let control = WalkControl::default();
    let events = AtomicUsize::new(0);
    let query = SearchQuery::new("hit", SearchOptions::default());
    search(
        tmp.path(),
        &query,
        &WalkOptions::default(),
        &control,
        |_| {
            events.fetch_add(1, Ordering::Relaxed);
            control.cancel();
        },
    )
    .unwrap();
    let at_return = events.load(Ordering::Relaxed);
    assert!(control.is_cancelled());
    assert!(
        control.dirs_read() < 601,
        "walk read {} dirs",
        control.dirs_read()
    );
    std::thread::sleep(std::time::Duration::from_millis(50));
    assert_eq!(events.load(Ordering::Relaxed), at_return);
}

#[cfg(unix)]
#[test]
fn unreadable_folder_is_reported_with_its_path() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::tempdir().unwrap();
    let locked = tmp.path().join("locked");
    fs::create_dir(&locked).unwrap();
    fs::write(tmp.path().join("ok.txt"), b"x").unwrap();
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
    let restore = || fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
    if fs::read_dir(&locked).is_ok() {
        restore();
        return; // running as root: permissions are not enforced
    }
    let reported = Mutex::new(Vec::new());
    let control = WalkControl::default();
    let query = SearchQuery::new("", SearchOptions::default());
    let result = search(
        tmp.path(),
        &query,
        &WalkOptions::default(),
        &control,
        |event| {
            if let SearchEvent::Unreadable(folder) = event {
                reported.lock().unwrap().push(folder);
            }
        },
    );
    restore();
    result.unwrap();
    let reported = reported.into_inner().unwrap();
    assert_eq!(reported.len(), 1);
    assert_eq!(reported[0].path, locked);
    assert!(!reported[0].error.is_empty());
    assert_eq!(control.unreadable(), 1);
}
