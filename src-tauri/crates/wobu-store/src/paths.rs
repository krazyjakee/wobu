//! Where things live, and what kind of filesystem they live on.

use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// Local application data — the index, the recent-projects list. Never inside a
/// project folder.
pub fn app_data_dir() -> PathBuf {
    directories::ProjectDirs::from("", "", "wobu")
        .map(|d| d.data_dir().to_path_buf())
        // A machine with no resolvable home is exotic enough that a visible
        // relative directory beats failing to start.
        .unwrap_or_else(|| PathBuf::from(".wobu-data"))
}

/// The SQLite index for a project, keyed by the project's ULID rather than its
/// path — so the index survives the share being remounted somewhere else.
///
/// This holds no canonical data; deleting it is always safe.
pub fn index_path(project_id: &wobu_core::Id) -> PathBuf {
    app_data_dir().join("index").join(format!("{project_id}.sqlite"))
}

pub fn recents_path() -> PathBuf {
    app_data_dir().join("recent.json")
}

/// Project-relative paths are stored with `/` separators and converted on read,
/// because the same share is `/Volumes/art/…` on one machine and `Z:\art\…` on
/// another.
pub fn to_rel_string(rel: &Path) -> String {
    rel.components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

pub fn from_rel_string(root: &Path, rel: &str) -> PathBuf {
    let mut p = root.to_path_buf();
    for segment in rel.split('/').filter(|s| !s.is_empty() && *s != ".") {
        p.push(segment);
    }
    p
}

/// Filesystem types where `inotify`/`FSEvents` do not observe writes made by
/// other hosts, and where SQLite's locking is unreliable.
#[cfg(target_os = "linux")]
const NETWORK_FS: &[&str] = &[
    "nfs",
    "nfs4",
    "cifs",
    "smb",
    "smbfs",
    "smb2",
    "afpfs",
    "ncpfs",
    "9p",
    "fuse.sshfs",
    "fuse.davfs",
    "davfs",
    "fuse.rclone",
    "fuse.gvfsd-fuse",
    "glusterfs",
    "ceph",
    "lustre",
];

/// Whether a path sits on a network mount.
///
/// This picks the change-detection strategy (watcher vs. polling), so a false
/// negative means a collaborator's edits stay invisible until restart. When we
/// cannot tell, we say `false` and the watcher falls back on its own timer —
/// see [`crate::watcher`].
pub fn is_network_path(path: &Path) -> bool {
    #[cfg(target_os = "linux")]
    {
        let Ok(mounts) = std::fs::read_to_string("/proc/self/mounts") else {
            return false;
        };
        let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());

        // Longest matching mount point wins: /mnt and /mnt/art can both match,
        // and only the deeper one describes the filesystem we are actually on.
        let mut best: Option<(usize, &str)> = None;
        for line in mounts.lines() {
            let mut fields = line.split_whitespace();
            let (Some(_dev), Some(mount_point), Some(fstype)) =
                (fields.next(), fields.next(), fields.next())
            else {
                continue;
            };
            // /proc/self/mounts octal-escapes spaces and similar.
            let mount_point = mount_point.replace("\\040", " ");
            if canonical.starts_with(&mount_point)
                && best.is_none_or(|(len, _)| mount_point.len() > len)
            {
                best = Some((mount_point.len(), fstype));
            }
        }
        best.is_some_and(|(_, fstype)| {
            NETWORK_FS.iter().any(|n| fstype == *n || fstype.starts_with(&format!("{n}.")))
        })
    }
    #[cfg(windows)]
    {
        // A UNC path is unambiguously remote. Mapped drive letters are not
        // detected yet, so they fall through to the polling default.
        let s = path.to_string_lossy();
        s.starts_with("\\\\") || s.starts_with("//")
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    {
        let _ = path;
        false
    }
}

/// Whether we can actually write into this project folder. Detected on open so
/// the UI can say so plainly rather than failing on the first save.
pub fn is_writable(root: &Path) -> bool {
    let probe_dir = root.join(".wobu").join("tmp");
    if std::fs::create_dir_all(&probe_dir).is_err() {
        return false;
    }
    let probe = probe_dir.join(format!("{}.probe", wobu_core::new_id()));
    match std::fs::write(&probe, b"") {
        Ok(()) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

pub fn ensure_dir(path: &Path) -> Result<()> {
    std::fs::create_dir_all(path).map_err(|e| Error::io(path, e))
}

/// Whether an open project's folder is still reachable.
///
/// Probes `project.json` rather than the directory, and that is the whole
/// point: unmounting a share usually leaves the mountpoint behind as an empty
/// directory, so `root.is_dir()` cheerfully returns `true` for a world that is
/// no longer there. Every caller of this is deciding whether to believe an
/// empty directory listing — which is precisely the case `is_dir` gets wrong.
pub fn project_is_present(root: &Path) -> bool {
    root.join("project.json").is_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_paths_use_forward_slashes_everywhere() {
        let rel = Path::new("nodes").join("character").join("kael-vantris.md");
        assert_eq!(to_rel_string(&rel), "nodes/character/kael-vantris.md");
    }

    #[test]
    fn relative_paths_round_trip_against_a_root() {
        let root = Path::new("/Volumes/art/Ashfall.wobu");
        let joined = from_rel_string(root, "nodes/character/kael-vantris.md");
        assert_eq!(joined, root.join("nodes/character/kael-vantris.md"));
    }

    #[test]
    fn a_local_temp_dir_is_not_a_network_path() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!is_network_path(dir.path()));
    }

    #[test]
    fn a_fresh_temp_dir_is_writable() {
        let dir = tempfile::tempdir().unwrap();
        assert!(is_writable(dir.path()));
    }

    #[test]
    fn the_index_lives_outside_any_project_folder() {
        let id = wobu_core::new_id();
        let path = index_path(&id);
        assert!(path.to_string_lossy().contains("index"));
        assert!(path.ends_with(format!("{id}.sqlite")));
    }
}
