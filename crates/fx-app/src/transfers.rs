//! Copy and move jobs as the UI sees them: a device-aware queue, throughput/ETA, and the finished notice.

use crate::ops::{Notice, Outcome, items};
use fx_core::{
    Conflict, CopyError, Op, Request, Resolution, RunState, Snapshot, StorageKind, Summary,
    Transfer, VolumeInfo, volume_info,
};
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Throughput is averaged over this window.
const RATE_WINDOW: Duration = Duration::from_secs(3);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransferState {
    /// Waits for a running transfer on the same HDD or removable volume.
    Queued,
    Running,
    Paused,
    /// Paused because the destination is full: retry (resume), skip or cancel.
    DiskFull,
    Finished,
}

/// One row of the transfer panel.
#[derive(Clone, Debug)]
pub struct TransferInfo {
    pub id: u64,
    /// "Copying 3 items to Documents".
    pub label: String,
    pub state: TransferState,
    pub snapshot: Option<Snapshot>,
    /// Bytes per second, averaged.
    pub rate: f64,
    pub eta: Option<Duration>,
    /// The oldest conflict waiting for an answer.
    pub conflict: Option<Conflict>,
    /// Errors and warnings so far.
    pub errors: Vec<CopyError>,
    pub summary: Option<Summary>,
}

/// Exponential moving average of bytes/s and files/s.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Rate {
    last: Option<(Instant, u64, u64)>,
    bytes: f64,
    files: f64,
}

impl Rate {
    pub(crate) fn update(&mut self, now: Instant, bytes: u64, files: u64) {
        if let Some((then, b, f)) = self.last {
            let dt = now.saturating_duration_since(then).as_secs_f64();
            if dt <= 0.0 {
                return;
            }
            let alpha = 1.0 - (-dt / RATE_WINDOW.as_secs_f64()).exp();
            self.bytes += alpha * ((bytes.saturating_sub(b)) as f64 / dt - self.bytes);
            self.files += alpha * ((files.saturating_sub(f)) as f64 / dt - self.files);
        }
        self.last = Some((now, bytes, files));
    }

    /// The slower of the byte and the file estimate: small-file jobs are bound by files per second.
    /// `None` until the totals are final or while nothing moves.
    pub(crate) fn eta(&self, s: &Snapshot) -> Option<Duration> {
        if !s.walk_complete {
            return None;
        }
        let by_bytes = (self.bytes > 0.0)
            .then(|| s.bytes_total.saturating_sub(s.bytes_done) as f64 / self.bytes);
        let by_files = (self.files > 0.0)
            .then(|| s.files_total.saturating_sub(s.files_done) as f64 / self.files);
        let secs = match (by_bytes, by_files) {
            (Some(a), Some(b)) => a.max(b),
            (a, b) => a.or(b)?,
        };
        secs.is_finite().then(|| Duration::from_secs_f64(secs))
    }
}

/// HDDs and USB sticks thrash when two jobs share them; SSDs and shares do not mind.
fn exclusive(v: &VolumeInfo) -> bool {
    matches!(v.storage, StorageKind::Hdd | StorageKind::Removable)
}

/// Whether a transfer touching `new` must wait for one touching `running`.
pub(crate) fn must_wait(new: &[VolumeInfo], running: &[VolumeInfo]) -> bool {
    new.iter()
        .any(|a| exclusive(a) && running.iter().any(|b| b.id == a.id))
}

struct Entry {
    id: u64,
    label: String,
    volumes: Vec<VolumeInfo>,
    /// Directories whose listings change, reloaded when the transfer ends.
    touched: Vec<PathBuf>,
    queued: Option<Request>,
    running: Option<Transfer>,
    summary: Option<Summary>,
    rate: Rate,
}

#[derive(Default)]
pub(crate) struct Transfers {
    entries: Vec<Entry>,
    next_id: u64,
}

impl Transfers {
    /// Queues `req`; it starts at the next `poll` unless it has to wait for its volumes.
    pub(crate) fn submit(&mut self, req: Request, label: String) {
        let mut dirs: BTreeSet<PathBuf> = BTreeSet::new();
        for item in &req.items {
            dirs.extend(item.src.parent().map(PathBuf::from));
            dirs.extend(item.dst.parent().map(PathBuf::from));
        }
        let volumes = dirs.iter().map(|d| volume_info(d)).collect();
        let id = self.next_id;
        self.next_id += 1;
        self.entries.push(Entry {
            id,
            label,
            volumes,
            touched: dirs.into_iter().collect(),
            queued: Some(req),
            running: None,
            summary: None,
            rate: Rate::default(),
        });
    }

    /// Starts what may start, collects what finished. Returns outcomes to apply and whether anything changed.
    pub(crate) fn poll(&mut self, wake: &Arc<dyn Fn() + Send + Sync>) -> (Vec<Outcome>, bool) {
        let now = Instant::now();
        let mut outcomes = Vec::new();
        let mut changed = false;
        for e in &mut self.entries {
            if let Some(t) = &e.running {
                let s = t.handle().snapshot();
                e.rate.update(now, s.bytes_done, s.files_done);
                if t.is_finished() {
                    let summary = e.running.take().expect("checked above").join();
                    outcomes.push(Outcome {
                        notice: notice(&summary, &e.label),
                        touched: e.touched.clone(),
                        created: None,
                    });
                    e.summary = Some(summary);
                    changed = true;
                }
            }
        }
        // Clean results are reported by the notice alone; the rest stays in the panel until dismissed.
        self.entries.retain(|e| {
            e.summary
                .as_ref()
                .is_none_or(|s| s.cancelled || s.error_count() > 0 || !s.kept_sources.is_empty())
        });
        for i in 0..self.entries.len() {
            if self.entries[i].queued.is_none() {
                continue;
            }
            let busy: Vec<VolumeInfo> = self
                .entries
                .iter()
                .filter(|e| e.running.is_some())
                .flat_map(|e| e.volumes.iter().copied())
                .collect();
            if must_wait(&self.entries[i].volumes, &busy) {
                continue;
            }
            let e = &mut self.entries[i];
            let req = e.queued.take().expect("checked above");
            let wake = Arc::clone(wake);
            e.running = Some(Transfer::start(req, move || wake()));
            changed = true;
        }
        (outcomes, changed)
    }

    pub(crate) fn active(&self) -> bool {
        self.entries
            .iter()
            .any(|e| e.running.is_some() || e.queued.is_some())
    }

    pub(crate) fn infos(&self) -> Vec<TransferInfo> {
        self.entries
            .iter()
            .map(|e| {
                let handle = e.running.as_ref().map(Transfer::handle);
                let snapshot = handle.map(|h| h.snapshot());
                let state = match (&e.queued, &snapshot) {
                    (Some(_), _) => TransferState::Queued,
                    (None, Some(s)) if s.disk_full => TransferState::DiskFull,
                    (None, Some(s)) if s.state == RunState::Paused => TransferState::Paused,
                    (None, Some(_)) => TransferState::Running,
                    (None, None) => TransferState::Finished,
                };
                TransferInfo {
                    id: e.id,
                    label: e.label.clone(),
                    state,
                    eta: snapshot.as_ref().and_then(|s| e.rate.eta(s)),
                    rate: e.rate.bytes,
                    snapshot,
                    conflict: handle.and_then(|h| h.conflicts().into_iter().next()),
                    errors: match (handle, &e.summary) {
                        (Some(h), _) => h.report(),
                        (None, Some(s)) => s.errors.clone(),
                        (None, None) => Vec::new(),
                    },
                    summary: e.summary.clone(),
                }
            })
            .collect()
    }

    fn running(&self, id: u64) -> Option<&Transfer> {
        self.entries
            .iter()
            .find(|e| e.id == id)
            .and_then(|e| e.running.as_ref())
    }

    pub(crate) fn pause(&self, id: u64) {
        if let Some(t) = self.running(id) {
            t.handle().pause();
        }
    }

    pub(crate) fn resume(&self, id: u64) {
        if let Some(t) = self.running(id) {
            t.handle().resume();
        }
    }

    pub(crate) fn skip_disk_full(&self, id: u64) {
        if let Some(t) = self.running(id) {
            t.handle().skip_disk_full();
        }
    }

    /// A queued transfer is dropped; a running one stops at the next chunk.
    pub(crate) fn cancel(&mut self, id: u64) {
        self.entries.retain(|e| e.id != id || e.queued.is_none());
        if let Some(t) = self.running(id) {
            t.handle().cancel();
        }
    }

    pub(crate) fn answer(
        &self,
        id: u64,
        conflict: u64,
        resolution: Resolution,
        apply_to_all: bool,
    ) {
        if let Some(t) = self.running(id) {
            t.handle()
                .answer_conflict(conflict, resolution, apply_to_all);
        }
    }

    /// Removes a finished transfer from the panel.
    pub(crate) fn dismiss(&mut self, id: u64) {
        self.entries.retain(|e| e.id != id || e.summary.is_none());
    }
}

/// The one-line result: "Copied 3 items", "Moved 2 of 3 items. 4 errors: <first>", "Cancelled ...".
pub(crate) fn notice(s: &Summary, label: &str) -> Notice {
    let verb = match s.op {
        Op::Copy => "Copied",
        Op::Move => "Moved",
    };
    if s.cancelled {
        return Notice::info(format!(
            "Cancelled {}: {} files done",
            lowercase_first(label),
            s.files
        ));
    }
    let mut extra = Vec::new();
    if s.skipped > 0 {
        extra.push(format!("{} skipped", s.skipped));
    }
    if s.cloned > 0 {
        extra.push(format!("{} cloned", s.cloned));
    }
    let extra = if extra.is_empty() {
        String::new()
    } else {
        format!(" ({})", extra.join(", "))
    };
    let errors = s.error_count();
    if errors == 0 && s.kept_sources.is_empty() {
        return Notice::info(format!("{verb} {}{extra}", items(s.items)));
    }
    let first = s
        .errors
        .iter()
        .find(|e| !e.warning)
        .map(|e| format!(" First: {e}"))
        .unwrap_or_default();
    let kept = if s.kept_sources.is_empty() {
        String::new()
    } else {
        format!(" {} kept in place.", items(s.kept_sources.len()))
    };
    let failed = if errors == 1 {
        "1 error".to_owned()
    } else {
        format!("{errors} errors")
    };
    Notice::error(format!(
        "{verb} {} of {}{extra}. {failed}.{kept}{first}",
        s.items_done,
        items(s.items),
    ))
}

fn lowercase_first(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map(|f| f.to_lowercase().chain(c).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use fx_core::{ErrorOp, FsCaps, FsKind};

    fn vol(id: u64, storage: StorageKind) -> VolumeInfo {
        VolumeInfo {
            id,
            fs: FsKind::Ext4,
            storage,
            caps: FsCaps::for_fs(FsKind::Ext4),
        }
    }

    #[test]
    fn only_shared_hdd_or_usb_volumes_queue() {
        let hdd = vol(1, StorageKind::Hdd);
        let usb = vol(2, StorageKind::Removable);
        let ssd = vol(3, StorageKind::Ssd);
        assert!(must_wait(&[ssd, hdd], &[hdd]));
        assert!(must_wait(&[usb], &[ssd, usb]));
        assert!(
            !must_wait(&[ssd], &[ssd]),
            "SSDs run transfers concurrently"
        );
        assert!(
            !must_wait(&[hdd], &[vol(4, StorageKind::Hdd)]),
            "different disks"
        );
        assert!(!must_wait(&[hdd], &[]));
    }

    #[test]
    fn rate_averages_and_eta_takes_the_slower_estimate() {
        let t0 = Instant::now();
        let mut r = Rate::default();
        r.update(t0, 0, 0);
        for i in 1..=30 {
            r.update(t0 + Duration::from_secs(i), i * 1000, i * 10);
        }
        assert!((r.bytes - 1000.0).abs() < 10.0, "{}", r.bytes);
        let snap = |walk_complete| Snapshot {
            bytes_done: 30_000,
            files_done: 300,
            bytes_total: 40_000,
            files_total: 600,
            errors: 0,
            skipped: 0,
            cloned: 0,
            walk_complete,
            state: RunState::Running,
            disk_full: false,
            pending_conflicts: 0,
            current: None,
            elapsed: Duration::from_secs(30),
        };
        assert_eq!(r.eta(&snap(false)), None, "totals still grow");
        let eta = r.eta(&snap(true)).unwrap().as_secs_f64();
        // 10 s by bytes, 30 s by files.
        assert!((eta - 30.0).abs() < 1.0, "{eta}");
    }

    fn summary(op: Op) -> Summary {
        Summary {
            op,
            items: 3,
            items_done: 3,
            files: 10,
            bytes: 100,
            cloned: 0,
            skipped: 0,
            errors: Vec::new(),
            cancelled: false,
            kept_sources: Vec::new(),
            plans: Vec::new(),
            elapsed: Duration::ZERO,
        }
    }

    #[test]
    fn notices_summarize_the_outcome() {
        assert_eq!(notice(&summary(Op::Copy), "x").text, "Copied 3 items");
        let mut s = summary(Op::Move);
        s.skipped = 2;
        assert_eq!(notice(&s, "x").text, "Moved 3 items (2 skipped)");
        s.items_done = 2;
        s.kept_sources = vec![PathBuf::from("/a")];
        s.errors.push(CopyError {
            path: PathBuf::from("/a/f"),
            op: ErrorOp::Copy,
            kind: std::io::ErrorKind::PermissionDenied,
            os_code: Some(13),
            message: "Permission denied".into(),
            warning: false,
        });
        let n = notice(&s, "x");
        assert_eq!(n.kind, crate::NoticeKind::Error);
        assert_eq!(
            n.text,
            "Moved 2 of 3 items (2 skipped). 1 error. 1 item kept in place. First: /a/f: could not copy: Permission denied"
        );
        let mut c = summary(Op::Copy);
        c.cancelled = true;
        assert_eq!(
            notice(&c, "Copying 3 items to b").text,
            "Cancelled copying 3 items to b: 10 files done"
        );
    }
}
