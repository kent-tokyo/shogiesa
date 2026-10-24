use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// Compare path identity without relying on spelling alone. Existing paths use their OS file
/// identity; future paths use a canonical parent and normalized components.
pub(crate) fn paths_collide(left: &Path, right: &Path) -> Result<bool> {
    if left.exists() && right.exists() {
        return same_file::is_same_file(left, right)
            .with_context(|| format!("cannot compare path identity for {left:?} and {right:?}"));
    }

    fn future_path_key(path: &Path) -> Result<PathBuf> {
        if path.exists() {
            return fs::canonicalize(path)
                .with_context(|| format!("cannot canonicalize existing path {path:?}"));
        }
        let parent = path
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let file_name = path
            .file_name()
            .ok_or_else(|| anyhow::anyhow!("output path has no file name: {path:?}"))?;
        match fs::canonicalize(parent) {
            Ok(parent) => Ok(parent.join(file_name)),
            Err(_) => {
                let absolute = if path.is_absolute() {
                    path.to_path_buf()
                } else {
                    std::env::current_dir()?.join(path)
                };
                let mut normalized = PathBuf::new();
                for component in absolute.components() {
                    match component {
                        std::path::Component::CurDir => {}
                        std::path::Component::ParentDir => {
                            normalized.pop();
                        }
                        other => normalized.push(other.as_os_str()),
                    }
                }
                Ok(normalized)
            }
        }
    }

    Ok(future_path_key(left)? == future_path_key(right)?)
}

/// Reject aliases between read and write targets and between two write targets.
pub(crate) fn validate_path_roles(
    read_paths: &[(&str, &Path)],
    write_paths: &[(&str, &Path)],
) -> Result<()> {
    for &(read_name, read_path) in read_paths {
        for &(write_name, write_path) in write_paths {
            if paths_collide(read_path, write_path)? {
                anyhow::bail!("{read_name} must not be the same path as {write_name}");
            }
        }
    }
    for (index, &(left_name, left_path)) in write_paths.iter().enumerate() {
        for &(right_name, right_path) in &write_paths[index + 1..] {
            if paths_collide(left_path, right_path)? {
                anyhow::bail!("{left_name} must not be the same path as {right_name}");
            }
        }
    }
    Ok(())
}
