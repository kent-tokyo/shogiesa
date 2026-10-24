use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// A same-directory temporary output that replaces its destination only after every byte has
/// been written, flushed, and synchronized successfully.
pub(crate) struct AtomicWriter {
    destination: PathBuf,
    temp: Option<tempfile::NamedTempFile>,
    writer: Option<BufWriter<File>>,
}

impl AtomicWriter {
    pub(crate) fn new(destination: &Path) -> Result<Self> {
        let destination = match fs::symlink_metadata(destination) {
            Ok(metadata) if metadata.file_type().is_symlink() => fs::canonicalize(destination)
                .with_context(|| format!("cannot resolve output symlink {destination:?}"))?,
            Ok(_) => destination.to_path_buf(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => destination.to_path_buf(),
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("cannot inspect output path {destination:?}"));
            }
        };
        let parent = destination
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let temp = tempfile::NamedTempFile::new_in(parent)
            .with_context(|| format!("cannot create temporary output in {parent:?}"))?;
        let file = temp
            .reopen()
            .with_context(|| format!("cannot reopen temporary output for {destination:?}"))?;
        Ok(Self {
            destination,
            temp: Some(temp),
            writer: Some(BufWriter::new(file)),
        })
    }

    pub(crate) fn commit(mut self) -> Result<()> {
        let mut writer = self
            .writer
            .take()
            .expect("atomic writer always owns its buffer before commit");
        writer
            .flush()
            .with_context(|| format!("cannot flush temporary output for {:?}", self.destination))?;
        writer
            .get_ref()
            .sync_all()
            .with_context(|| format!("cannot sync temporary output for {:?}", self.destination))?;
        drop(writer);

        let temp = self
            .temp
            .take()
            .expect("atomic writer always owns its temporary file before commit");
        temp.persist(&self.destination).map_err(|error| {
            anyhow::anyhow!(
                "cannot atomically replace {:?}: {}",
                self.destination,
                error.error
            )
        })?;
        Ok(())
    }
}

impl Write for AtomicWriter {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        self.writer
            .as_mut()
            .expect("atomic writer buffer is present until commit")
            .write(buffer)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.writer
            .as_mut()
            .expect("atomic writer buffer is present until commit")
            .flush()
    }
}

pub(crate) fn atomic_write_bytes(path: &Path, contents: &[u8]) -> Result<()> {
    let mut writer = AtomicWriter::new(path)?;
    writer.write_all(contents)?;
    writer.commit()
}
