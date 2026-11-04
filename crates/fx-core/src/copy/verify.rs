//! Optional verification: after a file is copied, both sides are read again and their xxh3 hashes compared.
//!
//! ponytail: the destination is usually served from the page cache, so this proves the copy engine wrote the
//! right bytes, not that the medium stored them. Reading uncached (O_DIRECT / `FILE_FLAG_NO_BUFFERING`) is
//! the upgrade if that guarantee is ever needed; it would also make verification slower.

use super::progress::Shared;
use std::cell::RefCell;
use std::fs::File;
use std::io::{self, Read};
use std::path::Path;
use xxhash_rust::xxh3::Xxh3;

const BUF_SIZE: usize = 1024 * 1024;

thread_local! {
    static BUF: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

/// Length and 128-bit xxh3 digest of the file at `path`.
fn digest(sh: &Shared, path: &Path) -> io::Result<(u64, u128)> {
    let mut file = File::open(path)?;
    let mut hasher = Xxh3::new();
    let mut len = 0u64;
    BUF.with_borrow_mut(|buf| {
        buf.resize(BUF_SIZE, 0);
        loop {
            sh.checkpoint()?;
            match file.read(buf) {
                Ok(0) => return Ok(()),
                Ok(n) => {
                    hasher.update(&buf[..n]);
                    len += n as u64;
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
    })?;
    Ok((len, hasher.digest128()))
}

/// Fails with `InvalidData` when the two files differ in length or content.
pub(crate) fn check(sh: &Shared, src: &Path, dst: &Path) -> io::Result<()> {
    if digest(sh, src)? == digest(sh, dst)? {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "the copy does not match the source (checksum mismatch)",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equal_files_pass_and_any_difference_fails() {
        let t = tempfile::tempdir().unwrap();
        let sh = Shared::new(Box::new(|| {}));
        let data: Vec<u8> = (0..3_000_000u32).map(|i| (i % 253) as u8).collect();
        let (a, b) = (t.path().join("a"), t.path().join("b"));
        std::fs::write(&a, &data).unwrap();
        std::fs::write(&b, &data).unwrap();
        check(&sh, &a, &b).unwrap();

        let mut flipped = data.clone();
        flipped[2_999_999] ^= 1;
        std::fs::write(&b, &flipped).unwrap();
        assert_eq!(
            check(&sh, &a, &b).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );

        std::fs::write(&b, &data[..data.len() - 1]).unwrap();
        assert_eq!(
            check(&sh, &a, &b).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }
}
