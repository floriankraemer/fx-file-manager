//! `fx find`: streams matches to stdout as the walk finds them, unreadable folders to stderr.

use crate::{BackendArg, StorageArg, fmt_duration, fmt_env, os_bytes, plural};
use fx_core::{
    GlobScope, Hit, SearchEvent, SearchOptions, SearchQuery, UnreadableFolder, WalkControl,
    WalkOptions, Walked, search,
};
use std::io::{self, BufWriter, Stdout, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

#[derive(clap::Args)]
pub(crate) struct FindArgs {
    /// Directory to search.
    path: PathBuf,
    /// Substring, or a glob when it contains `*` or `?`.
    pattern: String,
    /// Match globs against the path relative to the search root instead of the name.
    #[arg(long)]
    relative: bool,
    /// Match letter case exactly; by default ASCII case is ignored.
    #[arg(long)]
    case_sensitive: bool,
    #[arg(long, value_enum, default_value = "native")]
    backend: BackendArg,
    /// Override storage detection.
    #[arg(long, value_enum)]
    storage: Option<StorageArg>,
    /// Suppress match output and unreadable-folder lines, only the summary.
    #[arg(long)]
    quiet: bool,
}

impl FindArgs {
    fn search_options(&self) -> SearchOptions {
        SearchOptions {
            scope: if self.relative {
                GlobScope::RelativePath
            } else {
                GlobScope::Name
            },
            case_sensitive: self.case_sensitive,
        }
    }

    fn walk_options(&self) -> WalkOptions {
        WalkOptions {
            backend: self.backend.into(),
            storage: self.storage.map(Into::into),
        }
    }
}

pub(crate) fn run(args: &FindArgs) -> Result<(), Box<dyn std::error::Error>> {
    let query = SearchQuery::new(&args.pattern, args.search_options());
    let control = WalkControl::default();
    let sink = Sink::new(args.quiet, Instant::now());

    let start = Instant::now();
    let walked = search(
        &args.path,
        &query,
        &args.walk_options(),
        &control,
        |event| sink.on_event(event),
    )?;
    let elapsed = start.elapsed();

    sink.finish()?;
    print_summary(&sink, &control, &walked, elapsed);
    Ok(())
}

/// Receives events from many threads; keeps output lines whole and the first write error.
struct Sink {
    quiet: bool,
    started: Instant,
    matches: AtomicU64,
    first_result: OnceLock<Duration>,
    stdout: Mutex<HitStream>,
}

struct HitStream {
    out: BufWriter<Stdout>,
    error: Option<io::Error>,
}

impl Sink {
    fn new(quiet: bool, started: Instant) -> Self {
        Self {
            quiet,
            started,
            matches: AtomicU64::new(0),
            first_result: OnceLock::new(),
            stdout: Mutex::new(HitStream {
                out: BufWriter::with_capacity(64 * 1024, io::stdout()),
                error: None,
            }),
        }
    }

    fn on_event(&self, event: SearchEvent) {
        match event {
            SearchEvent::Hits(hits) => self.on_hits(&hits),
            SearchEvent::Unreadable(folder) => self.on_unreadable(&folder),
        }
    }

    fn on_hits(&self, hits: &[Hit]) {
        self.first_result.get_or_init(|| self.started.elapsed());
        self.matches.fetch_add(hits.len() as u64, Ordering::Relaxed);
        if !self.quiet {
            self.write_hits(hits);
        }
    }

    fn write_hits(&self, hits: &[Hit]) {
        let block = hit_block(hits);
        let mut stream = self.stdout.lock().unwrap();
        if stream.error.is_some() {
            return;
        }
        if let Err(e) = stream.out.write_all(&block) {
            stream.error = Some(e);
        }
    }

    fn on_unreadable(&self, folder: &UnreadableFolder) {
        if !self.quiet {
            eprintln!("unreadable: {}: {}", folder.path.display(), folder.error);
        }
    }

    fn finish(&self) -> io::Result<()> {
        let mut stream = self.stdout.lock().unwrap();
        if let Some(e) = stream.error.take() {
            return Err(e);
        }
        stream.out.flush()
    }

    fn matches(&self) -> u64 {
        self.matches.load(Ordering::Relaxed)
    }

    fn first_result(&self) -> Option<Duration> {
        self.first_result.get().copied()
    }
}

/// One path per line, raw bytes on Unix; sized up front so a block is one allocation.
fn hit_block(hits: &[Hit]) -> Vec<u8> {
    let len: usize = hits
        .iter()
        .map(|h| h.path.as_os_str().as_encoded_bytes().len() + 1)
        .sum();
    let mut block = Vec::with_capacity(len);
    for hit in hits {
        block.extend_from_slice(&os_bytes(hit.path.as_os_str()));
        block.push(b'\n');
    }
    block
}

fn print_summary(sink: &Sink, control: &WalkControl, walked: &Walked, elapsed: Duration) {
    eprintln!(
        "{} · {} walked{} · {} · total {} · {}",
        plural(sink.matches(), "match", "matches"),
        plural(control.dirs_read(), "dir", "dirs"),
        unreadable_note(control.unreadable()),
        first_result_note(sink.first_result()),
        fmt_duration(elapsed),
        fmt_env(walked.storage, &walked.strategy),
    );
}

fn unreadable_note(count: u64) -> String {
    if count == 0 {
        String::new()
    } else {
        format!(", {count} unreadable")
    }
}

fn first_result_note(first: Option<Duration>) -> String {
    first.map_or("no match".to_owned(), |d| {
        format!("first result {}", fmt_duration(d))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use fx_core::EntryKind;

    #[test]
    fn hit_block_is_one_path_per_line() {
        let hits = [
            Hit {
                path: PathBuf::from("/r/a.txt"),
                kind: EntryKind::File,
                hidden: false,
            },
            Hit {
                path: PathBuf::from("/r/b"),
                kind: EntryKind::Dir,
                hidden: true,
            },
        ];
        assert_eq!(hit_block(&hits), b"/r/a.txt\n/r/b\n");
    }

    #[test]
    fn summary_notes() {
        assert_eq!(unreadable_note(0), "");
        assert_eq!(unreadable_note(2), ", 2 unreadable");
        assert_eq!(first_result_note(None), "no match");
    }
}
