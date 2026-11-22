//! Sidebar locations and breadcrumb segments. Pure path logic plus filesystem existence checks.

use crate::place_group::PlaceGroup;
use crate::settings::home_dir;
use crate::state::Bookmark;
#[cfg(target_os = "linux")]
use std::fs;
use std::path::{Component, Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Place {
    pub label: String,
    pub path: PathBuf,
    pub group: PlaceGroup,
    /// UNC path behind a mapped network drive, e.g. `\\server\share`.
    pub share: Option<String>,
}

impl Place {
    /// Sidebar text: the label, followed by the network share when `show_share` is set and there is one.
    pub fn title(&self, show_share: bool) -> String {
        match &self.share {
            Some(share) if show_share => format!("{} ({share})", self.label),
            _ => self.label.clone(),
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
/// Probes the filesystem and can block on a dead mount, so call it off the UI thread.
pub(crate) fn places() -> Vec<Place> {
    let mut out = Vec::new();
    if let Some(home) = home_dir() {
        out.extend(existing(
            std::iter::once((crate::t("places.home").to_owned(), home.clone())).chain(
                USER_DIRS
                    .iter()
                    .map(|name| ((*name).to_owned(), home.join(name))),
            ),
        ));
    }
    out.extend(roots());
    #[cfg(windows)]
    out.extend(distro_places(crate::wsl::distro_names()));
    out
}

/// Retranslates the labels of `places` in place, without probing anything.
pub fn relabel_places(places: &mut [Place]) {
    let home = home_dir();
    for place in places
        .iter_mut()
        .filter(|p| p.group == PlaceGroup::Places && Some(&p.path) == home.as_ref())
    {
        place.label = crate::t("places.home").to_owned();
    }
}

/// One place per bookmark, in order. Touches no filesystem: a bookmark stays even when its folder is gone.
pub fn bookmark_places(bookmarks: &[Bookmark]) -> Vec<Place> {
    bookmarks
        .iter()
        .map(|b| Place {
            label: b.label.clone(),
            path: b.path.clone(),
            group: PlaceGroup::Bookmarks,
            share: None,
        })
        .collect()
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

fn existing(candidates: impl Iterator<Item = (String, PathBuf)>) -> Vec<Place> {
    candidates
        .filter(|(_, path)| path.is_dir())
        .map(|(label, path)| Place {
            label,
            path,
            group: PlaceGroup::Places,
            share: None,
        })
        .collect()
}

#[cfg(windows)]
fn roots() -> Vec<Place> {
    (b'A'..=b'Z')
        .filter_map(|letter| {
            let label = format!("{}:", char::from(letter));
            let path = PathBuf::from(format!("{label}\\"));
            path.is_dir().then(|| Place {
                share: fx_core::network_share(&label),
                label,
                path,
                group: PlaceGroup::Drives,
            })
        })
        .collect()
}

/// Names of Docker's internal helper distros, which are not meant to be browsed.
#[cfg(any(windows, test))]
const HIDDEN_DISTROS: [&str; 2] = ["docker-desktop", "docker-desktop-data"];

#[cfg(any(windows, test))]
fn distro_places(names: Vec<String>) -> Vec<Place> {
    let mut places: Vec<Place> = names
        .into_iter()
        .filter(|name| !HIDDEN_DISTROS.contains(&name.as_str()))
        .map(|name| Place {
            path: PathBuf::from(format!("\\\\wsl.localhost\\{name}")),
            label: name,
            group: PlaceGroup::Wsl,
            share: None,
        })
        .collect();
    places.sort_by(|a, b| a.label.cmp(&b.label));
    places
}

#[cfg(all(unix, not(target_os = "linux")))]
fn roots() -> Vec<Place> {
    vec![root_place()]
}

#[cfg(target_os = "linux")]
fn roots() -> Vec<Place> {
    let mut out = vec![root_place()];
    out.extend(child_dirs(Path::new("/mnt")));
    if let Some(user) = std::env::var_os("USER") {
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
        share: None,
    }
}

/// Directories directly inside `dir` (labelled by full path), sorted by label. A missing mount directory is normal, so it yields nothing.
#[cfg(target_os = "linux")]
fn child_dirs(dir: &Path) -> Vec<Place> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) => {
            if err.kind() != std::io::ErrorKind::NotFound {
                log::debug!("cannot read {}: {err}", dir.display());
            }
            return Vec::new();
        }
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
                share: None,
            }
        })
        .collect();
    places.sort_by(|a, b| a.label.cmp(&b.label));
    places
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_appends_share_only_when_enabled() {
        let mut place = Place {
            label: "A:".to_owned(),
            path: PathBuf::from("A:\\"),
            group: PlaceGroup::Drives,
            share: Some(r"\\midgard\data".to_owned()),
        };
        assert_eq!(place.title(true), r"A: (\\midgard\data)");
        assert_eq!(place.title(false), "A:");
        place.share = None;
        assert_eq!(place.title(true), "A:");
    }

    #[test]
    fn relabel_translates_only_the_home_place() {
        let home = home_dir().unwrap_or_else(|| PathBuf::from("/home"));
        let place = |label: &str, path: PathBuf| Place {
            label: label.to_owned(),
            path,
            group: PlaceGroup::Places,
            share: None,
        };
        let mut listed = vec![
            place("stale", home.clone()),
            place("Documents", home.join("Documents")),
        ];
        relabel_places(&mut listed);
        assert_eq!(listed[0].label, crate::t("places.home"));
        assert_eq!(listed[1].label, "Documents");
    }

    #[test]
    fn bookmarks_become_places_without_touching_the_filesystem() {
        let places = bookmark_places(&[Bookmark {
            label: "Gone".to_owned(),
            path: PathBuf::from("/definitely/not/here"),
        }]);
        assert_eq!(places.len(), 1);
        assert_eq!(places[0].group, PlaceGroup::Bookmarks);
        assert_eq!(places[0].label, "Gone");
        assert_eq!(places[0].share, None);
    }

    #[test]
    fn distros_are_sorted_wsl_places_without_docker_helpers() {
        let found = distro_places(vec![
            "Ubuntu".to_owned(),
            "docker-desktop".to_owned(),
            "Debian".to_owned(),
        ]);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].label, "Debian");
        assert_eq!(found[0].path, PathBuf::from(r"\\wsl.localhost\Debian"));
        assert_eq!(found[1].label, "Ubuntu");
        assert_eq!(found[1].group, PlaceGroup::Wsl);
    }

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
