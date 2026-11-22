//! Diagnostic log for the desktop app: one size-capped, rotating plain-text file in the
//! platform's log directory, fed by the `log` facade that fx and its dependencies (eframe,
//! winit, wgpu) already write to.
//!
//! The budget is fixed at `(KEEP + 1) * MAX_FILE_BYTES`, so a logging storm can only rotate
//! old lines away, never fill the disk. `FX_LOG=error|warn|info|debug|trace|off` changes the
//! level of fx's own crates; dependencies stay at warn or quieter.

mod file;

use file::RotatingFile;
use log::{LevelFilter, Log, Metadata, Record};
use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;
const KEEP: usize = 3;
/// A single record longer than this is cut, so one huge message cannot evict the whole history.
const MAX_LINE_BYTES: usize = 8 * 1024;
const OWN_CRATES: [&str; 5] = ["fx_core", "fx_app", "fx_gui", "fx_cli", "icon_theme"];

struct FileLogger {
    sink: Mutex<RotatingFile>,
    own: LevelFilter,
    deps: LevelFilter,
    echo_stderr: bool,
}

/// Installs the file logger and a panic hook that records panics before the process aborts.
/// Returns the log file path, or `None` when no log directory is available or writable; the app
/// then runs without a log rather than failing.
pub fn init() -> Option<PathBuf> {
    let path = log_dir()?.join("fx.log");
    let sink = RotatingFile::open(path.clone(), MAX_FILE_BYTES, KEEP).ok()?;
    let own = level_from_env();
    let logger = FileLogger {
        sink: Mutex::new(sink),
        own,
        deps: own.min(LevelFilter::Warn),
        echo_stderr: cfg!(debug_assertions),
    };
    log::set_boxed_logger(Box::new(logger)).ok()?;
    log::set_max_level(own.max(LevelFilter::Warn));
    install_panic_hook();
    Some(path)
}

impl Log for FileLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        let limit = if is_own(metadata.target()) {
            self.own
        } else {
            self.deps
        };
        metadata.level() <= limit
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let line = format_line(SystemTime::now(), record);
        if self.echo_stderr {
            eprint!("{line}");
        }
        let mut sink = self.sink.lock().unwrap_or_else(PoisonError::into_inner);
        sink.write_line(line.as_bytes());
    }

    fn flush(&self) {}
}

fn is_own(target: &str) -> bool {
    let krate = target.split("::").next().unwrap_or(target);
    OWN_CRATES.contains(&krate)
}

fn level_from_env() -> LevelFilter {
    std::env::var("FX_LOG")
        .ok()
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(LevelFilter::Info)
}

/// `2026-11-22T10:11:12.345Z WARN  [thread] target: message`, one line, UTC.
fn format_line(now: SystemTime, record: &Record) -> String {
    let mut line = String::with_capacity(128);
    let _ = write!(
        line,
        "{} {:<5} [{}] {}: {}",
        timestamp(now),
        record.level(),
        std::thread::current().name().unwrap_or("?"),
        record.target(),
        record.args()
    );
    truncate_at_char(&mut line, MAX_LINE_BYTES);
    line.push('\n');
    line
}

fn truncate_at_char(line: &mut String, max: usize) {
    if line.len() <= max {
        return;
    }
    let mut cut = max;
    while !line.is_char_boundary(cut) {
        cut -= 1;
    }
    line.truncate(cut);
    line.push_str(" [...]");
}

fn timestamp(now: SystemTime) -> String {
    let since = now.duration_since(UNIX_EPOCH).unwrap_or_default();
    let secs = since.as_secs() as i64;
    let (year, month, day) = crate::format::civil_from_days(secs.div_euclid(86_400));
    let of_day = secs.rem_euclid(86_400);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z",
        of_day / 3_600,
        of_day % 3_600 / 60,
        of_day % 60,
        since.subsec_millis()
    )
}

/// Release builds abort on panic, so without this hook a crash leaves no trace at all.
fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        log::error!(
            target: "fx_app::panic",
            "panic: {info}\n{}",
            std::backtrace::Backtrace::force_capture()
        );
        previous(info);
    }));
}

/// Linux: `$XDG_STATE_HOME/fx/logs` (XDG puts logs under state), macOS: `~/Library/Logs/fx`,
/// Windows: `%LOCALAPPDATA%\fx\logs` (logs are per machine, so not the roaming `%APPDATA%`).
fn log_dir() -> Option<PathBuf> {
    platform_log_dir().filter(|dir| dir.is_absolute())
}

#[cfg(windows)]
fn platform_log_dir() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA")
        .or_else(|| std::env::var_os("APPDATA"))
        .map(|dir| PathBuf::from(dir).join("fx").join("logs"))
}

#[cfg(target_os = "macos")]
fn platform_log_dir() -> Option<PathBuf> {
    crate::settings::home_dir().map(|home| home.join("Library").join("Logs").join("fx"))
}

#[cfg(all(unix, not(target_os = "macos")))]
fn platform_log_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute())
        .or_else(|| crate::settings::home_dir().map(|home| home.join(".local").join("state")))
        .map(|dir| dir.join("fx").join("logs"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use log::Level;
    use std::time::Duration;

    #[test]
    fn timestamps_are_iso_utc_with_millis() {
        let t = UNIX_EPOCH + Duration::from_millis(1_700_000_000_123);
        assert_eq!(timestamp(t), "2023-11-14T22:13:20.123Z");
    }

    #[test]
    fn own_crates_are_recognised_by_target_prefix() {
        assert!(is_own("fx_core::copy::pipeline"));
        assert!(is_own("fx_gui"));
        assert!(!is_own("fx_corex"));
        assert!(!is_own("wgpu_core::device"));
    }

    #[test]
    fn long_lines_are_cut_on_a_char_boundary() {
        let mut line = "ä".repeat(10);
        truncate_at_char(&mut line, 5);
        assert_eq!(line, "ää [...]");
    }

    #[test]
    fn records_render_on_one_prefixed_line() {
        let args = format_args!("copied {} files", 3);
        let record = Record::builder()
            .level(Level::Warn)
            .target("fx_core::copy")
            .args(args)
            .build();
        let line = format_line(UNIX_EPOCH, &record);
        assert!(
            line.starts_with("1970-01-01T00:00:00.000Z WARN  ["),
            "{line}"
        );
        assert!(
            line.ends_with("] fx_core::copy: copied 3 files\n"),
            "{line}"
        );
    }
}
