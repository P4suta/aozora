#!/usr/bin/env python3

import errno
import os
import pty
import select
import signal
import sys
import tempfile
import time
from pathlib import Path


def exercise(binary, args, *, send=b"", expected=0, markers=()):
    pid, fd = pty.fork()
    if pid == 0:
        os.environ["AOZORA_LANG"] = "en"
        os.environ["TERM"] = "xterm-256color"
        os.execv(binary, [binary, *args])

    deadline = time.monotonic() + 10
    output = bytearray()
    status = None
    if send:
        time.sleep(0.4)
        os.write(fd, send)

    while time.monotonic() < deadline:
        finished, status = os.waitpid(pid, os.WNOHANG)
        if finished:
            break
        readable, _, _ = select.select([fd], [], [], 0.05)
        if readable:
            try:
                output.extend(os.read(fd, 65536))
            except OSError as error:
                if error.errno != errno.EIO:
                    raise
    else:
        os.kill(pid, signal.SIGKILL)
        os.waitpid(pid, 0)
        raise AssertionError(f"{' '.join(args)} timed out")

    while True:
        readable, _, _ = select.select([fd], [], [], 0)
        if not readable:
            break
        try:
            chunk = os.read(fd, 65536)
            if not chunk:
                break
            output.extend(chunk)
        except OSError as error:
            if error.errno == errno.EIO:
                break
            raise
    os.close(fd)

    code = os.waitstatus_to_exitcode(status)
    if code != expected:
        raise AssertionError(
            f"{' '.join(args)} exited {code}, expected {expected}: {output!r}"
        )
    for marker in markers:
        if marker not in output:
            raise AssertionError(f"{' '.join(args)} omitted {marker!r}: {output!r}")


def main():
    if len(sys.argv) != 2:
        raise SystemExit("usage: tty-smoke.py <aozora-binary>")
    binary = os.path.abspath(sys.argv[1])
    if not os.access(binary, os.X_OK):
        raise SystemExit(f"not executable: {binary}")

    exercise(
        binary,
        ["check"],
        expected=2,
        markers=(b"standard input is empty",),
    )
    exercise(
        binary,
        ["repl"],
        send=b":quit\r",
        markers=(b"aozora repl",),
    )
    exercise(
        binary,
        ["tui"],
        send=b"\x11",
        markers=(b"\x1b[?1049h", b"\x1b[?1049l"),
    )
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        (root / "first.txt").write_text("first", encoding="utf-8")
        (root / "second.txt").write_text("second", encoding="utf-8")
        exercise(
            binary,
            ["fmt", "--check", directory],
            markers=(b"unchanged",),
        )
    print("tty-smoke: all interactive paths passed")


if __name__ == "__main__":
    main()
