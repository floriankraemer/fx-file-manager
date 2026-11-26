//! The freedesktop thumbnailer D-Bus service (tumbler, also behind GNOME and KDE setups). It renders
//! videos, PDFs and other non-images into the shared cache; fx then reads them like any cached thumbnail.

use std::collections::{HashSet, VecDeque};
use std::path::Path;
use std::sync::{Condvar, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};
use zbus::blocking::{Connection, MessageIterator};

const DEST: &str = "org.freedesktop.thumbnails.Thumbnailer1";
const PATH: &str = "/org/freedesktop/thumbnails/Thumbnailer1";
/// A video thumbnailer can be slow, but a hung one must not hold a worker forever.
const TIMEOUT: Duration = Duration::from_secs(10);
/// `Finished` is broadcast for every client's requests; only the latest ones are kept.
const RECENT: usize = 256;

pub(crate) struct Tumbler {
    connection: Connection,
    mime_types: HashSet<String>,
    finished: Mutex<Recent>,
    signal: Condvar,
}

/// Handles of recently finished requests, oldest first.
#[derive(Default)]
struct Recent(VecDeque<u32>);

impl Recent {
    fn push(&mut self, handle: u32) {
        if self.0.len() == RECENT {
            self.0.pop_front();
        }
        self.0.push_back(handle);
    }

    /// True once, if `handle` has finished.
    fn take(&mut self, handle: u32) -> bool {
        let Some(at) = self.0.iter().position(|h| *h == handle) else {
            return false;
        };
        self.0.remove(at);
        true
    }
}

/// The session's thumbnailer, connected on first use; `None` without a session bus or service.
pub(crate) fn shared() -> Option<&'static Tumbler> {
    static TUMBLER: OnceLock<Option<Tumbler>> = OnceLock::new();
    let tumbler = TUMBLER.get_or_init(|| match Tumbler::connect() {
        Ok(t) => Some(t),
        Err(e) => {
            log::debug!("no thumbnailer service: {e}");
            None
        }
    });
    let tumbler = tumbler.as_ref()?;
    tumbler.listen();
    Some(tumbler)
}

impl Tumbler {
    fn connect() -> zbus::Result<Self> {
        let connection = Connection::session()?;
        let reply = connection.call_method(Some(DEST), PATH, Some(DEST), "GetSupported", &())?;
        let (_schemes, mime_types): (Vec<String>, Vec<String>) = reply.body().deserialize()?;
        Ok(Self {
            connection,
            mime_types: mime_types.into_iter().collect(),
            finished: Mutex::new(Recent::default()),
            signal: Condvar::new(),
        })
    }

    /// Starts the one thread that records `Finished` signals.
    fn listen(&'static self) {
        static STARTED: OnceLock<()> = OnceLock::new();
        STARTED.get_or_init(|| {
            let rule = zbus::MatchRule::builder()
                .msg_type(zbus::message::Type::Signal)
                .interface(DEST)
                .and_then(|r| r.member("Finished"))
                .map(|r| r.build());
            let messages =
                rule.and_then(|rule| MessageIterator::for_match_rule(rule, &self.connection, None));
            match messages {
                Ok(messages) => drop(thread::spawn(move || self.record(messages))),
                Err(e) => log::debug!("cannot watch the thumbnailer: {e}"),
            }
        });
    }

    fn record(&self, messages: MessageIterator) {
        for message in messages.flatten() {
            let Ok(handle) = message.body().deserialize::<u32>() else {
                continue;
            };
            self.finished
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(handle);
            self.signal.notify_all();
        }
    }

    /// The MIME type the service would render `path` as, judged by its extension.
    pub(crate) fn mime_type(&self, path: &Path) -> Option<&'static str> {
        let mime = mime_guess::from_path(path).first_raw()?;
        self.mime_types.contains(mime).then_some(mime)
    }

    /// Asks the service to render `uri` at `flavor` and waits until it is done, successful or not.
    pub(crate) fn generate(&self, uri: &str, mime: &str, flavor: &str) -> zbus::Result<bool> {
        let args = (vec![uri], vec![mime], flavor, "foreground", 0u32);
        let reply = self
            .connection
            .call_method(Some(DEST), PATH, Some(DEST), "Queue", &args)?;
        let handle: u32 = reply.body().deserialize()?;
        Ok(self.wait(handle))
    }

    fn wait(&self, handle: u32) -> bool {
        let deadline = Instant::now() + TIMEOUT;
        let mut finished = self.finished.lock().unwrap_or_else(|e| e.into_inner());
        while !finished.take(handle) {
            let Some(left) = deadline.checked_duration_since(Instant::now()) else {
                return false;
            };
            finished = match self.signal.wait_timeout(finished, left) {
                Ok((guard, _)) => guard,
                Err(e) => e.into_inner().0,
            };
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recent_handles_are_taken_once_and_bounded() {
        let mut recent = Recent::default();
        for h in 0..(RECENT as u32 + 10) {
            recent.push(h);
        }
        assert!(!recent.take(3), "the oldest handles are dropped");
        assert!(recent.take(RECENT as u32 + 9));
        assert!(!recent.take(RECENT as u32 + 9));
        assert_eq!(recent.0.len(), RECENT - 1);
    }
}
