use super::loading::list_view;
use super::preload::PRELOAD_MAX;
use super::*;
use crate::state::{Bookmark, WindowState};
use crate::tab::LoadState;
use crate::tree::Children;
use crate::view::SortColumn;
use fx_core::{EntryKind, Resolution};
use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::Ordering;
use std::thread;
use std::time::Duration;

/// Makes a dotfile hidden on Windows too, where only the hidden attribute counts.
fn hide(path: &Path) {
    if cfg!(windows) {
        let ok = Command::new("attrib").arg("+h").arg(path).status().unwrap();
        assert!(ok.success(), "attrib +h {}", path.display());
    }
}

/// Waits until no file operation runs and no tab is loading.
fn settle(explorer: &mut Explorer) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while explorer.busy()
        || explorer.windows.iter().any(|w| {
            w.tree
                .children
                .values()
                .any(|c| matches!(c, Children::Loading))
                || w.tabs
                    .iter()
                    .any(|tab| matches!(tab.state(), LoadState::Loading))
        })
    {
        assert!(Instant::now() < deadline, "work did not finish in time");
        explorer.poll();
        thread::sleep(Duration::from_millis(1));
    }
}

fn names(explorer: &Explorer) -> Vec<String> {
    let tab = explorer.active();
    let LoadState::Loaded(view) = tab.state() else {
        panic!("tab is not loaded: {:?}", tab.state());
    };
    tab.order()
        .iter()
        .map(|&i| {
            view.listing
                .name(&view.listing.entries[i as usize])
                .to_string_lossy()
                .into_owned()
        })
        .collect()
}

fn row_of(explorer: &Explorer, name: &str) -> usize {
    names(explorer)
        .iter()
        .position(|n| n == name)
        .unwrap_or_else(|| panic!("no row named {name}"))
}

fn selected_names(explorer: &Explorer) -> Vec<String> {
    let all = names(explorer);
    explorer
        .active()
        .selection()
        .into_iter()
        .map(|row| all[row].clone())
        .collect()
}

fn last_notice(explorer: &mut Explorer) -> Notice {
    let mut last = None;
    while let Some(notice) = explorer.take_notice() {
        last = Some(notice);
    }
    last.expect("expected a notice")
}

fn explorer_in(root: &Path) -> Explorer {
    let mut explorer = Explorer::new(root.to_path_buf(), || {});
    settle(&mut explorer);
    explorer
}

/// Waits until every tab that has started loading is done; unloaded restored tabs stay `Loading` forever.
fn settle_started(explorer: &mut Explorer) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while explorer.windows.iter().any(|w| {
        w.tabs
            .iter()
            .any(|t| t.generation() > 0 && matches!(t.state(), LoadState::Loading))
    }) {
        assert!(Instant::now() < deadline, "work did not finish in time");
        explorer.poll();
        thread::sleep(Duration::from_millis(1));
    }
}

/// Waits until the bookmark checks have reported.
fn wait_bookmarks(explorer: &mut Explorer) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while (0..explorer.bookmarks().len()).any(|i| explorer.bookmark_available(i).is_none()) {
        assert!(Instant::now() < deadline, "bookmark check did not finish");
        explorer.poll();
        thread::sleep(Duration::from_millis(1));
    }
}

fn with_state_in(
    dir: &Path,
    start: Option<PathBuf>,
    fallback: &Path,
    settings: Settings,
    state: State,
) -> Explorer {
    let mut explorer = Explorer::with_state(
        start,
        fallback.to_path_buf(),
        settings,
        state,
        Some(dir.join("state.toml")),
        || {},
    );
    settle_started(&mut explorer);
    explorer
}

fn saved(tabs: &[&Path], active_tab: usize) -> WindowState {
    WindowState {
        tabs: tabs.iter().map(|p| p.to_path_buf()).collect(),
        active_tab,
    }
}

fn mkdirs(root: &Path, names: &[&str]) -> Vec<PathBuf> {
    names
        .iter()
        .map(|n| {
            let p = root.join(n);
            fs::create_dir(&p).unwrap();
            p
        })
        .collect()
}

#[test]
fn bookmark_add_remove_rename_move_and_persist() {
    let dir = tempfile::tempdir().unwrap();
    let folders = mkdirs(dir.path(), &["a", "b", "c"]);
    let mut explorer = with_state_in(
        dir.path(),
        None,
        dir.path(),
        Settings::default(),
        State::default(),
    );
    for f in &folders {
        explorer.add_bookmark(f.clone());
    }
    explorer.add_bookmark(folders[0].clone());
    let labels = |e: &Explorer| {
        e.bookmarks()
            .iter()
            .map(|b| b.label.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(labels(&explorer), ["a", "b", "c"], "duplicate is a no-op");
    assert!(explorer.is_bookmarked(&folders[1]));

    explorer.rename_bookmark(1, "  Bee  ");
    explorer.rename_bookmark(0, "   ");
    explorer.move_bookmark(2, 0);
    assert_eq!(labels(&explorer), ["c", "a", "Bee"]);
    explorer.remove_bookmark(1);
    explorer.remove_bookmark(9);
    assert_eq!(labels(&explorer), ["c", "Bee"]);
    assert!(!explorer.is_bookmarked(&folders[0]));

    let on_disk = State::load_from(&dir.path().join("state.toml"));
    assert_eq!(on_disk.bookmarks, explorer.bookmarks());
}

#[test]
fn toggle_bookmarks_the_active_folder() {
    let dir = tempfile::tempdir().unwrap();
    let mut explorer = explorer_in(dir.path());
    explorer.toggle_bookmark_active();
    assert!(explorer.is_bookmarked(dir.path()));
    explorer.toggle_bookmark_active();
    assert!(explorer.bookmarks().is_empty());
}

#[test]
fn toggle_bookmarks_any_folder() {
    let dir = tempfile::tempdir().unwrap();
    let folders = mkdirs(dir.path(), &["sub"]);
    let mut explorer = explorer_in(dir.path());
    explorer.toggle_bookmark(folders[0].clone());
    assert!(explorer.is_bookmarked(&folders[0]));
    assert!(!explorer.is_bookmarked(dir.path()));
    explorer.toggle_bookmark(folders[0].clone());
    assert!(explorer.bookmarks().is_empty());
}

#[test]
fn bookmark_availability_is_checked_in_the_background() {
    let dir = tempfile::tempdir().unwrap();
    let folders = mkdirs(dir.path(), &["here"]);
    let state = State {
        bookmarks: vec![
            Bookmark {
                label: "here".to_owned(),
                path: folders[0].clone(),
            },
            Bookmark {
                label: "gone".to_owned(),
                path: dir.path().join("gone"),
            },
        ],
        windows: Vec::new(),
    };
    let mut explorer = with_state_in(dir.path(), None, dir.path(), Settings::default(), state);
    wait_bookmarks(&mut explorer);
    assert_eq!(explorer.bookmark_available(0), Some(true));
    assert_eq!(explorer.bookmark_available(1), Some(false));
    fs::create_dir(dir.path().join("gone")).unwrap();
    explorer.check_bookmarks();
    let deadline = Instant::now() + Duration::from_secs(10);
    while explorer.bookmark_available(1) != Some(true) {
        assert!(Instant::now() < deadline);
        explorer.poll();
        thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn session_restores_surviving_tabs_and_clamps_active() {
    let dir = tempfile::tempdir().unwrap();
    let f = mkdirs(dir.path(), &["a", "b"]);
    let gone = dir.path().join("gone");
    let state = State {
        bookmarks: Vec::new(),
        windows: vec![saved(&[&f[0], &gone, &f[1]], 1)],
    };
    let explorer = with_state_in(
        dir.path(),
        None,
        dir.path(),
        Settings::default(),
        state.clone(),
    );
    let paths: Vec<_> = explorer
        .tabs()
        .iter()
        .map(|t| t.path().to_path_buf())
        .collect();
    assert_eq!(paths, [f[0].clone(), f[1].clone()]);
    assert_eq!(explorer.active().path(), f[1], "next survivor takes over");

    let far = State {
        windows: vec![saved(&[&f[0], &f[1]], 99)],
        ..State::default()
    };
    let explorer = with_state_in(dir.path(), None, dir.path(), Settings::default(), far);
    assert_eq!(explorer.active_index(), 1);
}

#[test]
fn cli_path_wins_over_the_session() {
    let dir = tempfile::tempdir().unwrap();
    let f = mkdirs(dir.path(), &["a", "cli"]);
    let state = State {
        bookmarks: Vec::new(),
        windows: vec![saved(&[&f[0]], 0)],
    };
    let explorer = with_state_in(
        dir.path(),
        Some(f[1].clone()),
        dir.path(),
        Settings::default(),
        state,
    );
    assert_eq!(explorer.tabs().len(), 1);
    assert_eq!(explorer.active().path(), f[1]);
    assert_eq!(explorer.window_ids().len(), 1);
}

#[test]
fn restore_session_off_opens_the_fallback_and_never_writes_windows() {
    let dir = tempfile::tempdir().unwrap();
    let f = mkdirs(dir.path(), &["a"]);
    let state = State {
        bookmarks: vec![Bookmark {
            label: "a".to_owned(),
            path: f[0].clone(),
        }],
        windows: vec![saved(&[&f[0]], 0)],
    };
    let settings = Settings {
        restore_session: false,
        ..Settings::default()
    };
    let mut explorer = with_state_in(dir.path(), None, dir.path(), settings, state);
    assert_eq!(explorer.active().path(), dir.path());
    explorer.new_tab(f[0].clone());
    explorer.save_session();
    let on_disk = State::load_from(&dir.path().join("state.toml"));
    assert!(on_disk.windows.is_empty());
    assert_eq!(on_disk.bookmarks.len(), 1, "bookmarks are still saved");
}

#[test]
fn fallback_when_no_saved_tab_survives() {
    let dir = tempfile::tempdir().unwrap();
    let state = State {
        bookmarks: Vec::new(),
        windows: vec![saved(&[&dir.path().join("x")], 0), WindowState::default()],
    };
    let explorer = with_state_in(dir.path(), None, dir.path(), Settings::default(), state);
    assert_eq!(explorer.window_ids().len(), 1);
    assert_eq!(explorer.active().path(), dir.path());
}

#[test]
fn multi_window_session_round_trips_through_save_session() {
    let dir = tempfile::tempdir().unwrap();
    let f = mkdirs(dir.path(), &["a", "b", "c"]);
    let mut explorer = explorer_in(&f[0]);
    explorer.new_tab(f[1].clone());
    explorer.set_active(0);
    let second = explorer.open_window(f[2].clone());
    settle_started(&mut explorer);
    explorer.state_path = Some(dir.path().join("state.toml"));
    explorer.save_session();

    let state = State::load_from(&dir.path().join("state.toml"));
    assert_eq!(state.windows.len(), 2);
    let mut restored = with_state_in(dir.path(), None, dir.path(), Settings::default(), state);
    let ids = restored.window_ids();
    assert_eq!(ids.len(), 2);
    assert_eq!(restored.tabs().len(), 2);
    assert_eq!(restored.active().path(), f[0]);
    restored.set_window(ids[1]);
    assert_eq!(restored.active().path(), f[2]);
    let _ = second;
}

#[test]
fn inactive_restored_tabs_load_when_first_activated() {
    let dir = tempfile::tempdir().unwrap();
    let f = mkdirs(dir.path(), &["a", "b", "c"]);
    fs::write(f[1].join("file.txt"), b"x").unwrap();
    let state = State {
        bookmarks: Vec::new(),
        windows: vec![saved(&[&f[0], &f[1], &f[2]], 0)],
    };
    let mut explorer = with_state_in(dir.path(), None, dir.path(), Settings::default(), state);
    assert!(matches!(explorer.tabs()[0].state(), LoadState::Loaded(_)));
    for i in [1, 2] {
        assert!(matches!(explorer.tabs()[i].state(), LoadState::Loading));
        assert_eq!(explorer.tabs()[i].generation(), 0, "no load in flight");
    }
    // Housekeeping must leave an unloaded tab alone.
    explorer.revalidate();
    explorer.poll();
    explorer.set_active(1);
    settle_started(&mut explorer);
    assert_eq!(names(&explorer), ["file.txt"]);
    assert_eq!(explorer.tabs()[2].generation(), 0);
    // Closing the active tab activates a neighbour, which must then load too.
    explorer.close_tab(1);
    settle_started(&mut explorer);
    assert_eq!(explorer.active().path(), f[2]);
    assert!(matches!(explorer.active().state(), LoadState::Loaded(_)));
}

#[test]
fn dirty_session_is_written_by_poll() {
    let dir = tempfile::tempdir().unwrap();
    let f = mkdirs(dir.path(), &["a"]);
    let mut explorer = with_state_in(
        dir.path(),
        None,
        dir.path(),
        Settings::default(),
        State::default(),
    );
    explorer.new_tab(f[0].clone());
    explorer.poll();
    let on_disk = State::load_from(&dir.path().join("state.toml"));
    assert_eq!(on_disk.windows[0].tabs.len(), 2);
    assert_eq!(on_disk.windows[0].active_tab, 1);
}

#[test]
fn windows_have_their_own_tabs_and_open_keeps_the_current_one() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir(root.join("other")).unwrap();
    let mut explorer = explorer_in(root);
    let first = explorer.window();

    let second = explorer.open_window(root.join("other"));
    assert_eq!(explorer.window(), first, "open does not switch");
    assert_eq!(explorer.window_ids(), vec![first, second]);
    assert_eq!(explorer.active().path(), root);

    explorer.set_window(second);
    assert_eq!(explorer.tabs().len(), 1);
    assert_eq!(explorer.active().path(), root.join("other"));
    explorer.new_tab(root.to_path_buf());
    assert_eq!(explorer.tabs().len(), 2);

    explorer.set_window(999);
    assert_eq!(explorer.window(), second, "unknown id is ignored");
    explorer.set_window(first);
    assert_eq!(explorer.tabs().len(), 1, "tabs are per window");
    let ids: Vec<u64> = explorer
        .windows
        .iter()
        .flat_map(|w| w.tabs.iter().map(Tab::id))
        .collect();
    assert_eq!(ids.len(), 3);
    assert!(ids.iter().enumerate().all(|(i, id)| !ids[..i].contains(id)));
}

#[test]
fn clipboard_is_shared_between_windows() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::write(root.join("f.txt"), b"x").unwrap();
    fs::create_dir(root.join("dst")).unwrap();
    let mut explorer = explorer_in(root);
    let a = explorer.window();
    let b = explorer.open_window(root.join("dst"));
    settle(&mut explorer);

    explorer.copy_to_clipboard(&[row_of(&explorer, "f.txt")], false);
    explorer.set_window(b);
    assert!(explorer.can_paste());
    explorer.paste(None);
    settle(&mut explorer);
    assert_eq!(fs::read(root.join("dst/f.txt")).unwrap(), b"x");
    explorer.set_window(a);
    assert!(explorer.can_paste(), "a copy clipboard can be pasted again");
}

#[test]
fn the_last_window_is_never_closed_and_current_stays_valid() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let mut explorer = explorer_in(root);
    let a = explorer.window();
    let b = explorer.open_window(root.to_path_buf());
    let c = explorer.open_window(root.to_path_buf());

    explorer.set_window(c);
    explorer.close_window(a);
    assert_eq!(explorer.window_ids(), vec![b, c]);
    assert_eq!(explorer.window(), c, "current follows its window");

    explorer.close_window(c);
    assert_eq!(explorer.window_ids(), vec![b]);
    assert_eq!(explorer.window(), b);

    explorer.close_window(999);
    explorer.close_window(b);
    assert_eq!(explorer.window_ids(), vec![b], "the last window stays");
    assert_eq!(explorer.tabs().len(), 1);
}

#[test]
fn loads_complete_in_a_window_that_is_not_current() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir(root.join("other")).unwrap();
    fs::write(root.join("other/in-other.txt"), b"x").unwrap();
    let mut explorer = explorer_in(root);
    let b = explorer.open_window(root.join("other"));
    settle(&mut explorer);

    let loaded = &explorer.windows[1].tabs[0];
    assert!(matches!(loaded.state(), LoadState::Loaded(_)));
    explorer.set_window(b);
    assert_eq!(names(&explorer), vec!["in-other.txt"]);
}

/// `zeta/`, `alpha/inner.md`, `big.bin` (2 KiB), `small.txt` and a hidden `.hidden`, already loaded.
fn sample_tree() -> (tempfile::TempDir, PathBuf, Explorer) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    fs::create_dir(root.join("zeta")).unwrap();
    fs::create_dir(root.join("alpha")).unwrap();
    fs::write(root.join("big.bin"), vec![0u8; 2048]).unwrap();
    fs::write(root.join("small.txt"), b"x").unwrap();
    fs::write(root.join(".hidden"), b"").unwrap();
    hide(&root.join(".hidden"));
    fs::write(root.join("alpha").join("inner.md"), b"").unwrap();

    let mut explorer = Explorer::new(root.clone(), || {});
    settle(&mut explorer);
    assert_eq!(names(&explorer), ["alpha", "zeta", "big.bin", "small.txt"]);
    (dir, root, explorer)
}

#[test]
fn sorts_by_size_both_ways() {
    let (_dir, _root, mut explorer) = sample_tree();
    explorer.active_mut().set_sort_column(SortColumn::Size);
    assert_eq!(names(&explorer)[2..], ["small.txt", "big.bin"]);
    explorer.active_mut().set_sort_column(SortColumn::Size);
    assert_eq!(names(&explorer)[2..], ["big.bin", "small.txt"]);
    explorer.active_mut().set_sort(crate::SortSpec {
        column: SortColumn::Size,
        ascending: true,
    });
    assert_eq!(names(&explorer)[2..], ["small.txt", "big.bin"]);
}

#[test]
fn navigates_into_back_forward_and_up() {
    let (_dir, root, mut explorer) = sample_tree();
    explorer.navigate(root.join("alpha"));
    settle(&mut explorer);
    assert_eq!(names(&explorer), ["inner.md"]);

    explorer.back();
    settle(&mut explorer);
    assert_eq!(explorer.active().path(), root);
    explorer.forward();
    settle(&mut explorer);
    assert_eq!(explorer.active().path(), root.join("alpha"));
    explorer.up();
    settle(&mut explorer);
    assert_eq!(explorer.active().path(), root);
}

#[test]
fn filters_and_shows_hidden_entries() {
    let (_dir, _root, mut explorer) = sample_tree();
    explorer.active_mut().set_filter("SMALL");
    assert_eq!(names(&explorer), ["small.txt"]);
    explorer.active_mut().set_filter("");

    assert!(!names(&explorer).contains(&".hidden".to_owned()));
    explorer.active_mut().set_show_hidden(true);
    assert!(names(&explorer).contains(&".hidden".to_owned()));
}

#[test]
fn opens_and_closes_tabs_but_keeps_the_last() {
    let (_dir, root, mut explorer) = sample_tree();
    explorer.new_tab(root);
    assert_eq!(explorer.tabs().len(), 2);
    assert_eq!(explorer.active_index(), 1);
    explorer.close_tab(0);
    assert_eq!(explorer.tabs().len(), 1);
    explorer.close_tab(0);
    assert_eq!(explorer.tabs().len(), 1, "the last tab is never closed");
}

#[test]
fn rename_validates_and_never_overwrites() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::write(root.join("a.txt"), b"a").unwrap();
    fs::write(root.join("b.txt"), b"b").unwrap();
    let mut explorer = explorer_in(root);
    let a = row_of(&explorer, "a.txt");

    for bad in ["", "   ", "x/y", ".", ".."] {
        explorer.rename(a, bad);
        let notice = last_notice(&mut explorer);
        assert_eq!(notice.kind, NoticeKind::Error, "name {bad:?}");
    }
    assert!(root.join("a.txt").exists());

    explorer.rename(a, "b.txt");
    settle(&mut explorer);
    assert_eq!(last_notice(&mut explorer).kind, NoticeKind::Error);
    assert_eq!(fs::read(root.join("a.txt")).unwrap(), b"a");
    assert_eq!(fs::read(root.join("b.txt")).unwrap(), b"b");

    explorer.rename(a, "c.txt");
    settle(&mut explorer);
    assert_eq!(last_notice(&mut explorer).kind, NoticeKind::Info);
    assert!(!root.join("a.txt").exists());
    assert_eq!(fs::read(root.join("c.txt")).unwrap(), b"a");
    assert!(names(&explorer).contains(&"c.txt".to_owned()));
}

#[test]
fn suggests_a_free_folder_name() {
    let dir = tempfile::tempdir().unwrap();
    let mut explorer = explorer_in(dir.path());
    assert_eq!(explorer.suggest_folder_name(), "New folder");
    explorer.create_folder("New folder");
    settle(&mut explorer);
    assert_eq!(explorer.suggest_folder_name(), "New folder (2)");
}

#[test]
fn create_folder_selects_the_new_folder_after_reload() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::write(root.join("file.txt"), b"").unwrap();
    let mut explorer = explorer_in(root);

    explorer.create_folder("a/b");
    assert_eq!(last_notice(&mut explorer).kind, NoticeKind::Error);

    explorer.create_folder("New");
    settle(&mut explorer);
    assert!(root.join("New").is_dir());
    assert_eq!(last_notice(&mut explorer).kind, NoticeKind::Info);
    assert_eq!(selected_names(&explorer), ["New"]);

    explorer.create_folder("New");
    settle(&mut explorer);
    assert_eq!(last_notice(&mut explorer).kind, NoticeKind::Error);
}

#[test]
fn tabs_inside_a_deleted_folder_move_to_the_nearest_existing_parent() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir_all(root.join("tree").join("deep")).unwrap();
    let mut explorer = explorer_in(root);
    explorer.new_tab(root.join("tree").join("deep"));
    settle(&mut explorer);
    explorer.set_active(0);

    explorer.delete_paths(vec![root.join("tree")], true);
    settle(&mut explorer);

    assert_eq!(explorer.tabs()[1].path(), root);
    assert!(matches!(explorer.tabs()[1].state(), LoadState::Loaded(_)));
}

#[test]
fn permanent_delete_removes_files_and_trees() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::write(root.join("file.txt"), b"x").unwrap();
    fs::create_dir_all(root.join("tree").join("deep")).unwrap();
    fs::write(root.join("tree").join("deep").join("leaf"), b"x").unwrap();
    let mut explorer = explorer_in(root);

    explorer.delete(&[row_of(&explorer, "file.txt")], true);
    settle(&mut explorer);
    assert_eq!(last_notice(&mut explorer).text, "Deleted 1 item");
    assert!(!root.join("file.txt").exists());

    explorer.delete(&[row_of(&explorer, "tree")], true);
    settle(&mut explorer);
    assert!(!root.join("tree").exists());
    assert!(names(&explorer).is_empty());
}

#[cfg(unix)]
#[test]
fn permanent_delete_of_a_symlink_keeps_its_target() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir(root.join("target")).unwrap();
    fs::write(root.join("target").join("precious.txt"), b"keep").unwrap();
    std::os::unix::fs::symlink(root.join("target"), root.join("link")).unwrap();
    let mut explorer = explorer_in(root);

    explorer.delete(&[row_of(&explorer, "link")], true);
    settle(&mut explorer);
    assert!(
        fs::symlink_metadata(root.join("link")).is_err(),
        "link is gone"
    );
    assert_eq!(
        fs::read(root.join("target").join("precious.txt")).unwrap(),
        b"keep",
        "target survives"
    );
}

#[test]
fn multi_delete_continues_past_a_failing_item() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    for name in ["one", "gone", "two"] {
        fs::write(root.join(name), b"x").unwrap();
    }
    let mut explorer = explorer_in(root);
    let rows = [
        row_of(&explorer, "one"),
        row_of(&explorer, "gone"),
        row_of(&explorer, "two"),
    ];
    // Vanishes after the listing, so its delete fails.
    fs::remove_file(root.join("gone")).unwrap();

    explorer.delete(&rows, true);
    settle(&mut explorer);
    let notice = last_notice(&mut explorer);
    assert_eq!(notice.kind, NoticeKind::Error);
    assert!(
        notice.text.starts_with("Deleted 2 of 3 items."),
        "{}",
        notice.text
    );
    assert!(
        notice.text.contains("Could not delete gone"),
        "{}",
        notice.text
    );
    assert!(!root.join("one").exists());
    assert!(!root.join("two").exists());
}

#[test]
fn delete_ignores_out_of_range_rows_and_refuses_while_busy() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::write(root.join("keep"), b"x").unwrap();
    let mut explorer = explorer_in(root);

    explorer.delete(&[99], true);
    assert_eq!(last_notice(&mut explorer).kind, NoticeKind::Error);
    assert!(root.join("keep").exists());

    explorer.delete(&[0], true);
    explorer.create_folder("second");
    assert!(explorer.busy());
    assert_eq!(
        last_notice(&mut explorer).text,
        "Another file operation is still running"
    );
    settle(&mut explorer);
    assert!(!root.join("keep").exists());
    assert!(!root.join("second").exists(), "refused create is not run");
}

#[test]
fn paste_copies_and_cut_paste_moves_in_the_background() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir_all(root.join("src/sub")).unwrap();
    fs::write(root.join("src/sub/f.txt"), b"x").unwrap();
    fs::create_dir(root.join("dst")).unwrap();
    let mut explorer = explorer_in(root);

    explorer.copy_to_clipboard(&[row_of(&explorer, "src")], false);
    explorer.paste(Some(root.join("dst")));
    settle(&mut explorer);
    // APFS, btrfs and XFS clone the file: "Copied 1 item (1 cloned)".
    let text = last_notice(&mut explorer).text;
    assert!(text.starts_with("Copied 1 item"), "{text}");
    assert_eq!(fs::read(root.join("dst/src/sub/f.txt")).unwrap(), b"x");
    assert!(explorer.can_paste(), "a copy clipboard can be pasted again");

    explorer.copy_to_clipboard(&[row_of(&explorer, "src")], true);
    explorer.paste(Some(root.to_path_buf()));
    assert!(!explorer.can_paste(), "a cut clipboard is spent");
    assert!(
        explorer.transfers().is_empty(),
        "cutting into its own folder does nothing"
    );
    assert!(root.join("src").exists());

    fs::create_dir(root.join("moved")).unwrap();
    explorer.copy_to_clipboard(&[row_of(&explorer, "src")], true);
    explorer.paste(Some(root.join("moved")));
    settle(&mut explorer);
    assert_eq!(last_notice(&mut explorer).text, "Moved 1 item");
    assert!(!root.join("src").exists());
    assert_eq!(fs::read(root.join("moved/src/sub/f.txt")).unwrap(), b"x");
}

#[test]
fn dropping_copies_or_moves() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir_all(root.join("src/sub")).unwrap();
    fs::write(root.join("src/sub/f.txt"), b"x").unwrap();
    fs::create_dir(root.join("dst")).unwrap();
    let mut explorer = explorer_in(root);

    explorer.drop_into(vec![root.join("src")], root, false);
    assert!(
        explorer.transfers().is_empty(),
        "a move into its own folder does nothing"
    );

    explorer.drop_into(vec![root.join("src")], &root.join("dst"), true);
    settle(&mut explorer);
    assert_eq!(fs::read(root.join("dst/src/sub/f.txt")).unwrap(), b"x");
    assert!(root.join("src").exists(), "a copy keeps its source");

    fs::create_dir(root.join("moved")).unwrap();
    explorer.drop_into(vec![root.join("src")], &root.join("moved"), false);
    settle(&mut explorer);
    assert_eq!(last_notice(&mut explorer).text, "Moved 1 item");
    assert!(!root.join("src").exists());
}

#[test]
fn paste_conflicts_wait_for_an_answer() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::write(root.join("f.txt"), b"new").unwrap();
    fs::create_dir(root.join("dst")).unwrap();
    fs::write(root.join("dst/f.txt"), b"old").unwrap();
    let mut explorer = explorer_in(root);
    assert_eq!(
        explorer.settings().conflict_policy,
        crate::ConflictChoice::Ask
    );

    explorer.copy_to_clipboard(&[row_of(&explorer, "f.txt")], false);
    explorer.paste(Some(root.join("dst")));
    wait_for(&mut explorer, "a conflict", |e| {
        e.transfers().first().is_some_and(|t| t.conflict.is_some())
    });
    let t = explorer.transfers().remove(0);
    let conflict = t.conflict.unwrap();
    assert_eq!(conflict.dst, root.join("dst/f.txt"));
    explorer.answer_conflict(t.id, conflict.id, Resolution::KeepBoth, false);
    settle(&mut explorer);
    assert_eq!(fs::read(root.join("dst/f.txt")).unwrap(), b"old");
    assert_eq!(fs::read(root.join("dst/f (2).txt")).unwrap(), b"new");
}

#[test]
fn failed_transfers_stay_in_the_panel_until_dismissed() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir(root.join("d")).unwrap();
    let mut explorer = explorer_in(root);
    explorer.copy_to_clipboard(&[row_of(&explorer, "d")], false);
    explorer.paste(Some(root.join("d")));
    settle(&mut explorer);
    let notice = last_notice(&mut explorer);
    assert_eq!(notice.kind, NoticeKind::Error);
    assert!(notice.text.contains("into itself"), "{}", notice.text);
    let transfers = explorer.transfers();
    assert_eq!(transfers.len(), 1);
    assert_eq!(transfers[0].state, crate::TransferState::Finished);
    assert_eq!(transfers[0].errors.len(), 1);
    explorer.dismiss_transfer(transfers[0].id);
    assert!(explorer.transfers().is_empty());
}

/// Uses the real OS trash, so it leaves the file in the user's trash on purpose.
#[test]
#[ignore = "writes into the real OS trash; run with --ignored"]
fn trash_delete_moves_the_file_away() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::write(root.join("bin.txt"), b"x").unwrap();
    let mut explorer = explorer_in(root);

    explorer.delete(&[row_of(&explorer, "bin.txt")], false);
    settle(&mut explorer);
    let notice = last_notice(&mut explorer);
    assert_eq!(notice.text, "Moved 1 item to trash");
    assert!(!root.join("bin.txt").exists());
}

#[test]
fn click_builds_windows_style_selection() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    for name in ["a", "b", "c", "d", "e"] {
        fs::write(root.join(name), b"").unwrap();
    }
    let mut explorer = explorer_in(root);
    let tab = explorer.active_mut();

    tab.click(1, false, false);
    assert_eq!(tab.selection(), [1]);
    tab.click(3, true, false);
    assert_eq!(tab.selection(), [1, 3]);
    assert_eq!(tab.selected(), Some(3), "ctrl-click moves the focus");
    tab.click(3, true, false);
    assert_eq!(tab.selection(), [1], "ctrl-click toggles off");
    tab.click(4, false, true);
    assert_eq!(
        tab.selection(),
        [3, 4],
        "shift range from the anchor (row 3)"
    );
    assert_eq!(tab.selected(), Some(3), "shift keeps the anchor");
    tab.click(1, true, true);
    assert_eq!(tab.selection(), [1, 2, 3, 4], "ctrl+shift adds the range");
    tab.click(0, false, false);
    assert_eq!(tab.selection(), [0]);
    assert!(tab.is_selected(0) && !tab.is_selected(1));

    tab.select_all();
    assert_eq!(tab.selection(), [0, 1, 2, 3, 4]);
    tab.click(99, false, false);
    assert_eq!(tab.selection().len(), 5, "out-of-range clicks are ignored");
    tab.select(Some(2));
    assert_eq!(tab.selection(), [2]);
    tab.move_selection(1);
    assert_eq!(tab.selection(), [3], "keyboard moves are single-select");
}

#[test]
fn selection_survives_resort_and_refresh_by_name() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::write(root.join("a.txt"), b"1").unwrap();
    fs::write(root.join("b.txt"), b"22").unwrap();
    fs::write(root.join("c.txt"), b"333").unwrap();
    let mut explorer = explorer_in(root);
    explorer.active_mut().click(0, false, false);
    explorer.active_mut().click(2, true, false);
    assert_eq!(selected_names(&explorer), ["a.txt", "c.txt"]);

    explorer.active_mut().set_sort_column(SortColumn::Size);
    assert_eq!(selected_names(&explorer), ["a.txt", "c.txt"]);

    explorer.refresh();
    settle(&mut explorer);
    assert_eq!(selected_names(&explorer), ["a.txt", "c.txt"]);
}

#[test]
fn entry_details_describe_files_dirs_and_permissions() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::write(root.join("file.txt"), b"12345").unwrap();
    fs::create_dir(root.join("folder")).unwrap();
    let explorer = explorer_in(root);

    let file = explorer
        .entry_details(row_of(&explorer, "file.txt"))
        .unwrap();
    assert_eq!(file.name, "file.txt");
    assert_eq!(file.path, root.join("file.txt"));
    assert_eq!(file.kind, EntryKind::File);
    assert_eq!(file.size, Some(5));
    assert!(file.modified.is_some());

    let folder = explorer.entry_details(row_of(&explorer, "folder")).unwrap();
    assert_eq!(folder.kind, EntryKind::Dir);
    assert_eq!(folder.size, None, "directories are not walked");
    assert!(explorer.entry_details(99).is_none());
}

#[cfg(unix)]
#[test]
fn entry_details_show_unix_mode() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let file = root.join("locked.txt");
    fs::write(&file, b"").unwrap();
    fs::set_permissions(&file, fs::Permissions::from_mode(0o640)).unwrap();
    let explorer = explorer_in(root);

    let details = explorer
        .entry_details(row_of(&explorer, "locked.txt"))
        .unwrap();
    assert_eq!(details.permissions, "rw-r-----");
}

#[test]
fn open_in_new_tab_only_for_folders() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::write(root.join("file.txt"), b"").unwrap();
    fs::create_dir(root.join("folder")).unwrap();
    let mut explorer = explorer_in(root);

    explorer.open_in_new_tab(row_of(&explorer, "file.txt"));
    assert_eq!(explorer.tabs().len(), 1);
    explorer.open_in_new_tab(row_of(&explorer, "folder"));
    assert_eq!(explorer.tabs().len(), 2);
    assert_eq!(explorer.active().path(), root.join("folder"));
}

#[test]
fn settings_apply_to_tabs_and_persist() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::write(root.join(".hidden"), b"").unwrap();
    hide(&root.join(".hidden"));
    let mut explorer = Explorer::with_settings(root.to_path_buf(), Settings::default(), || {});
    explorer.settings_path = Some(root.join("cfg").join("settings.toml"));
    settle(&mut explorer);
    assert!(!names(&explorer).contains(&".hidden".to_owned()));

    let updated = Settings {
        show_hidden: true,
        ..Settings::default()
    };
    explorer.set_settings(updated.clone());
    assert!(names(&explorer).contains(&".hidden".to_owned()));
    assert_eq!(explorer.settings(), &updated);
    assert_eq!(
        Settings::load_from(&root.join("cfg").join("settings.toml")),
        updated
    );

    explorer.new_tab(root.to_path_buf());
    settle(&mut explorer);
    assert!(
        names(&explorer).contains(&".hidden".to_owned()),
        "new tabs inherit it"
    );
}

#[test]
fn settings_save_failure_becomes_an_error_notice() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let blocker = root.join("not-a-dir");
    fs::write(&blocker, b"").unwrap();
    let mut explorer = explorer_in(root);
    explorer.settings_path = Some(blocker.join("settings.toml"));

    explorer.set_settings(Settings {
        show_hidden: true,
        ..Settings::default()
    });
    let notice = last_notice(&mut explorer);
    assert_eq!(notice.kind, NoticeKind::Error);
    assert!(
        notice.text.starts_with("Could not save settings"),
        "{}",
        notice.text
    );
}

fn child_names(explorer: &Explorer, dir: &Path) -> Vec<String> {
    match explorer.tree().children(dir) {
        Some(Children::Loaded(nodes)) => nodes.iter().map(|n| n.name.clone()).collect(),
        other => panic!("not loaded: {other:?}"),
    }
}

#[test]
fn tree_toggle_loads_sorted_folders_only_and_keeps_cache() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    for name in ["d10", "d2", "Alpha", ".hid"] {
        fs::create_dir(root.join(name)).unwrap();
    }
    fs::write(root.join("file.txt"), b"").unwrap();
    hide(&root.join(".hid"));
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(root.join("d2"), root.join("linkdir")).unwrap();
        std::os::unix::fs::symlink(root.join("file.txt"), root.join("linkfile")).unwrap();
    }
    let mut explorer = explorer_in(root);

    assert!(explorer.tree().children(root).is_none());
    explorer.tree_toggle(root.to_path_buf());
    assert!(explorer.tree().is_expanded(root));
    assert_eq!(explorer.tree().children(root), Some(&Children::Loading));
    settle(&mut explorer);
    #[cfg(unix)]
    assert_eq!(
        child_names(&explorer, root),
        ["Alpha", "d2", "d10", "linkdir"]
    );
    #[cfg(not(unix))]
    assert_eq!(child_names(&explorer, root), ["Alpha", "d2", "d10"]);

    explorer.tree_toggle(root.to_path_buf());
    assert!(!explorer.tree().is_expanded(root));
    assert!(matches!(
        explorer.tree().children(root),
        Some(Children::Loaded(_))
    ));

    explorer.tree_toggle(root.join("missing"));
    settle(&mut explorer);
    assert!(matches!(
        explorer.tree().children(&root.join("missing")),
        Some(Children::Failed(_))
    ));
}

#[test]
fn tree_show_hidden_reloads_cached_entries() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir(root.join(".hid")).unwrap();
    hide(&root.join(".hid"));
    fs::create_dir(root.join("vis")).unwrap();
    let mut explorer = explorer_in(root);
    let cfg = tempfile::tempdir().unwrap();
    explorer.settings_path = Some(cfg.path().join("settings.toml"));
    explorer.tree_expand(root.to_path_buf());
    settle(&mut explorer);
    assert_eq!(child_names(&explorer, root), ["vis"]);

    explorer.set_settings(Settings {
        show_hidden: true,
        ..Settings::default()
    });
    let deadline = Instant::now() + Duration::from_secs(10);
    while child_names(&explorer, root) != [".hid", "vis"] {
        assert!(
            Instant::now() < deadline,
            "reload did not finish in time: {:?}",
            explorer.tree().children(root)
        );
        explorer.poll();
        thread::sleep(Duration::from_millis(1));
    }
    assert!(explorer.tree().is_expanded(root));
}

#[test]
fn tree_reveal_expands_ancestors_below_the_deepest_root() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let deep = root.join("a").join("b").join("c");
    fs::create_dir_all(&deep).unwrap();
    fs::create_dir_all(root.join("a").join("sibling")).unwrap();
    let mut explorer = explorer_in(root);

    let roots = [root.to_path_buf(), root.join("a")];
    explorer.tree_reveal(&deep, &roots);
    settle(&mut explorer);
    assert!(explorer.tree().is_expanded(&root.join("a")));
    assert!(explorer.tree().is_expanded(&root.join("a").join("b")));
    assert!(!explorer.tree().is_expanded(&deep));
    assert!(!explorer.tree().is_expanded(root), "above the deepest root");
    assert!(
        !explorer.tree().is_expanded(&root.join("a").join("sibling")),
        "siblings of the path stay collapsed"
    );
    assert_eq!(child_names(&explorer, &root.join("a").join("b")), ["c"]);

    explorer.tree_reveal(root, &[]);
    assert!(!explorer.tree().is_expanded(root), "no root, no change");
}

#[test]
fn tree_reloads_after_a_folder_is_created() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir(root.join("one")).unwrap();
    let mut explorer = explorer_in(root);
    explorer.tree_expand(root.to_path_buf());
    settle(&mut explorer);
    assert_eq!(child_names(&explorer, root), ["one"]);

    explorer.create_folder("two");
    settle(&mut explorer);
    let deadline = Instant::now() + Duration::from_secs(10);
    while child_names(&explorer, root) != ["one", "two"] {
        assert!(
            Instant::now() < deadline,
            "reload did not finish in time: {:?}",
            explorer.tree().children(root)
        );
        explorer.poll();
        thread::sleep(Duration::from_millis(1));
    }
    assert!(explorer.tree().is_expanded(root));
}

/// Polls until `done` holds; bounded so a broken change fails instead of hanging.
fn wait_for(explorer: &mut Explorer, what: &str, done: impl Fn(&Explorer) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done(explorer) {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        explorer.poll();
        thread::sleep(Duration::from_millis(1));
    }
}

fn cached(explorer: &Explorer) -> Vec<String> {
    let mut names: Vec<String> = explorer.windows[0]
        .preloaded
        .keys()
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// Explorer whose settings are saved into `dir`, never into the user's real config.
fn explorer_with(dir: &Path, settings: Settings) -> Explorer {
    let mut explorer = Explorer::with_settings(dir.to_path_buf(), settings, || {});
    explorer.settings_path = Some(dir.join("cfg").join("settings.toml"));
    settle(&mut explorer);
    explorer
}

#[test]
fn preload_caches_subfolders_only() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir(root.join("a")).unwrap();
    fs::create_dir(root.join("b")).unwrap();
    fs::write(root.join("f.txt"), b"x").unwrap();
    let mut explorer = explorer_in(root);
    wait_for(&mut explorer, "preload", |e| {
        e.windows[0].preloaded.len() == 2
    });
    assert_eq!(cached(&explorer), ["a", "b"]);
}

#[test]
fn preloaded_folder_paints_instantly_then_refreshes() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let child = root.join("child");
    fs::create_dir(&child).unwrap();
    fs::write(child.join("old.txt"), b"x").unwrap();
    let mut explorer = explorer_in(root);
    wait_for(&mut explorer, "preload", |e| {
        e.windows[0].preloaded.len() == 1
    });

    fs::write(child.join("new.txt"), b"x").unwrap();
    explorer.navigate(child);
    assert_eq!(names(&explorer), ["old.txt"], "painted from the cache");
    settle(&mut explorer);
    wait_for(&mut explorer, "fresh listing", |e| {
        names(e).contains(&"new.txt".to_owned())
    });
}

#[test]
fn disabled_preload_caches_nothing_and_navigation_loads() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir(root.join("a")).unwrap();
    let settings = Settings {
        preload_subfolders: false,
        ..Settings::default()
    };
    let mut explorer = explorer_with(root, settings);
    for _ in 0..50 {
        explorer.poll();
        thread::sleep(Duration::from_millis(1));
    }
    assert!(explorer.windows[0].preloaded.is_empty());
    explorer.navigate(root.join("a"));
    assert!(matches!(explorer.active().state(), LoadState::Loading));
}

#[test]
fn stale_preload_results_are_dropped() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir(root.join("a")).unwrap();
    let mut explorer = explorer_in(root);
    wait_for(&mut explorer, "preload", |e| {
        e.windows[0].preloaded.len() == 1
    });
    explorer.cancel_for_test();
    let stale = explorer.windows[0].preload_gen.load(Ordering::Acquire) - 1;
    let listed = list_view(root.join("a"));
    explorer
        .preload_tx
        .send(PreloadDone {
            window: 0,
            generation: stale,
            path: root.join("a"),
            result: listed,
        })
        .unwrap();
    explorer.poll();
    assert!(explorer.windows[0].preloaded.is_empty());
}

impl Explorer {
    /// Simulates a newer preload generation superseding the running one.
    fn cancel_for_test(&mut self) {
        self.windows[0].preload_gen.fetch_add(1, Ordering::AcqRel);
        self.windows[0].preloaded.clear();
    }
}

#[test]
fn preload_respects_the_folder_cap() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    for i in 0..PRELOAD_MAX + 5 {
        fs::create_dir(root.join(format!("d{i:02}"))).unwrap();
    }
    let mut explorer = explorer_in(root);
    wait_for(&mut explorer, "preload", |e| {
        e.windows[0].preloaded.len() == PRELOAD_MAX
    });
    for _ in 0..50 {
        explorer.poll();
        thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(explorer.windows[0].preloaded.len(), PRELOAD_MAX);
}

#[cfg(unix)]
#[test]
fn symlinked_folders_are_not_preloaded() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir(root.join("real")).unwrap();
    std::os::unix::fs::symlink(root.join("real"), root.join("link")).unwrap();
    let mut explorer = explorer_in(root);
    wait_for(&mut explorer, "preload", |e| {
        !e.windows[0].preloaded.is_empty()
    });
    for _ in 0..50 {
        explorer.poll();
        thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(cached(&explorer), ["real"]);
}

#[test]
fn deleting_evicts_the_cached_folder() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir(root.join("a")).unwrap();
    fs::create_dir(root.join("b")).unwrap();
    let mut explorer = explorer_in(root);
    wait_for(&mut explorer, "preload", |e| {
        e.windows[0].preloaded.len() == 2
    });
    let a = row_of(&explorer, "a");
    explorer.delete(&[a], true);
    settle(&mut explorer);
    assert!(!explorer.windows[0].preloaded.contains_key(&root.join("a")));
}

#[test]
fn toggling_the_setting_clears_and_repopulates() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir(root.join("a")).unwrap();
    let mut explorer = explorer_with(root, Settings::default());
    wait_for(&mut explorer, "preload", |e| {
        e.windows[0].preloaded.len() == 1
    });

    explorer.set_settings(Settings {
        preload_subfolders: false,
        ..Settings::default()
    });
    assert!(explorer.windows[0].preloaded.is_empty());
    explorer.set_settings(Settings::default());
    wait_for(&mut explorer, "repopulate", |e| {
        e.windows[0].preloaded.len() == 1
    });
}

#[test]
fn revalidate_picks_up_external_files_without_a_loading_flash() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::write(root.join("a.txt"), b"x").unwrap();
    let mut explorer = explorer_in(root);
    explorer.active_mut().select(Some(0));

    fs::write(root.join("b.txt"), b"x").unwrap();
    explorer.revalidate();
    assert!(matches!(explorer.active().state(), LoadState::Loaded(_)));
    wait_for(&mut explorer, "revalidated listing", |e| {
        names(e).contains(&"b.txt".to_owned())
    });
    assert_eq!(selected_names(&explorer), ["a.txt"], "selection survives");
}

#[test]
fn revalidate_keeps_the_running_preload() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir(root.join("a")).unwrap();
    let mut explorer = explorer_in(root);
    wait_for(&mut explorer, "preload", |e| {
        e.windows[0].preloaded.len() == 1
    });
    let generation = explorer.windows[0].preload_gen.load(Ordering::Acquire);

    fs::write(root.join("new.txt"), b"x").unwrap();
    explorer.revalidate();
    wait_for(&mut explorer, "revalidated listing", |e| {
        names(e).contains(&"new.txt".to_owned())
    });
    assert_eq!(
        explorer.windows[0].preload_gen.load(Ordering::Acquire),
        generation
    );
    assert_eq!(cached(&explorer), ["a"]);
}

#[test]
fn navigating_discards_a_pending_revalidate() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir(root.join("sub")).unwrap();
    fs::write(root.join("sub").join("inner.txt"), b"x").unwrap();
    let mut explorer = explorer_in(root);
    explorer.revalidate();
    explorer.navigate(root.join("sub"));
    settle(&mut explorer);
    for _ in 0..50 {
        explorer.poll();
        thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(explorer.active().path(), root.join("sub"));
    assert_eq!(names(&explorer), ["inner.txt"]);
}

#[test]
fn tick_notices_external_changes() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let mut explorer = explorer_in(root);
    fs::write(root.join("late.txt"), b"x").unwrap();
    explorer.windows[0].next_tick = Instant::now();
    wait_for(&mut explorer, "tick", |e| {
        names(e).contains(&"late.txt".to_owned())
    });
}
