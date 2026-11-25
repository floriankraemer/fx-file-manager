//! Fixtures shared by the unit tests of several modules.

use fx_app::Explorer;

/// An explorer whose active tab shows the finished search results for one file.
pub fn searching_explorer() -> Explorer {
    use std::time::{Duration, Instant};
    let dir = std::env::temp_dir().join(format!("fx-search-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("alpha.txt"), b"").unwrap();
    let mut explorer = Explorer::new(dir, || {});
    explorer.request_search("alpha");
    let deadline = Instant::now() + Duration::from_secs(10);
    while explorer.search().is_none_or(fx_app::SearchView::is_running) {
        assert!(Instant::now() < deadline, "search did not finish");
        explorer.poll();
        std::thread::sleep(Duration::from_millis(5));
    }
    explorer
}
