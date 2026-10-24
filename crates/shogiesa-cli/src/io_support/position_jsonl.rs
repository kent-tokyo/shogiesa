use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;

use anyhow::{Context, Result};
use shogiesa_core::schema::{
    PositionRecordJsonlReader, PositionRecordLine, PositionRecordLineError,
};

use super::hashing::Sha256Reader;

struct ManifestJsonlBufReader<R> {
    inner: R,
    normalized_hasher: blake3::Hasher,
}

impl<R> ManifestJsonlBufReader<R> {
    fn new(inner: R) -> Self {
        Self {
            inner,
            normalized_hasher: blake3::Hasher::new(),
        }
    }

    fn identity(self) -> (R, String) {
        (
            self.inner,
            self.normalized_hasher.finalize().to_hex().to_string(),
        )
    }
}

impl<R: BufRead> Read for ManifestJsonlBufReader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        self.inner.read(buffer)
    }
}

impl<R: BufRead> BufRead for ManifestJsonlBufReader<R> {
    fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
        self.inner.fill_buf()
    }

    fn consume(&mut self, amount: usize) {
        self.inner.consume(amount);
    }

    fn read_line(&mut self, buffer: &mut String) -> std::io::Result<usize> {
        let start = buffer.len();
        let read = self.inner.read_line(buffer)?;
        if read > 0 {
            let line = &buffer[start..];
            let normalized = line.strip_suffix('\n').unwrap_or(line);
            let normalized = normalized.strip_suffix('\r').unwrap_or(normalized);
            self.normalized_hasher.update(normalized.as_bytes());
            self.normalized_hasher.update(b"\n");
        }
        Ok(read)
    }
}

type Inner = PositionRecordJsonlReader<ManifestJsonlBufReader<BufReader<Sha256Reader<File>>>>;

pub(crate) struct HashedPositionJsonlReader {
    inner: Inner,
}

impl Iterator for HashedPositionJsonlReader {
    type Item = std::result::Result<PositionRecordLine, PositionRecordLineError>;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next()
    }
}

pub(crate) fn open_hashed_position_jsonl(path: &Path) -> Result<HashedPositionJsonlReader> {
    let file = File::open(path).with_context(|| format!("cannot open {path:?}"))?;
    Ok(HashedPositionJsonlReader {
        inner: PositionRecordJsonlReader::new(ManifestJsonlBufReader::new(BufReader::new(
            Sha256Reader::new(file),
        ))),
    })
}

pub(crate) fn finish_hashed_position_jsonl(reader: HashedPositionJsonlReader) -> (String, String) {
    let (reader, normalized_blake3) = reader.inner.into_inner().identity();
    let exact_sha256 = reader.into_inner().digest_hex();
    (normalized_blake3, exact_sha256)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    use crate::io_support::hashing::sha256_hex;

    #[test]
    fn normalized_and_exact_identities_are_distinct() {
        let input = b"first\r\n\nlast-without-newline";
        let inner = BufReader::new(Sha256Reader::new(Cursor::new(input)));
        let mut reader = ManifestJsonlBufReader::new(inner);
        let mut line = String::new();
        while reader.read_line(&mut line).unwrap() != 0 {
            line.clear();
        }

        let (reader, normalized) = reader.identity();
        assert_eq!(
            normalized,
            blake3::hash(b"first\n\nlast-without-newline\n")
                .to_hex()
                .to_string()
        );
        assert_eq!(reader.into_inner().digest_hex(), sha256_hex(input));
    }
}
