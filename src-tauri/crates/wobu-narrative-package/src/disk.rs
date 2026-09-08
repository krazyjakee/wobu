use crate::{
    Error, INCOMPLETE, MAX_MANIFEST_BYTES, Manifest, Package, Result, invalid, parse, validate,
};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path};

fn io(path: &Path, source: std::io::Error) -> Error {
    Error::Io { path: path.display().to_string(), source }
}
/// Check every existing path component, including the package root's ancestors.
fn no_symlinks(path: &Path) -> Result<()> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().map_err(|e| io(path, e))?.join(path)
    };
    let mut current = std::path::PathBuf::new();
    for part in absolute.components() {
        if matches!(part, Component::ParentDir) {
            return Err(invalid("parent path components are not allowed"));
        }
        current.push(part);
        let metadata = fs::symlink_metadata(&current).map_err(|e| io(&current, e))?;
        if metadata.file_type().is_symlink() {
            return Err(invalid(format!("symlink path is not allowed: {}", current.display())));
        }
    }
    Ok(())
}
fn bounded_read(path: &Path, limit: u64) -> Result<Vec<u8>> {
    no_symlinks(path)?;
    let mut file = File::open(path).map_err(|e| io(path, e))?;
    let metadata = file.metadata().map_err(|e| io(path, e))?;
    if !metadata.is_file() || metadata.len() > limit {
        return Err(invalid("non-file or oversized payload"));
    }
    let mut bytes = Vec::new();
    Read::by_ref(&mut file).take(limit + 1).read_to_end(&mut bytes).map_err(|e| io(path, e))?;
    if bytes.len() as u64 > limit {
        return Err(invalid("payload grew past size limit"));
    }
    Ok(bytes)
}

pub fn read(root: &Path) -> Result<Package> {
    no_symlinks(root)?;
    if root.join(INCOMPLETE).symlink_metadata().is_ok() {
        return Err(invalid("export is incomplete"));
    }
    let manifest: Manifest =
        parse(&bounded_read(&root.join("manifest.json"), MAX_MANIFEST_BYTES)?)?;
    validate::manifest(&manifest)?;
    let mut files = std::collections::BTreeMap::new();
    for (name, record) in &manifest.files {
        files.insert(name.clone(), bounded_read(&root.join(name), record.bytes)?);
    }
    let package = Package { manifest, files };
    package.graph()?;
    Ok(package)
}

/// Exclusively reserve a new directory. Any interruption leaves `.incomplete` in place;
/// consumers must use `read`, which refuses it. No destination is replaced or merged.
pub fn publish(package: &Package, destination: &Path) -> Result<()> {
    publish_checked(package, destination, || Ok(()))
}
fn publish_checked(
    package: &Package,
    destination: &Path,
    before_manifest: impl FnOnce() -> Result<()>,
) -> Result<()> {
    package.graph()?;
    let parent = destination.parent().ok_or_else(|| invalid("destination has no parent"))?;
    no_symlinks(parent)?;
    fs::create_dir(destination).map_err(|e| io(destination, e))?;
    write_new(&destination.join(INCOMPLETE), b"Incomplete Wobu narrative export. Do not load.\n")?;
    for (name, bytes) in &package.files {
        let path = destination.join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| io(parent, e))?;
            no_symlinks(parent)?;
        }
        write_new(&path, bytes)?;
        if let Some(parent) = path.parent() {
            sync_dir(parent)?;
        }
    }
    before_manifest()?;
    write_new(&destination.join("manifest.json"), &package.manifest_bytes()?)?;
    sync_dir(destination)?;
    sync_dir(parent)?;
    fs::remove_file(destination.join(INCOMPLETE)).map_err(|e| io(destination, e))?;
    if let Err(error) = sync_dir(destination) {
        let _ = write_new(
            &destination.join(INCOMPLETE),
            b"Publication flush failed. Incomplete export.\n",
        );
        return Err(error);
    }
    Ok(())
}
fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file =
        OpenOptions::new().write(true).create_new(true).open(path).map_err(|e| io(path, e))?;
    file.write_all(bytes).and_then(|_| file.sync_all()).map_err(|e| io(path, e))
}
fn sync_dir(path: &Path) -> Result<()> {
    // Windows does not expose opening a directory as a normal File; file payloads
    // are flushed there, while Unix additionally flushes directory entries.
    #[cfg(unix)]
    File::open(path).and_then(|f| f.sync_all()).map_err(|e| io(path, e))?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use wobu_narrative::{Beat, Destination, Outcome, Scene, StateSchema};
    use wobu_narrative_compiler::{CompileOptions, compile};
    #[test]
    fn interrupted_publication_keeps_marker_and_existing_destination() {
        let mut scene = Scene::new("Interruption");
        let mut beat = Beat::new("End");
        beat.outcomes.push(Outcome::new(Destination::End { label: "done".into() }));
        scene.beats.push(beat);
        let package = Package::build(
            compile(&[scene], &StateSchema::default(), &CompileOptions::default()).graph.unwrap(),
            false,
        )
        .unwrap();
        let destination =
            std::env::temp_dir().join(format!("wobu-export-interruption-{}", std::process::id()));
        let result = publish_checked(&package, &destination, || {
            Err(invalid("simulated interruption before manifest"))
        });
        assert!(result.is_err());
        assert!(destination.join(INCOMPLETE).is_file());
        assert!(!destination.join("manifest.json").exists());
        assert!(read(&destination).is_err());
        assert!(publish(&package, &destination).is_err());
        std::fs::remove_dir_all(destination).unwrap();
    }
}
