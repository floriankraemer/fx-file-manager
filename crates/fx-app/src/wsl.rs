//! Installed WSL distribution names, read from the per-user registry. Any failure yields nothing.

use std::ptr;
use windows_sys::Win32::Foundation::ERROR_SUCCESS;
use windows_sys::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_READ, RRF_RT_REG_SZ, RegCloseKey, RegEnumKeyExW, RegGetValueW,
    RegOpenKeyExW,
};

const LXSS: &str = r"Software\Microsoft\Windows\CurrentVersion\Lxss";
const BUF_LEN: usize = 256;

struct Key(HKEY);

impl Drop for Key {
    fn drop(&mut self) {
        // SAFETY: the key was opened by RegOpenKeyExW and is closed exactly once.
        unsafe { RegCloseKey(self.0) };
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

fn open(path: &str) -> Option<Key> {
    let mut key: HKEY = ptr::null_mut();
    let path = wide(path);
    // SAFETY: `path` is NUL-terminated and `key` is a valid out pointer.
    let status = unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, path.as_ptr(), 0, KEY_READ, &mut key) };
    (status == ERROR_SUCCESS).then_some(Key(key))
}

fn subkey_name(parent: &Key, index: u32) -> Option<String> {
    let mut buf = [0u16; BUF_LEN];
    let mut len = BUF_LEN as u32;
    // SAFETY: `buf` holds `len` UTF-16 units; the optional out parameters are null.
    let status = unsafe {
        RegEnumKeyExW(
            parent.0,
            index,
            buf.as_mut_ptr(),
            &mut len,
            ptr::null(),
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
        )
    };
    (status == ERROR_SUCCESS).then(|| String::from_utf16_lossy(&buf[..len as usize]))
}

fn distribution_name(subkey: &str) -> Option<String> {
    let mut buf = [0u16; BUF_LEN];
    let mut bytes = std::mem::size_of_val(&buf) as u32;
    let key = open(&format!("{LXSS}\\{subkey}"))?;
    let value = wide("DistributionName");
    // SAFETY: `buf` provides `bytes` writable bytes; `value` is NUL-terminated.
    let status = unsafe {
        RegGetValueW(
            key.0,
            ptr::null(),
            value.as_ptr(),
            RRF_RT_REG_SZ,
            ptr::null_mut(),
            buf.as_mut_ptr().cast(),
            &mut bytes,
        )
    };
    if status != ERROR_SUCCESS {
        return None;
    }
    // `bytes` counts the terminating NUL.
    let units = (bytes as usize / 2).saturating_sub(1).min(BUF_LEN);
    Some(String::from_utf16_lossy(&buf[..units]))
}

pub(crate) fn distro_names() -> Vec<String> {
    let Some(lxss) = open(LXSS) else {
        return Vec::new();
    };
    (0..)
        .map_while(|index| subkey_name(&lxss, index))
        .filter_map(|subkey| distribution_name(&subkey))
        .collect()
}
