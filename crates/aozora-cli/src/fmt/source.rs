//! Guarded source-input reading, shared by the formatter and the rest of the
//! `aozora` CLI.
//!
//! The parser core addresses spans with `u32` byte offsets, so a source longer
//! than [`MAX_SOURCE_BYTES`] trips a lexer assert. Under `panic = "abort"` that
//! assert tears the whole process down (SIGABRT / exit 134) — and only after a
//! multi-gigabyte read has already been paid for. The `aozora-py` and
//! `aozora-wasm` bindings reject oversize input at their own frontends (their
//! `MAX_SOURCE_BYTES` / `source_len_within_span_limit` guards); this module is
//! the same guard for the native read paths.
//!
//! Rejection happens at two points so the giant read is never paid for:
//!
//! - **before the read** — a file's [`fs::metadata`] size, or a bounded stdin
//!   read that stops one byte past the limit, decides up front;
//! - **after decode** — Shift_JIS → UTF-8 can expand past the limit even when
//!   the raw bytes fit, so [`encoding::decode`](crate::fmt::decode) re-checks the
//!   decoded length.
//!
//! An oversize input is a usage error returned as [`OversizeInput`].

use std::error::Error;
use std::fmt;
use std::fs;
use std::io::{self, Read};
use std::path::Path;

use anyhow::{Context, Result};

/// Largest source the parser core accepts, in bytes.
///
/// Its span offsets are `u32`, so `u32::MAX` bytes is the inclusive upper
/// bound; anything larger trips the lexer's span-offset assert. Mirrors the
/// binding-side guards in `aozora-py` and `aozora-wasm`.
pub(crate) const MAX_SOURCE_BYTES: u64 = u32::MAX as u64;

/// A source whose byte length exceeds [`MAX_SOURCE_BYTES`].
///
/// Carried through `anyhow` so the frontend maps it to a usage exit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct OversizeInput {
    /// The offending length, in bytes.
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

/// Reject a source whose byte length exceeds the `u32` span limit.
///
/// `u32::MAX` bytes is accepted (the inclusive upper bound); one byte more is
/// rejected — matching the lexer assert and the binding guards.
///
/// # Errors
///
/// Returns [`OversizeInput`] when `byte_len` exceeds [`MAX_SOURCE_BYTES`].
pub(crate) fn ensure_within_span_limit(byte_len: u64) -> Result<(), OversizeInput> {
    if byte_len > MAX_SOURCE_BYTES {
        Err(OversizeInput { bytes: byte_len })
    } else {
        Ok(())
    }
}

/// Read `path`, rejecting it up front — via its metadata size — when it exceeds
/// [`MAX_SOURCE_BYTES`].
///
/// Checking the size before the read means an oversize file is refused
/// immediately rather than after buffering gigabytes the parser core would only
/// abort on.
///
/// # Errors
///
/// Returns [`OversizeInput`] for an oversize file, or the underlying I/O error
/// (with the path as context) if the metadata or the read fails.
pub(crate) fn read_file(path: &Path) -> Result<Vec<u8>> {
    let size = fs::metadata(path)
        .with_context(|| format!("reading {}", path.display()))?
        .len();
    ensure_within_span_limit(size)?;
    fs::read(path).with_context(|| format!("reading {}", path.display()))
}

/// Byte cap for the bounded stdin read: one byte past [`MAX_SOURCE_BYTES`].
///
/// Reading a single byte beyond the inclusive limit is enough to tell "too big"
/// from "exactly at the limit": input exactly at the limit is still read whole,
/// and one byte over is still seen (then rejected by the length check) — all
/// without materialising a multi-gigabyte buffer.
const fn stdin_read_cap() -> u64 {
    MAX_SOURCE_BYTES + 1
}

/// Read all of stdin, stopping once the input passes [`MAX_SOURCE_BYTES`]
/// rather than buffering an unbounded amount only to reject it.
///
/// The reader is capped at `stdin_read_cap` bytes: one byte past the limit is
/// enough to tell "too big" from "exactly at the limit" without materialising a
/// multi-gigabyte buffer.
///
/// # Errors
///
/// Returns [`OversizeInput`] when stdin exceeds the limit, or the underlying
/// I/O error if the read fails.
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
        // u32::MAX bytes is the inclusive upper bound — accepted.
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
        // The 4 GiB + 16 B file that aborted the CLI (76 s read → SIGABRT).
        let bytes = MAX_SOURCE_BYTES + 17; // 4294967295 + 17 = 4294967312
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
        // The stdin reader must stop one byte past the inclusive `u32::MAX`
        // bound: exactly-at-limit input is read whole, one byte over is still
        // seen (then rejected). Pinning the concrete cap distinguishes the
        // real `+ 1` from a `* 1` (== MAX) or `- 1` (== MAX - 1) miscap that
        // would truncate an at-limit or over-limit read.
        assert_eq!(stdin_read_cap(), 4_294_967_296);
        assert_eq!(stdin_read_cap(), MAX_SOURCE_BYTES + 1);
        // One strictly greater than the inclusive bound…
        assert!(stdin_read_cap() > MAX_SOURCE_BYTES);
        // …and exactly one, so an at-limit read is neither truncated nor
        // padded.
        assert_eq!(stdin_read_cap() - MAX_SOURCE_BYTES, 1);
    }
}
