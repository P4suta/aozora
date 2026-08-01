use std::error::Error;
use std::fmt;
use std::io::{self, Write};

#[derive(Debug)]
struct StdoutBrokenPipe;

impl fmt::Display for StdoutBrokenPipe {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("stdout was closed")
    }
}

impl Error for StdoutBrokenPipe {}

pub(crate) struct StdoutWriter<W> {
    inner: W,
}

pub(crate) fn guard<W: Write>(inner: W) -> StdoutWriter<W> {
    StdoutWriter { inner }
}

pub(crate) fn stdout() -> StdoutWriter<io::Stdout> {
    guard(io::stdout())
}

#[must_use]
pub(crate) fn is_broken_pipe(err: &anyhow::Error) -> bool {
    err.chain().any(|cause| {
        cause.downcast_ref::<StdoutBrokenPipe>().is_some()
            || cause
                .downcast_ref::<io::Error>()
                .and_then(io::Error::get_ref)
                .and_then(|inner| inner.downcast_ref::<StdoutBrokenPipe>())
                .is_some()
    })
}

impl<W: Write> Write for StdoutWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.inner.write(buf).map_err(mark_broken_pipe)
    }

    fn write_vectored(&mut self, bufs: &[io::IoSlice<'_>]) -> io::Result<usize> {
        self.inner.write_vectored(bufs).map_err(mark_broken_pipe)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush().map_err(mark_broken_pipe)
    }
}

fn mark_broken_pipe(err: io::Error) -> io::Error {
    if err.kind() == io::ErrorKind::BrokenPipe {
        io::Error::new(io::ErrorKind::BrokenPipe, StdoutBrokenPipe)
    } else {
        err
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Context;

    use super::*;

    struct FailingWriter(io::ErrorKind);

    impl Write for FailingWriter {
        fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
            Err(self.0.into())
        }

        fn flush(&mut self) -> io::Result<()> {
            Err(self.0.into())
        }
    }

    #[test]
    fn stdout_broken_pipe_survives_anyhow_context() {
        let mut out = guard(FailingWriter(io::ErrorKind::BrokenPipe));
        let err = out
            .write_all(b"x")
            .map_err(anyhow::Error::new)
            .context("write output")
            .unwrap_err();
        assert!(is_broken_pipe(&err));
    }

    #[test]
    fn raw_broken_pipe_is_not_stdout_broken_pipe() {
        let err = anyhow::Error::new(io::Error::from(io::ErrorKind::BrokenPipe));
        assert!(!is_broken_pipe(&err));
    }

    #[test]
    fn other_stdout_errors_are_not_broken_pipe() {
        let mut out = guard(FailingWriter(io::ErrorKind::PermissionDenied));
        let err = anyhow::Error::new(out.write_all(b"x").unwrap_err());
        assert!(!is_broken_pipe(&err));
    }
}
