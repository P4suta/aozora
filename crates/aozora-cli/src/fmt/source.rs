use std::error::Error;
use std::fmt;
use std::fs;
use std::io::{self, Read};
use std::path::Path;

use anyhow::{Context, Result};

pub(crate) const MAX_SOURCE_BYTES: u64 = u32::MAX as u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct OversizeInput {
    pub bytes: u64,
}

impl fmt::Display for OversizeInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "ソースが 4 GiB (u32::MAX) のスパン上限を超えています: {} バイト",
            self.bytes
        )
    }
}

impl Error for OversizeInput {}

pub(crate) fn ensure_within_span_limit(byte_len: u64) -> Result<(), OversizeInput> {
    if byte_len > MAX_SOURCE_BYTES {
        Err(OversizeInput { bytes: byte_len })
    } else {
        Ok(())
    }
}

pub(crate) fn read_file(path: &Path) -> Result<Vec<u8>> {
    let size = fs::metadata(path)
        .with_context(|| format!("reading {}", path.display()))?
        .len();
    ensure_within_span_limit(size)?;
    fs::read(path).with_context(|| format!("reading {}", path.display()))
}

const fn stdin_read_cap() -> u64 {
    MAX_SOURCE_BYTES + 1
}

pub(crate) fn read_stdin() -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    io::stdin()
        .take(stdin_read_cap())
        .read_to_end(&mut buf)
        .context("reading stdin")?;
    ensure_within_span_limit(buf.len() as u64)?;
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn within_limit_is_accepted_up_to_the_inclusive_bound() {
        ensure_within_span_limit(0).expect("empty source is in range");
        ensure_within_span_limit(1024).expect("1 KiB source is in range");
        ensure_within_span_limit(MAX_SOURCE_BYTES).expect("u32::MAX bytes is accepted");
    }

    #[test]
    fn one_byte_past_the_limit_is_rejected() {
        let err = ensure_within_span_limit(MAX_SOURCE_BYTES + 1)
            .expect_err("u32::MAX + 1 bytes must be rejected");
        assert_eq!(
            err,
            OversizeInput {
                bytes: MAX_SOURCE_BYTES + 1
            }
        );
    }

    #[test]
    fn the_ledger_repro_size_is_rejected() {
        let bytes = MAX_SOURCE_BYTES + 17;
        assert_eq!(bytes, 4_294_967_312);
        let err = ensure_within_span_limit(bytes).expect_err("4 GiB + 16 B must be rejected");
        assert_eq!(err.bytes, bytes);
    }

    #[test]
    fn message_names_the_limit_and_the_offending_size() {
        let msg = OversizeInput {
            bytes: 4_294_967_312,
        }
        .to_string();
        assert!(msg.contains("4 GiB"), "message names the limit: {msg}");
        assert!(msg.contains("u32::MAX"), "message names u32::MAX: {msg}");
        assert!(
            msg.contains("4294967312"),
            "message names the offending size: {msg}"
        );
    }

    #[test]
    fn stdin_read_cap_is_exactly_one_byte_past_the_inclusive_limit() {
        assert_eq!(stdin_read_cap(), 4_294_967_296);
        assert_eq!(stdin_read_cap(), MAX_SOURCE_BYTES + 1);
        assert!(stdin_read_cap() > MAX_SOURCE_BYTES);
        assert_eq!(stdin_read_cap() - MAX_SOURCE_BYTES, 1);
    }
}
