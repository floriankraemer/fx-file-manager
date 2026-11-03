//! Embeds the vendored Material icon theme into the binary.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
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

fn main() {
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
