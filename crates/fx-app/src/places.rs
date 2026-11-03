//! Sidebar locations and breadcrumb segments. Pure path logic plus filesystem existence checks.

use std::env;
#[cfg(target_os = "linux")]
use std::fs;
use std::path::{Component, Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Place {
    pub label: String,
    pub path: PathBuf,
    pub group: PlaceGroup,
}

/// Sidebar section a place belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaceGroup {
    /// Home and its standard folders.
    Places,
    /// Filesystem roots and mounts.
    Drives,
}

impl PlaceGroup {
    pub fn title(self) -> &'static str {
        match self {
            Self::Places => "Places",
            Self::Drives => "Drives",
        }
    }
}

/// Home-relative folders shown when they exist.
const USER_DIRS: [&str; 6] = [
    "Desktop",
    "Documents",
    "Downloads",
    "Pictures",
    "Music",
    "Videos",
];

#[cfg(windows)]
const ROOT_LABEL: &str = "\\";
#[cfg(not(windows))]
const ROOT_LABEL: &str = "/";

/// Home and its existing standard folders, then the filesystem roots.
pub fn places() -> Vec<Place> {
    let mut out = Vec::new();
    if let Some(home) = home_dir() {
        out.extend(existing(
            std::iter::once(("Home".to_owned(), home.clone())).chain(
                USER_DIRS
                    .iter()
                    .map(|name| ((*name).to_owned(), home.join(name))),
            ),
        ));
    }
    out.extend(roots());
    out
}

/// Each segment of `path` with the path that ends at it. The root segment is labelled `/` on Unix.
pub fn breadcrumbs(path: &Path) -> Vec<(String, PathBuf)> {
    let mut acc = PathBuf::new();
    let mut out: Vec<(String, PathBuf)> = Vec::new();
    let mut after_prefix = false;
    for component in path.components() {
        acc.push(component);
        match component {
            // A Windows root directly after its drive prefix extends that segment, so `C:` becomes `C:\`.
            Component::RootDir if after_prefix => {
                if let Some(last) = out.last_mut() {
                    last.1 = acc.clone();
                }
            }
            Component::RootDir => out.push((ROOT_LABEL.to_owned(), acc.clone())),
            _ => out.push((
                component.as_os_str().to_string_lossy().into_owned(),
                acc.clone(),
            )),
        }
        after_prefix = matches!(component, Component::Prefix(_));
    }
    out
}

pub(crate) fn home_dir() -> Option<PathBuf> {
    env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

fn existing(candidates: impl Iterator<Item = (String, PathBuf)>) -> Vec<Place> {
    candidates
        .filter(|(_, path)| path.is_dir())
        .map(|(label, path)| Place {
            label,
            path,
            group: PlaceGroup::Places,
        })
        .collect()
}

#[cfg(windows)]
fn roots() -> Vec<Place> {
    (b'C'..=b'Z')
        .filter_map(|letter| {
            let label = format!("{}:", char::from(letter));
            let path = PathBuf::from(format!("{label}\\"));
            path.is_dir().then_some(Place {
                label,
                path,
                group: PlaceGroup::Drives,
            })
        })
        .collect()
}

#[cfg(all(unix, not(target_os = "linux")))]
fn roots() -> Vec<Place> {
    vec![root_place()]
}

#[cfg(target_os = "linux")]
fn roots() -> Vec<Place> {
    let mut out = vec![root_place()];
    out.extend(child_dirs(Path::new("/mnt")));
    if let Some(user) = env::var_os("USER") {
        out.extend(child_dirs(&Path::new("/media").join(user)));
    }
    out
}

#[cfg(unix)]
fn root_place() -> Place {
    Place {
        label: ROOT_LABEL.to_owned(),
        path: PathBuf::from(ROOT_LABEL),
        group: PlaceGroup::Drives,
    }
}

/// Directories directly inside `dir` (labelled by full path), sorted by label. A missing mount directory is normal, so it yields nothing.
#[cfg(target_os = "linux")]
fn child_dirs(dir: &Path) -> Vec<Place> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut places: Vec<Place> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .map(|path| {
            // Full path as label: bare mount names such as `c` are meaningless in a list.
            let label = path.to_string_lossy().into_owned();
            Place {
                label,
                path,
                group: PlaceGroup::Drives,
            }
        })
        .collect();
    places.sort_by(|a, b| a.label.cmp(&b.label));
    places
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[cfg(target_os = "linux")]
    #[test]
    fn mounts_are_drives_labelled_by_full_path() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("c")).unwrap();
        let found = child_dirs(dir.path());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].group, PlaceGroup::Drives);
        assert_eq!(found[0].label, dir.path().join("c").to_string_lossy());
    }

    #[cfg(unix)]
    #[test]
    fn breadcrumbs_of_usr_lib() {
        assert_eq!(
            breadcrumbs(Path::new("/usr/lib")),
            [
                ("/".to_owned(), PathBuf::from("/")),
                ("usr".to_owned(), PathBuf::from("/usr")),
                ("lib".to_owned(), PathBuf::from("/usr/lib")),
            ]
        );
    }
}
