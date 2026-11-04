//! `fx`: presentation layer over fx-core.

use clap::{Parser, ValueEnum};
use fx_core::{
    Backend, ConflictPolicy, Entry, EntryKind, Item, ListOptions, Listing, Method, Op, Order,
    Overrides, Request, Schedule, StorageKind, Strategy, Summary, TransferHandle, WalkOptions,
    list_dir, walk,
};
use std::io::IsTerminal;
use std::io::{self, BufWriter, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[derive(Parser)]
#[command(
    name = "fx",
    about = "Fast directory listing, copying and moving",
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
    /// Copy files and folders (like `cp -a`): `fx cp SRC... DIR` or `fx cp SRC NEW_NAME`.
    Cp(TransferArgs),
    /// Move files and folders: same-volume moves rename, others copy, flush, then delete.
    Mv(TransferArgs),
}

#[derive(clap::Args)]
struct TransferArgs {
    /// Sources, then the destination.
    #[arg(required = true, num_args = 2..)]
    paths: Vec<PathBuf>,
    /// What to do when a destination entry exists. Folders always merge.
    #[arg(long, value_enum, default_value = "skip")]
    conflict: ConflictArg,
    /// Override storage detection on both sides.
    #[arg(long, value_enum)]
    storage: Option<StorageArg>,
    /// Override the planned worker count.
    #[arg(long)]
    workers: Option<usize>,
    /// Force one data method (no fallback): ficlone, copy-file-range, sendfile, clonefileat,
    /// copyfile2, duplicate-extents, read-write.
    #[arg(long, value_parser = parse_method)]
    method: Option<Method>,
    /// Override the planned schedule.
    #[arg(long, value_enum)]
    schedule: Option<ScheduleArg>,
    /// No progress line.
    #[arg(long)]
    quiet: bool,
}

#[derive(Clone, Copy, ValueEnum)]
enum ConflictArg {
    Skip,
    Overwrite,
    KeepBoth,
    OverwriteIfNewer,
    SkipIdentical,
}

#[derive(Clone, Copy, ValueEnum)]
enum ScheduleArg {
    Streaming,
    /// Disk order by first extent.
    TwoPhase,
    /// Disk order approximated by inode number.
    TwoPhaseInode,
}

fn parse_method(name: &str) -> Result<Method, String> {
    match Method::from_name(name) {
        Some(Method::Rename) | None => Err(format!("unknown data method `{name}`")),
        Some(m) => Ok(m),
    }
}

impl From<ConflictArg> for ConflictPolicy {
    fn from(c: ConflictArg) -> Self {
        match c {
            ConflictArg::Skip => ConflictPolicy::Skip,
            ConflictArg::Overwrite => ConflictPolicy::Overwrite,
            ConflictArg::KeepBoth => ConflictPolicy::KeepBoth,
            ConflictArg::OverwriteIfNewer => ConflictPolicy::OverwriteIfNewer,
            ConflictArg::SkipIdentical => ConflictPolicy::SkipIdentical,
        }
    }
}

impl From<ScheduleArg> for Schedule {
    fn from(s: ScheduleArg) -> Self {
        match s {
            ScheduleArg::Streaming => Schedule::Streaming,
            ScheduleArg::TwoPhase => Schedule::TwoPhase {
                order: Order::Extent,
            },
            ScheduleArg::TwoPhaseInode => Schedule::TwoPhase {
                order: Order::Inode,
            },
        }
    }
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
    Removable,
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
            StorageArg::Removable => StorageKind::Removable,
        }
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match &cli.cmd {
        Some(Cmd::Walk(args)) => run_walk(args),
        Some(Cmd::Cp(args)) => run_transfer(args, Op::Copy),
        Some(Cmd::Mv(args)) => run_transfer(args, Op::Move),
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

/// `fx cp a b dir/` copies into `dir`; `fx cp a b` with no existing `b` creates `b` (like `cp`).
fn transfer_items(paths: &[PathBuf]) -> Result<Vec<Item>, String> {
    let (dest, sources) = paths.split_last().ok_or("missing destination")?;
    if dest.is_dir() {
        return Ok(sources
            .iter()
            .map(|src| Item {
                src: src.clone(),
                dst: dest.join(src.file_name().unwrap_or(src.as_os_str())),
            })
            .collect());
    }
    match sources {
        [src] => Ok(vec![Item {
            src: src.clone(),
            dst: dest.clone(),
        }]),
        _ => Err(format!("{}: not a directory", dest.display())),
    }
}

fn run_transfer(args: &TransferArgs, op: Op) -> Result<(), Box<dyn std::error::Error>> {
    let req = Request {
        items: transfer_items(&args.paths)?,
        op,
        policy: args.conflict.into(),
        overrides: Overrides {
            storage: args.storage.map(Into::into),
            workers: args.workers,
            method: args.method,
            schedule: args.schedule.map(Into::into),
        },
    };
    let handle = TransferHandle::new(|| {});
    let show = !args.quiet && io::stderr().is_terminal();
    let done = AtomicBool::new(false);
    let summary = std::thread::scope(|s| {
        if show {
            s.spawn(|| {
                while !done.load(Ordering::Relaxed) {
                    let p = handle.snapshot();
                    let more = if p.walk_complete { "" } else { "+" };
                    let secs = p.elapsed.as_secs_f64().max(1e-3);
                    eprint!(
                        "\r{}/{}{more} files · {} · {}/s\x1b[K",
                        p.files_done,
                        p.files_total,
                        fmt_bytes(p.bytes_done),
                        fmt_bytes((p.bytes_done as f64 / secs) as u64),
                    );
                    std::thread::sleep(Duration::from_millis(200));
                }
                eprint!("\r\x1b[K");
            });
        }
        let summary = handle.run(&req);
        done.store(true, Ordering::Relaxed);
        summary
    });
    report(&summary);
    if summary.error_count() > 0 || summary.cancelled {
        return Err(format!("{} errors", summary.error_count()).into());
    }
    Ok(())
}

fn report(s: &Summary) {
    for e in s.errors.iter().filter(|e| !e.warning) {
        eprintln!("fx: {e}");
    }
    let warnings = s.errors.iter().filter(|e| e.warning).count();
    let counts = [
        Some(plural(s.files, "file", "files")),
        Some(fmt_bytes(s.bytes)),
        (s.cloned > 0).then(|| format!("{} cloned", s.cloned)),
        (s.skipped > 0).then(|| format!("{} skipped", s.skipped)),
        (warnings > 0).then(|| plural(warnings as u64, "warning", "warnings")),
        (!s.kept_sources.is_empty()).then(|| format!("{} sources kept", s.kept_sources.len())),
    ];
    let plans: Vec<String> = s
        .plans
        .iter()
        .map(|p| {
            let methods: Vec<&str> = p.methods.iter().map(|m| m.name()).collect();
            let schedule = match p.schedule {
                Schedule::Streaming => "streaming",
                Schedule::TwoPhase { .. } => "two-phase",
            };
            format!("{} workers, {schedule}, {}", p.workers, methods.join(" > "))
        })
        .collect();
    eprintln!(
        "{} · {} · {}",
        counts.into_iter().flatten().collect::<Vec<_>>().join(", "),
        fmt_duration(s.elapsed),
        if plans.is_empty() {
            "rename".to_owned()
        } else {
            plans.join("; ")
        },
    );
}

/// `1.5 MiB`-style sizes for the progress line.
fn fmt_bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut v = n as f64;
    let mut unit = 0;
    while v >= 1024.0 && unit < UNITS.len() - 1 {
        v /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{n} B")
    } else {
        format!("{v:.1} {}", UNITS[unit])
    }
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
    fn transfer_targets_follow_cp() {
        let dir = std::env::temp_dir();
        let items =
            transfer_items(&[PathBuf::from("/x/a"), PathBuf::from("/y/b"), dir.clone()]).unwrap();
        assert_eq!(items[1].dst, dir.join("b"));
        let items =
            transfer_items(&[PathBuf::from("/x/a"), PathBuf::from("/no/such/new")]).unwrap();
        assert_eq!(items[0].dst, PathBuf::from("/no/such/new"));
        assert!(
            transfer_items(&[
                PathBuf::from("a"),
                PathBuf::from("b"),
                PathBuf::from("/no/such")
            ])
            .is_err()
        );
        assert_eq!(fmt_bytes(512), "512 B");
        assert_eq!(fmt_bytes(1536), "1.5 KiB");
        assert!(parse_method("rename").is_err());
        assert_eq!(parse_method("read-write"), Ok(Method::ReadWrite));
    }

    #[test]
    fn decimal_padding_and_zero() {
        let mut v = Vec::new();
        push_decimal(&mut v, 0, 1);
        push_decimal(&mut v, 7, 2);
        assert_eq!(v, b"007");
    }
}
