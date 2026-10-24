use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;

use anyhow::{Context, Result};

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub(crate) fn hash_file_sha256(path: &Path) -> Result<String> {
    use sha2::{Digest, Sha256};
    let mut file = File::open(path).with_context(|| format!("cannot open {path:?}"))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let bytes_read = file
            .read(&mut buffer)
            .with_context(|| format!("cannot hash {path:?}"))?;
        if bytes_read == 0 {
            break;
        }
        hasher.update(&buffer[..bytes_read]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

pub(crate) struct Sha256Reader<R> {
    inner: R,
    hasher: sha2::Sha256,
    bytes_read: u64,
}

impl<R> Sha256Reader<R> {
    pub(crate) fn new(inner: R) -> Self {
        use sha2::Digest;
        Self {
            inner,
            hasher: sha2::Sha256::new(),
            bytes_read: 0,
        }
    }

    pub(crate) fn digest_hex(self) -> String {
        use sha2::Digest;
        self.hasher
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    pub(crate) fn identity(self) -> (u64, String) {
        let bytes_read = self.bytes_read;
        (bytes_read, self.digest_hex())
    }
}

impl<R: Read> Read for Sha256Reader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        use sha2::Digest;
        let read = self.inner.read(buffer)?;
        self.hasher.update(&buffer[..read]);
        self.bytes_read = self.bytes_read.saturating_add(read as u64);
        Ok(read)
    }
}

pub(crate) struct Sha256Writer<W> {
    inner: W,
    hasher: sha2::Sha256,
}

impl<W> Sha256Writer<W> {
    pub(crate) fn new(inner: W) -> Self {
        use sha2::Digest;
        Self {
            inner,
            hasher: sha2::Sha256::new(),
        }
    }

    pub(crate) fn digest_hex(&self) -> String {
        use sha2::Digest;
        self.hasher
            .clone()
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }
}

impl<W: Write> Write for Sha256Writer<W> {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        use sha2::Digest;
        let written = self.inner.write(buffer)?;
        self.hasher.update(&buffer[..written]);
        Ok(written)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}
