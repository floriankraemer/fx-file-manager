//! Embeds the vendored Material icon theme into the binary.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};
use std::{env, fs};

fn walk(dir: &Path, rel: &str, out: &mut Vec<(String, PathBuf)>) {
    for e in fs::read_dir(dir).expect("read icon dir").flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let rel = if rel.is_empty() {
            name
        } else {
            format!("{rel}/{name}")
        };
        let path = e.path();
        if path.is_dir() {
            walk(&path, &rel, out);
        } else {
            out.push((rel, path));
        }
    }
}

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

/// Civil date (UTC) from Unix seconds (Howard Hinnant's algorithm).
fn iso_date(secs: i64) -> String {
    let z = secs.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Build identity: CI passes `FX_GIT_HASH` / `FX_BUILD_DATE`; local builds fall back to git and the clock.
fn build_info() {
    for var in ["FX_GIT_HASH", "FX_BUILD_DATE", "SOURCE_DATE_EPOCH"] {
        println!("cargo:rerun-if-env-changed={var}");
    }
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    println!("cargo:rerun-if-changed=../../.git/index");
    let set = |k: &str| env::var(k).ok().filter(|v| !v.is_empty());

    let hash = set("FX_GIT_HASH").unwrap_or_else(|| {
        let Some(hash) = git(&["rev-parse", "--short=12", "HEAD"]) else {
            return "unknown".into();
        };
        let dirty = git(&["status", "--porcelain"]).is_some_and(|s| !s.is_empty());
        if dirty { format!("{hash}-dirty") } else { hash }
    });
    let date = set("FX_BUILD_DATE").unwrap_or_else(|| {
        let secs = set("SOURCE_DATE_EPOCH")
            .and_then(|s| s.parse().ok())
            .unwrap_or_else(|| {
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_or(0, |d| d.as_secs() as i64)
            });
        iso_date(secs)
    });
    println!("cargo:rustc-env=FX_GIT_HASH={hash}");
    println!("cargo:rustc-env=FX_BUILD_DATE={date}");
}

fn main() {
    build_info();
    let root = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap())
        .join("../../third_party/material-icon-theme")
        .canonicalize()
        .expect("third_party/material-icon-theme missing");
    println!("cargo:rerun-if-changed={}", root.display());
    let mut files = Vec::new();
    walk(&root, "", &mut files);
    files.sort();

    let mut src = format!(
        "pub const PACK_TOML: &str = include_str!({:?});\n\n",
        root.join("pack.toml").display().to_string()
    );
    src.push_str("pub fn asset(rel: &str) -> Option<&'static [u8]> {\n    Some(match rel {\n");
    for (rel, path) in files.iter().filter(|(r, _)| r.ends_with(".svg")) {
        writeln!(
            src,
            "        {rel:?} => include_bytes!({:?}),",
            path.display().to_string()
        )
        .unwrap();
    }
    src.push_str("        _ => return None,\n    })\n}\n");
    fs::write(
        PathBuf::from(env::var("OUT_DIR").unwrap()).join("icons_embedded.rs"),
        src,
    )
    .unwrap();
}
