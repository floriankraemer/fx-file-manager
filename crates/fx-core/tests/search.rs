//! `search` must agree with a naive recursive std::fs filter.

use fx_core::{Backend, EntryKind, GlobScope, SearchQuery, WalkOptions, WalkProgress, search};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;

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
    search(
        root,
        query,
        &opts,
        &AtomicBool::new(false),
        &WalkProgress::default(),
        |batch| {
            hits.lock()
                .unwrap()
                .extend(batch.into_iter().map(|h| (h.path, h.kind)));
        },
    )
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
        let query = SearchQuery::new(text, *scope);
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
    let hits = Mutex::new(Vec::new());
    let progress = WalkProgress::default();
    let query = SearchQuery::new("", GlobScope::Name);
    search(
        tmp.path(),
        &query,
        &WalkOptions::default(),
        &AtomicBool::new(false),
        &progress,
        |batch| hits.lock().unwrap().extend(batch),
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
    assert_eq!(
        progress
            .dirs_read
            .load(std::sync::atomic::Ordering::Relaxed),
        3
    );
}
