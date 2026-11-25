use super::tests::{settle, wait_for};
use super::*;
use crate::search::{SearchStatus, SearchView};
use crate::settings::SearchSettings;
use std::fs;
use std::path::Path;
use std::thread;
use std::time::Duration;

fn wait_for_search(explorer: &mut Explorer) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while explorer.search().is_some_and(SearchView::is_running) {
        assert!(Instant::now() < deadline, "search did not finish in time");
        explorer.poll();
        thread::sleep(Duration::from_millis(1));
    }
}

fn search_names(explorer: &Explorer) -> Vec<String> {
    let mut found: Vec<_> = explorer
        .search()
        .unwrap()
        .hits()
        .iter()
        .map(|h| h.path.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    found.sort();
    found
}

fn search_tree() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir_all(root.join("sub").join("deep")).unwrap();
    fs::create_dir_all(root.join(".secret")).unwrap();
    fs::write(root.join("alpha.txt"), b"").unwrap();
    fs::write(root.join("sub").join("deep").join("alpha_deep.txt"), b"").unwrap();
    fs::write(root.join("sub").join("beta.txt"), b"").unwrap();
    fs::write(root.join(".secret").join("alpha_in_secret.txt"), b"").unwrap();
    fs::write(root.join(".alpha_dot"), b"").unwrap();
    hide_on_windows(&root.join(".alpha_dot"));
    dir
}

/// Windows hides by attribute, not by leading dot.
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

fn search_explorer(root: &Path, search: SearchSettings) -> Explorer {
    let settings = Settings {
        search,
        ..Settings::default()
    };
    let mut explorer = Explorer::with_settings(root.to_path_buf(), settings, || {});
    explorer.settings_path = Some(root.join("cfg").join("settings.toml"));
    settle(&mut explorer);
    explorer
}

#[test]
fn search_streams_hits_and_reports_stats() {
    let dir = search_tree();
    let mut explorer = search_explorer(dir.path(), SearchSettings::default());
    explorer.start_search("beta");
    assert!(explorer.search().unwrap().is_running());
    wait_for_search(&mut explorer);
    let view = explorer.search().unwrap();
    assert_eq!(*view.status(), SearchStatus::Complete);
    assert_eq!(search_names(&explorer), ["beta.txt"]);
    assert_eq!(explorer.search().unwrap().dirs_walked(), 4);
    assert_eq!(explorer.search().unwrap().unreadable(), 0);
}

#[test]
fn blank_query_clears_instead_of_matching_everything() {
    let dir = search_tree();
    let mut explorer = search_explorer(dir.path(), SearchSettings::default());
    explorer.start_search("alpha");
    explorer.request_search("   ");
    assert!(explorer.search().is_none());
}

#[test]
fn search_hidden_entries_follow_show_hidden() {
    let dir = search_tree();
    let mut explorer = search_explorer(dir.path(), SearchSettings::default());
    explorer.start_search("alpha");
    wait_for_search(&mut explorer);
    assert_eq!(search_names(&explorer), ["alpha.txt", "alpha_deep.txt"]);

    explorer.set_settings(Settings {
        show_hidden: true,
        ..explorer.settings().clone()
    });
    explorer.start_search("alpha");
    wait_for_search(&mut explorer);
    assert_eq!(
        search_names(&explorer),
        [
            ".alpha_dot",
            "alpha.txt",
            "alpha_deep.txt",
            "alpha_in_secret.txt"
        ]
    );
}

#[test]
fn a_new_search_drops_the_old_searchs_results() {
    let dir = search_tree();
    let mut explorer = search_explorer(dir.path(), SearchSettings::default());
    explorer.start_search("alpha");
    explorer.start_search("beta");
    wait_for_search(&mut explorer);
    thread::sleep(Duration::from_millis(50));
    explorer.poll();
    assert_eq!(search_names(&explorer), ["beta.txt"]);
    assert_eq!(explorer.search().unwrap().query(), "beta");
}

#[test]
fn cancel_stops_delivery_but_keeps_hits() {
    let dir = search_tree();
    let mut explorer = search_explorer(dir.path(), SearchSettings::default());
    explorer.start_search("a");
    explorer.cancel_search();
    let view = explorer.search().unwrap();
    assert_eq!(*view.status(), SearchStatus::Stopped);
    let matches = view.matches();
    thread::sleep(Duration::from_millis(100));
    explorer.poll();
    let view = explorer.search().unwrap();
    assert_eq!(view.matches(), matches);
    assert_eq!(*view.status(), SearchStatus::Stopped);
}

#[test]
fn navigating_away_cancels_and_clears_the_search() {
    let dir = search_tree();
    let mut explorer = search_explorer(dir.path(), SearchSettings::default());
    explorer.start_search("alpha");
    let control = explorer.search().unwrap().control();
    explorer.navigate(dir.path().join("sub"));
    assert!(control.is_cancelled());
    assert!(explorer.search().is_none());
    settle(&mut explorer);
    thread::sleep(Duration::from_millis(50));
    explorer.poll();
    assert!(explorer.search().is_none());
}

#[test]
fn refreshing_keeps_the_search() {
    let dir = search_tree();
    let mut explorer = search_explorer(dir.path(), SearchSettings::default());
    explorer.start_search("alpha");
    explorer.refresh();
    assert!(explorer.search().is_some());
    wait_for_search(&mut explorer);
    assert_eq!(*explorer.search().unwrap().status(), SearchStatus::Complete);
}

#[test]
fn result_cap_stops_the_search() {
    let dir = tempfile::tempdir().unwrap();
    for i in 0..20 {
        fs::write(dir.path().join(format!("f{i}.txt")), b"").unwrap();
    }
    let search = SearchSettings {
        result_cap: 5,
        ..SearchSettings::default()
    };
    let mut explorer = search_explorer(dir.path(), search);
    explorer.start_search("f");
    wait_for_search(&mut explorer);
    let view = explorer.search().unwrap();
    assert_eq!(*view.status(), SearchStatus::StoppedAtCap);
    assert_eq!(view.matches(), 5);
    thread::sleep(Duration::from_millis(50));
    explorer.poll();
    assert_eq!(explorer.search().unwrap().matches(), 5);
}

#[test]
fn exactly_cap_matches_is_complete() {
    let dir = tempfile::tempdir().unwrap();
    for i in 0..5 {
        fs::write(dir.path().join(format!("f{i}.txt")), b"").unwrap();
    }
    let search = SearchSettings {
        result_cap: 5,
        ..SearchSettings::default()
    };
    let mut explorer = search_explorer(dir.path(), search);
    explorer.start_search("f");
    wait_for_search(&mut explorer);
    assert_eq!(*explorer.search().unwrap().status(), SearchStatus::Complete);
}

#[test]
fn unreadable_root_fails_the_search() {
    let dir = tempfile::tempdir().unwrap();
    let mut explorer = search_explorer(dir.path(), SearchSettings::default());
    let gone = dir.path().join("gone");
    explorer.active_mut().navigate_to(gone);
    explorer.start_search("x");
    wait_for_search(&mut explorer);
    assert!(matches!(
        explorer.search().unwrap().status(),
        SearchStatus::Failed(_)
    ));
}

#[test]
fn search_selection_moves_and_clamps() {
    let dir = search_tree();
    let mut explorer = search_explorer(dir.path(), SearchSettings::default());
    explorer.start_search("alpha");
    wait_for_search(&mut explorer);
    let selected = |e: &Explorer| e.search().unwrap().selected();
    explorer.move_search_selection(1);
    assert_eq!(selected(&explorer), Some(0));
    explorer.move_search_selection(10);
    assert_eq!(selected(&explorer), Some(1));
    explorer.move_search_selection(-10);
    assert_eq!(selected(&explorer), Some(0));
    explorer.select_search_hit(Some(99));
    assert_eq!(selected(&explorer), None);
}

#[test]
fn reveal_goes_to_the_parent_selects_the_hit_and_ends_the_search() {
    let dir = search_tree();
    let mut explorer = search_explorer(dir.path(), SearchSettings::default());
    explorer.start_search("alpha_deep");
    wait_for_search(&mut explorer);
    explorer.reveal_search_hit(0);
    assert!(explorer.search().is_none());
    let deep = dir.path().join("sub").join("deep");
    wait_for(&mut explorer, "reveal load", |e| {
        e.active().path() == deep && e.active().selected().is_some()
    });
    let row = explorer.active().selected().unwrap();
    assert_eq!(explorer.entry_path(row), Some(deep.join("alpha_deep.txt")));
}

#[test]
fn opening_a_folder_hit_enters_it() {
    let dir = search_tree();
    let mut explorer = search_explorer(dir.path(), SearchSettings::default());
    explorer.start_search("deep");
    wait_for_search(&mut explorer);
    let row = explorer
        .search()
        .unwrap()
        .hits()
        .iter()
        .position(|h| h.path.ends_with("deep"))
        .unwrap();
    explorer.open_search_hit(row);
    assert!(explorer.search().is_none());
    assert_eq!(
        explorer.active().path(),
        dir.path().join("sub").join("deep")
    );
}

#[test]
fn relative_folder_is_slash_separated_and_empty_at_the_root() {
    let dir = search_tree();
    let mut explorer = search_explorer(dir.path(), SearchSettings::default());
    explorer.start_search("alpha");
    wait_for_search(&mut explorer);
    let view = explorer.search().unwrap();
    let mut folders: Vec<_> = view
        .hits()
        .iter()
        .map(|h| view.relative_folder(h))
        .collect();
    folders.sort();
    assert_eq!(folders, ["", "sub/deep/"]);
}

fn show_hidden(explorer: &mut Explorer) {
    explorer.set_settings(Settings {
        show_hidden: true,
        ..explorer.settings().clone()
    });
}

#[test]
fn changing_show_hidden_restarts_the_running_searches() {
    let dir = search_tree();
    let mut explorer = search_explorer(dir.path(), SearchSettings::default());
    explorer.start_search("alpha");
    wait_for_search(&mut explorer);
    assert_eq!(search_names(&explorer).len(), 2);
    show_hidden(&mut explorer);
    wait_for_search(&mut explorer);
    assert_eq!(search_names(&explorer).len(), 4);
}

#[test]
fn restarting_searches_covers_inactive_tabs() {
    let dir = search_tree();
    let mut explorer = search_explorer(dir.path(), SearchSettings::default());
    explorer.start_search("alpha");
    explorer.new_tab(dir.path().join("sub"));
    show_hidden(&mut explorer);
    explorer.set_active(0);
    wait_for_search(&mut explorer);
    assert_eq!(search_names(&explorer).len(), 4);
}

#[test]
fn a_stopped_search_reports_stable_counts() {
    let dir = search_tree();
    let mut explorer = search_explorer(dir.path(), SearchSettings::default());
    explorer.start_search("a");
    explorer.cancel_search();
    thread::sleep(Duration::from_millis(100));
    explorer.poll();
    let view = explorer.search().unwrap();
    assert_eq!(*view.status(), SearchStatus::Stopped);
    let walked = view.dirs_walked();
    assert!(walked <= 4);
    thread::sleep(Duration::from_millis(50));
    assert_eq!(explorer.search().unwrap().dirs_walked(), walked);
}

#[test]
fn the_cap_is_enforced_in_the_worker() {
    let dir = tempfile::tempdir().unwrap();
    for i in 0..200 {
        fs::write(dir.path().join(format!("f{i}.txt")), b"").unwrap();
    }
    let search = SearchSettings {
        result_cap: 5,
        ..SearchSettings::default()
    };
    let mut explorer = search_explorer(dir.path(), search);
    explorer.start_search("f");
    wait_for_search(&mut explorer);
    thread::sleep(Duration::from_millis(50));
    explorer.poll();
    let view = explorer.search().unwrap();
    assert_eq!(*view.status(), SearchStatus::StoppedAtCap);
    assert_eq!(view.matches(), 5);
}

#[cfg(unix)]
#[test]
fn unreadable_folders_are_listed_with_their_path_and_counted() {
    use std::os::unix::fs::PermissionsExt;
    let dir = search_tree();
    let locked = dir.path().join("sub").join("deep");
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
    let readable = fs::read_dir(&locked).is_ok();
    let mut explorer = search_explorer(dir.path(), SearchSettings::default());
    explorer.start_search("alpha");
    wait_for_search(&mut explorer);
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
    if readable {
        return;
    }
    let view = explorer.search().unwrap();
    assert_eq!(view.unreadable(), 1);
    let folders = view.unreadable_folders();
    assert_eq!(folders.len(), 1);
    assert_eq!(folders[0].path, locked);
    assert!(!folders[0].error.is_empty());
}

#[test]
fn toggling_case_persists_and_restarts_the_search() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("Readme.md"), b"").unwrap();
    fs::write(dir.path().join("readme.txt"), b"").unwrap();
    let mut explorer = search_explorer(dir.path(), SearchSettings::default());
    explorer.start_search("readme");
    wait_for_search(&mut explorer);
    assert_eq!(search_names(&explorer), ["Readme.md", "readme.txt"]);

    explorer.toggle_search_case();
    assert!(explorer.search().unwrap().is_running());
    wait_for_search(&mut explorer);
    assert_eq!(search_names(&explorer), ["readme.txt"]);
    let saved = Settings::load_from(explorer.settings_path.as_ref().unwrap());
    assert!(saved.search.case_sensitive);
}

fn at_filesystem_root(dir: &Path) -> Explorer {
    let mut explorer = search_explorer(dir, SearchSettings::default());
    explorer
        .active_mut()
        .navigate_to(PathBuf::from(std::path::MAIN_SEPARATOR_STR));
    explorer
}

#[test]
fn the_root_notice_holds_the_query_and_shows_once_per_session() {
    let dir = tempfile::tempdir().unwrap();
    let mut explorer = at_filesystem_root(dir.path());
    assert_eq!(explorer.request_search("x"), SearchStart::NeedsRootNotice);
    assert!(explorer.search().is_none());

    explorer.acknowledge_root_notice(false);
    assert_eq!(explorer.search().unwrap().query(), "x");
    assert!(explorer.settings().search.warn_root);
    explorer.clear_search();
    assert_eq!(explorer.request_search("y"), SearchStart::Started);
    assert_eq!(explorer.search().unwrap().query(), "y");
}

#[test]
fn dont_show_again_persists_and_the_notice_is_skipped_below_the_root() {
    let dir = tempfile::tempdir().unwrap();
    let mut explorer = at_filesystem_root(dir.path());
    explorer.request_search("x");
    explorer.acknowledge_root_notice(true);
    let saved = Settings::load_from(explorer.settings_path.as_ref().unwrap());
    assert!(!saved.search.warn_root);

    let mut elsewhere = search_explorer(dir.path(), SearchSettings::default());
    assert_eq!(elsewhere.request_search("x"), SearchStart::Started);
}

#[test]
fn a_held_search_never_runs_in_another_tab() {
    let dir = tempfile::tempdir().unwrap();
    let mut explorer = at_filesystem_root(dir.path());
    explorer.request_search("x");
    explorer.new_tab(dir.path().to_path_buf());
    explorer.acknowledge_root_notice(false);
    assert!(explorer.search().is_none());
}

#[test]
fn a_notice_that_was_not_acknowledged_comes_back() {
    let dir = tempfile::tempdir().unwrap();
    let mut explorer = at_filesystem_root(dir.path());
    explorer.request_search("x");
    assert_eq!(explorer.request_search("x"), SearchStart::NeedsRootNotice);
}

#[test]
fn escape_stops_a_running_search_then_clears_it() {
    let dir = search_tree();
    let mut explorer = search_explorer(dir.path(), SearchSettings::default());
    explorer.start_search("a");
    assert!(explorer.escape_search());
    assert_eq!(*explorer.search().unwrap().status(), SearchStatus::Stopped);
    assert!(!explorer.escape_search());
    assert!(explorer.search().is_none());
}

#[test]
fn closing_a_tab_cancels_its_walk() {
    let dir = search_tree();
    let mut explorer = search_explorer(dir.path(), SearchSettings::default());
    explorer.start_search("alpha");
    let control = explorer.search().unwrap().control();
    explorer.new_tab(dir.path().join("sub"));
    explorer.close_tab(0);
    assert!(control.is_cancelled());
}
