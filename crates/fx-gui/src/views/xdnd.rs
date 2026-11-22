//! Drags files out of fx into other X11 programs by acting as an XDND source (protocol version 5).
//! winit offers no way to start a drag, so this talks to the X server on its own connection:
//! it follows the pointer, announces the files to the window below it and serves them as a URI list.

use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};
use x11rb::connection::Connection;
use x11rb::errors::ReplyError;
use x11rb::protocol::Event;
use x11rb::protocol::xproto::{
    AtomEnum, ClientMessageEvent, ConnectionExt, CreateWindowAux, EventMask, KeyButMask, PropMode,
    SELECTION_NOTIFY_EVENT, SelectionNotifyEvent, SelectionRequestEvent, Time, Window, WindowClass,
};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;
use x11rb::{COPY_DEPTH_FROM_PARENT, NONE};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const VERSION: u32 = 5;
const POLL: Duration = Duration::from_millis(8);
const FINISH_TIMEOUT: Duration = Duration::from_secs(5);

/// Starts dragging `paths` with the pointer until the button is released. Returns at once.
pub fn start(paths: Vec<PathBuf>) {
    thread::spawn(move || {
        if let Err(err) = drag(&paths) {
            log::warn!("starting the drag failed: {err}");
        }
    });
}

fn drag(paths: &[PathBuf]) -> Result<()> {
    let mut source = Source::connect(uri_list(paths))?;
    if let Err(err) = source.follow_until_released() {
        let _ = source.leave();
        return Err(err);
    }
    source.release()
}

/// `text/uri-list` body: one percent-encoded `file://` URI per line, each ended by CRLF.
fn uri_list(paths: &[PathBuf]) -> Vec<u8> {
    let mut out = Vec::new();
    for path in paths {
        out.extend_from_slice(b"file://");
        out.extend_from_slice(&percent_encode(path.as_os_str()));
        out.extend_from_slice(b"\r\n");
    }
    out
}

fn percent_encode(name: &OsStr) -> Vec<u8> {
    let mut out = Vec::new();
    for &byte in name.as_bytes() {
        if byte.is_ascii_alphanumeric() || b"/-._~".contains(&byte) {
            out.push(byte);
        } else {
            out.extend_from_slice(format!("%{byte:02X}").as_bytes());
        }
    }
    out
}

/// `XdndPosition` carries the root coordinates as two 16-bit halves of one word.
fn pack_position(x: i16, y: i16) -> u32 {
    (u32::from(x as u16) << 16) | u32::from(y as u16)
}

struct Atoms {
    aware: u32,
    enter: u32,
    position: u32,
    status: u32,
    leave: u32,
    drop: u32,
    finished: u32,
    selection: u32,
    copy: u32,
    uri_list: u32,
    targets: u32,
}

impl Atoms {
    fn intern(conn: &RustConnection) -> Result<Self> {
        let atom =
            |name: &[u8]| -> Result<u32> { Ok(conn.intern_atom(false, name)?.reply()?.atom) };
        Ok(Self {
            aware: atom(b"XdndAware")?,
            enter: atom(b"XdndEnter")?,
            position: atom(b"XdndPosition")?,
            status: atom(b"XdndStatus")?,
            leave: atom(b"XdndLeave")?,
            drop: atom(b"XdndDrop")?,
            finished: atom(b"XdndFinished")?,
            selection: atom(b"XdndSelection")?,
            copy: atom(b"XdndActionCopy")?,
            uri_list: atom(b"text/uri-list")?,
            targets: atom(b"TARGETS")?,
        })
    }
}

/// The window the pointer is over that understands XDND.
#[derive(Clone, Copy, PartialEq)]
struct Target {
    window: Window,
    version: u32,
}

struct Source {
    conn: RustConnection,
    root: Window,
    window: Window,
    atoms: Atoms,
    data: Vec<u8>,
    target: Option<Target>,
    accepted: bool,
    finished: bool,
    last_position: Option<(i16, i16)>,
}

impl Source {
    /// Connects and takes ownership of `XdndSelection` through a hidden window of our own.
    fn connect(data: Vec<u8>) -> Result<Self> {
        let (conn, screen) = x11rb::connect(None)?;
        let root = conn.setup().roots[screen].root;
        let window = conn.generate_id()?;
        conn.create_window(
            COPY_DEPTH_FROM_PARENT,
            window,
            root,
            0,
            0,
            1,
            1,
            0,
            WindowClass::INPUT_ONLY,
            0,
            &CreateWindowAux::new(),
        )?;
        let atoms = Atoms::intern(&conn)?;
        conn.set_selection_owner(window, atoms.selection, Time::CURRENT_TIME)?;
        conn.flush()?;
        Ok(Self {
            conn,
            root,
            window,
            atoms,
            data,
            target: None,
            accepted: false,
            finished: false,
            last_position: None,
        })
    }

    fn follow_until_released(&mut self) -> Result<()> {
        while self.button_down()? {
            self.pump()?;
            self.follow_pointer()?;
            thread::sleep(POLL);
        }
        Ok(())
    }

    fn button_down(&self) -> Result<bool> {
        let pointer = self.conn.query_pointer(self.root)?.reply()?;
        Ok(pointer.mask.contains(KeyButMask::BUTTON1))
    }

    /// Handles what the target and the selection protocol send back.
    fn pump(&mut self) -> Result<()> {
        while let Some(event) = self.conn.poll_for_event()? {
            self.handle(event)?;
        }
        Ok(())
    }

    fn handle(&mut self, event: Event) -> Result<()> {
        match event {
            Event::ClientMessage(msg) if msg.type_ == self.atoms.status => {
                self.accepted = msg.data.as_data32()[1] & 1 == 1;
            }
            Event::ClientMessage(msg) if msg.type_ == self.atoms.finished => {
                self.finished = true;
            }
            Event::SelectionRequest(request) => self.serve(&request)?,
            _ => {}
        }
        Ok(())
    }

    /// Answers a request for the dragged data, or for the formats it is offered in.
    fn serve(&self, request: &SelectionRequestEvent) -> Result<()> {
        let property = if request.property == NONE {
            request.target
        } else {
            request.property
        };
        let served = self.store(request, property)?;
        let notify = SelectionNotifyEvent {
            response_type: SELECTION_NOTIFY_EVENT,
            sequence: 0,
            time: request.time,
            requestor: request.requestor,
            selection: request.selection,
            target: request.target,
            property: if served { property } else { NONE },
        };
        self.conn
            .send_event(false, request.requestor, EventMask::NO_EVENT, notify)?;
        self.conn.flush()?;
        Ok(())
    }

    /// Writes the answer into the requestor's property; false when the format is unknown.
    fn store(&self, request: &SelectionRequestEvent, property: u32) -> Result<bool> {
        let (requestor, atoms) = (request.requestor, &self.atoms);
        if request.target == atoms.targets {
            let formats = [atoms.targets, atoms.uri_list];
            self.conn.change_property32(
                PropMode::REPLACE,
                requestor,
                property,
                AtomEnum::ATOM,
                &formats,
            )?;
            return Ok(true);
        }
        if request.target != atoms.uri_list {
            return Ok(false);
        }
        self.conn.change_property8(
            PropMode::REPLACE,
            requestor,
            property,
            atoms.uri_list,
            &self.data,
        )?;
        Ok(true)
    }

    /// Moves the drag to the window under the pointer and tells it where the pointer is.
    fn follow_pointer(&mut self) -> Result<()> {
        let pointer = self.conn.query_pointer(self.root)?.reply()?;
        let at = (pointer.root_x, pointer.root_y);
        let found = self.target_at(at)?;
        if found != self.target {
            self.leave()?;
            self.enter(found)?;
        }
        if self.target.is_some() && self.last_position != Some(at) {
            self.send_position(at)?;
        }
        Ok(())
    }

    /// The first window at `at`, from the top level down, that announces XDND support.
    /// A window that dies under the pointer mid-drag raises an X error and counts as no target.
    fn target_at(&self, at: (i16, i16)) -> Result<Option<Target>> {
        match self.find_target(at) {
            Err(err) if matches!(err.downcast_ref(), Some(ReplyError::X11Error(_))) => Ok(None),
            found => found,
        }
    }

    fn find_target(&self, at: (i16, i16)) -> Result<Option<Target>> {
        let mut window = self.root;
        loop {
            let child = self
                .conn
                .translate_coordinates(self.root, window, at.0, at.1)?
                .reply()?
                .child;
            if child == NONE {
                return Ok(None);
            }
            window = child;
            if let Some(version) = self.aware_version(window)? {
                return Ok(Some(Target { window, version }));
            }
        }
    }

    fn aware_version(&self, window: Window) -> Result<Option<u32>> {
        let reply = self
            .conn
            .get_property(false, window, self.atoms.aware, AtomEnum::ATOM, 0, 1)?
            .reply()?;
        let version = reply.value32().and_then(|mut v| v.next());
        Ok(version.map(|v| v.min(VERSION)))
    }

    fn enter(&mut self, target: Option<Target>) -> Result<()> {
        self.target = target;
        self.accepted = false;
        self.last_position = None;
        let Some(target) = target else {
            return Ok(());
        };
        let flags = target.version << 24;
        self.message(
            target,
            self.atoms.enter,
            [self.window, flags, self.atoms.uri_list, NONE, NONE],
        )
    }

    fn leave(&mut self) -> Result<()> {
        let Some(target) = self.target.take() else {
            return Ok(());
        };
        self.message(target, self.atoms.leave, [self.window, 0, 0, 0, 0])
    }

    fn send_position(&mut self, at: (i16, i16)) -> Result<()> {
        let Some(target) = self.target else {
            return Ok(());
        };
        self.last_position = Some(at);
        let data = [
            self.window,
            0,
            pack_position(at.0, at.1),
            Time::CURRENT_TIME.into(),
            self.atoms.copy,
        ];
        self.message(target, self.atoms.position, data)
    }

    fn message(&self, target: Target, kind: u32, data: [u32; 5]) -> Result<()> {
        let event = ClientMessageEvent::new(32, target.window, kind, data);
        self.conn
            .send_event(false, target.window, EventMask::NO_EVENT, event)?;
        self.conn.flush()?;
        Ok(())
    }

    /// Ends the drag: drops on a target that accepted, otherwise leaves it.
    fn release(mut self) -> Result<()> {
        let Some(target) = self.target.filter(|_| self.accepted) else {
            return self.leave();
        };
        let drop = [self.window, 0, Time::CURRENT_TIME.into(), 0, 0];
        self.message(target, self.atoms.drop, drop)?;
        self.serve_until_finished()
    }

    /// Keeps answering the target's requests for the data until it reports it is done.
    fn serve_until_finished(&mut self) -> Result<()> {
        let started = Instant::now();
        while !self.finished && started.elapsed() < FINISH_TIMEOUT {
            self.pump()?;
            thread::sleep(POLL);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uri_list_encodes_each_path_on_its_own_line() {
        let paths = [PathBuf::from("/a b/é.txt"), PathBuf::from("/x")];
        let list = String::from_utf8(uri_list(&paths)).unwrap();
        assert_eq!(list, "file:///a%20b/%C3%A9.txt\r\nfile:///x\r\n");
    }

    #[test]
    fn position_packs_x_above_y() {
        assert_eq!(pack_position(300, 200), (300 << 16) | 200);
    }

    #[test]
    fn negative_position_keeps_its_16_bit_halves() {
        assert_eq!(pack_position(-1, -2), 0xFFFF_FFFE);
    }
}
