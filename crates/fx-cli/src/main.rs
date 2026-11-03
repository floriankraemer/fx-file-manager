//! `fx`: presentation layer over fx-core.

use clap::{Parser, ValueEnum};
use fx_core::{
    Backend, Entry, EntryKind, ListOptions, Listing, StorageKind, Strategy, WalkOptions, list_dir,
    walk,
};
use std::io::{self, BufWriter, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[derive(Parser)]
#[command(
    name = "fx",
    about = "Fast directory listing",
    args_conflicts_with_subcommands = true
)]
struct Cli {
    #[command(subcommand)]
    cmd: Option<Cmd>,
    #[command(flatten)]
    list: ListArgs,
}

#[derive(clap::Subcommand)]
enum Cmd {
    /// Recursively count (and optionally print) everything below a directory.
    Walk(WalkArgs),
}

#[derive(clap::Args)]
struct WalkArgs {
    /// Directory to walk.
    #[arg(default_value = ".")]
    path: PathBuf,
    /// Print every full path, one per line (unordered).
    #[arg(long)]
    print: bool,
    #[arg(long, value_enum, default_value = "native")]
    backend: BackendArg,
    /// Override storage detection.
    #[arg(long, value_enum)]
    storage: Option<StorageArg>,
    /// Suppress entry output, only the summary.
    #[arg(long)]
    quiet: bool,
}

#[derive(clap::Args)]
struct ListArgs {
    /// Directory to list.
    #[arg(default_value = ".")]
    path: PathBuf,
    /// Skip metadata (fastest).
    #[arg(long)]
    names: bool,
    /// Keep enumeration order.
    #[arg(long)]
    no_sort: bool,
    #[arg(long, value_enum, default_value = "native")]
    backend: BackendArg,
    /// Override storage detection.
    #[arg(long, value_enum)]
    storage: Option<StorageArg>,
    /// No entry output, only the summary.
    #[arg(long)]
    quiet: bool,
}

#[derive(Clone, Copy, ValueEnum)]
enum BackendArg {
    Native,
    Std,
}

#[derive(Clone, Copy, ValueEnum)]
enum StorageArg {
    Ssd,
    Hdd,
    Network,
}

impl From<BackendArg> for Backend {
    fn from(b: BackendArg) -> Self {
        match b {
            BackendArg::Native => Backend::Native,
            BackendArg::Std => Backend::Std,
        }
    }
}

impl From<StorageArg> for StorageKind {
    fn from(s: StorageArg) -> Self {
        match s {
            StorageArg::Ssd => StorageKind::Ssd,
            StorageArg::Hdd => StorageKind::Hdd,
            StorageArg::Network => StorageKind::Network,
        }
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match &cli.cmd {
        Some(Cmd::Walk(args)) => run_walk(args),
        None => run_list(&cli.list),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        // Reader went away (`| head`): not worth a message.
        Err(e) if is_broken_pipe(&*e) => ExitCode::FAILURE,
        Err(e) => {
            eprintln!("fx: {e}");
            ExitCode::FAILURE
        }
    }
}

fn is_broken_pipe(e: &(dyn std::error::Error + 'static)) -> bool {
    e.downcast_ref::<io::Error>()
        .is_some_and(|e| e.kind() == io::ErrorKind::BrokenPipe)
}

fn run_list(args: &ListArgs) -> Result<(), Box<dyn std::error::Error>> {
    let opts = ListOptions {
        metadata: !args.names,
        sort: !args.no_sort,
        backend: args.backend.into(),
        storage: args.storage.map(Into::into),
    };

    let start = Instant::now();
    let listed = list_dir(&args.path, &opts)?;
    let elapsed = start.elapsed();

    if !args.quiet {
        write_listing(&listed.listing, !args.names)?;
    }
    eprintln!(
        "{} · {} · {}",
        plural(listed.listing.len() as u64, "entry", "entries"),
        fmt_duration(elapsed),
        fmt_env(listed.storage, &listed.strategy),
    );
    Ok(())
}

fn run_walk(args: &WalkArgs) -> Result<(), Box<dyn std::error::Error>> {
    let opts = WalkOptions {
        backend: args.backend.into(),
        storage: args.storage.map(Into::into),
    };
    let print = args.print && !args.quiet;
    let out = Mutex::new(BufWriter::with_capacity(64 * 1024, io::stdout()));
    // First write error is kept; later dirs skip printing.
    let write_error: Mutex<Option<io::Error>> = Mutex::new(None);

    let start = Instant::now();
    let walked = walk(&args.path, &opts, |dir, listing| {
        if !print || write_error.lock().unwrap().is_some() {
            return;
        }
        // Build the dir's block outside the lock; write it under the lock in one go.
        let mut block = Vec::with_capacity(listing.len() * 64);
        for e in &listing.entries {
            block.extend_from_slice(&os_bytes(dir.join(listing.name(e)).as_os_str()));
            block.push(b'\n');
        }
        if let Err(e) = out.lock().unwrap().write_all(&block) {
            write_error.lock().unwrap().get_or_insert(e);
        }
    })?;
    let elapsed = start.elapsed();

    let flushed = out.into_inner().unwrap().flush();
    if let Some(e) = write_error.into_inner().unwrap() {
        return Err(e.into());
    }
    flushed?;
    let s = walked.stats;
    let counts = [
        Some(plural(s.files, "file", "files")),
        Some(plural(s.dirs, "dir", "dirs")),
        Some(plural(s.symlinks, "symlink", "symlinks")),
        (s.others > 0).then(|| plural(s.others, "other", "others")),
        (s.errors > 0).then(|| plural(s.errors, "error", "errors")),
    ];
    eprintln!(
        "{} · {} · {}",
        counts.into_iter().flatten().collect::<Vec<_>>().join(", "),
        fmt_duration(elapsed),
        fmt_env(walked.storage, &walked.strategy),
    );
    Ok(())
}

fn plural(n: u64, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// `storage: ssd · 16 stat / 16 walk threads, 64 KiB buffer[, inode order]`
fn fmt_env(storage: StorageKind, s: &Strategy) -> String {
    format!(
        "storage: {storage} · {} stat / {} walk threads, {} KiB buffer{}",
        s.stat_threads,
        s.walk_threads,
        s.buf_size / 1024,
        if s.inode_order { ", inode order" } else { "" },
    )
}

/// Raw bytes on Unix (names need not be UTF-8); lossy elsewhere.
fn os_bytes(s: &std::ffi::OsStr) -> std::borrow::Cow<'_, [u8]> {
    #[cfg(unix)]
    {
        std::os::unix::ffi::OsStrExt::as_bytes(s).into()
    }
    #[cfg(not(unix))]
    {
        s.to_string_lossy().into_owned().into_bytes().into()
    }
}

fn fmt_duration(d: Duration) -> String {
    format!("{:.1} ms", d.as_secs_f64() * 1000.0)
}

fn write_listing(listing: &Listing, metadata: bool) -> io::Result<()> {
    let stdout = io::stdout().lock();
    let mut out = BufWriter::with_capacity(64 * 1024, stdout);
    let mut line = Vec::with_capacity(512);
    for e in &listing.entries {
        line.clear();
        line.push(kind_char(e.kind));
        line.push(b' ');
        if metadata {
            push_size(&mut line, e);
            line.push(b' ');
            push_mtime(&mut line, e.modified);
            line.push(b' ');
        }
        line.extend_from_slice(&os_bytes(listing.name(e)));
        line.push(b'\n');
        out.write_all(&line)?;
    }
    out.flush()
}

fn kind_char(kind: EntryKind) -> u8 {
    match kind {
        EntryKind::File => b'-',
        EntryKind::Dir => b'd',
        EntryKind::Symlink => b'l',
        EntryKind::Other => b'?',
    }
}

const SIZE_WIDTH: usize = 12;

/// Right-aligned decimal size, or `-` when unknown.
fn push_size(line: &mut Vec<u8>, e: &Entry) {
    let start = line.len();
    match e.size {
        Some(n) => push_decimal(line, n, 1),
        None => line.push(b'-'),
    }
    let len = line.len() - start;
    if len < SIZE_WIDTH {
        line.splice(start..start, std::iter::repeat_n(b' ', SIZE_WIDTH - len));
    }
}

/// Appends `n` in decimal, zero-padded to at least `width` digits.
fn push_decimal(line: &mut Vec<u8>, mut n: u64, width: usize) {
    let mut digits = [b'0'; 20];
    let mut i = digits.len();
    while n > 0 {
        i -= 1;
        digits[i] = b'0' + (n % 10) as u8;
        n /= 10;
    }
    i = i.min(digits.len() - width);
    line.extend_from_slice(&digits[i..]);
}

/// `YYYY-MM-DD HH:MM` in UTC, or dashes when unknown.
fn push_mtime(line: &mut Vec<u8>, modified: Option<SystemTime>) {
    let Some(secs) = modified.map(unix_secs) else {
        line.extend_from_slice(b"----------------");
        return;
    };
    let (y, m, d) = civil_from_days(secs.div_euclid(86_400));
    let rem = secs.rem_euclid(86_400) as u64;
    push_decimal(line, y as u64, 4);
    line.push(b'-');
    push_decimal(line, m, 2);
    line.push(b'-');
    push_decimal(line, d, 2);
    line.push(b' ');
    push_decimal(line, rem / 3600, 2);
    line.push(b':');
    push_decimal(line, rem % 3600 / 60, 2);
}

/// Whole seconds relative to the Unix epoch; negative before 1970.
fn unix_secs(t: SystemTime) -> i64 {
    match t.duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_secs() as i64,
        Err(e) => {
            let before = e.duration();
            -(before.as_secs() as i64) - i64::from(before.subsec_nanos() > 0)
        }
    }
}

/// Days since 1970-01-01 to (year, month, day); Howard Hinnant's civil_from_days.
fn civil_from_days(z: i64) -> (i64, u64, u64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u64;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u64;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(19_723), (2024, 1, 1));
        assert_eq!(civil_from_days(19_782), (2024, 2, 29));
    }

    #[test]
    fn pre_epoch_times() {
        assert_eq!(unix_secs(UNIX_EPOCH - Duration::from_millis(500)), -1);
        assert_eq!(unix_secs(UNIX_EPOCH - Duration::from_secs(86_400)), -86_400);
        let mut v = Vec::new();
        push_mtime(&mut v, Some(UNIX_EPOCH - Duration::from_secs(60)));
        assert_eq!(v, b"1969-12-31 23:59");
    }

    #[test]
    fn decimal_padding_and_zero() {
        let mut v = Vec::new();
        push_decimal(&mut v, 0, 1);
        push_decimal(&mut v, 7, 2);
        assert_eq!(v, b"007");
    }
}
