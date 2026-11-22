use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// Append-only log file with a hard size budget: `max_bytes` per file, `keep` rotated generations.
/// The directory never holds more than `(keep + 1) * max_bytes` bytes of log, whatever is logged.
pub(super) struct RotatingFile {
    path: PathBuf,
    file: Option<File>,
    written: u64,
    max_bytes: u64,
    keep: usize,
}

impl RotatingFile {
    pub(super) fn open(path: PathBuf, max_bytes: u64, keep: usize) -> io::Result<Self> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let file = open_append(&path)?;
        let written = file.metadata().map_or(0, |m| m.len());
        let mut log = Self {
            path,
            file: Some(file),
            written,
            max_bytes,
            keep,
        };
        if log.written >= max_bytes {
            log.rotate();
        }
        Ok(log)
    }

    /// Writes one whole line; a failed write is dropped, because logging must never take the app down.
    pub(super) fn write_line(&mut self, line: &[u8]) {
        if self.written + line.len() as u64 > self.max_bytes {
            self.rotate();
        }
        let Some(file) = self.file.as_mut() else {
            return;
        };
        if file.write_all(line).is_ok() {
            self.written += line.len() as u64;
        }
    }

    /// Shifts `fx.log` to `fx.1.log` and so on, dropping the oldest. When renaming fails (another
    /// process holds the file on a filesystem that forbids it) the current file is truncated
    /// instead, and when even that fails logging stops, so the size budget holds in every case.
    fn rotate(&mut self) {
        self.file = None;
        self.written = 0;
        if self.shift_generations().is_ok()
            && let Ok(file) = open_append(&self.path)
        {
            self.file = Some(file);
            return;
        }
        self.file = truncate(&self.path).ok();
    }

    fn shift_generations(&self) -> io::Result<()> {
        if self.keep == 0 {
            return truncate(&self.path).map(drop);
        }
        let oldest = self.generation(self.keep);
        if oldest.exists() {
            fs::remove_file(&oldest)?;
        }
        for n in (1..self.keep).rev() {
            let from = self.generation(n);
            if from.exists() {
                fs::rename(&from, self.generation(n + 1))?;
            }
        }
        fs::rename(&self.path, self.generation(1))
    }

    fn generation(&self, n: usize) -> PathBuf {
        let stem = self.path.file_stem().unwrap_or_default().to_string_lossy();
        self.path.with_file_name(format!("{stem}.{n}.log"))
    }
}

fn open_append(path: &Path) -> io::Result<File> {
    OpenOptions::new().create(true).append(true).open(path)
}

fn truncate(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir_bytes(dir: &Path) -> (usize, u64) {
        fs::read_dir(dir)
            .unwrap()
            .flatten()
            .fold((0, 0), |(n, b), e| (n + 1, b + e.metadata().unwrap().len()))
    }

    #[test]
    fn rotation_keeps_the_directory_within_budget() {
        let dir = tempfile::tempdir().unwrap();
        let mut log = RotatingFile::open(dir.path().join("fx.log"), 1_000, 2).unwrap();
        for _ in 0..10_000 {
            log.write_line(b"0123456789012345678901234567890123456789\n");
        }
        let (files, bytes) = dir_bytes(dir.path());
        assert_eq!(files, 3);
        assert!(bytes <= 3_000, "{bytes} bytes on disk");
        assert!(dir.path().join("fx.2.log").exists());
    }

    #[test]
    fn reopening_a_full_log_rotates_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fx.log");
        fs::write(&path, vec![b'x'; 2_000]).unwrap();
        let mut log = RotatingFile::open(path.clone(), 1_000, 1).unwrap();
        log.write_line(b"fresh\n");
        assert_eq!(fs::read(&path).unwrap(), b"fresh\n");
        assert_eq!(
            fs::metadata(dir.path().join("fx.1.log")).unwrap().len(),
            2_000
        );
    }

    #[test]
    fn keep_zero_truncates_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fx.log");
        let mut log = RotatingFile::open(path.clone(), 10, 0).unwrap();
        log.write_line(b"123456\n");
        log.write_line(b"abcdef\n");
        assert_eq!(fs::read(&path).unwrap(), b"abcdef\n");
        assert_eq!(dir_bytes(dir.path()).0, 1);
    }
}
