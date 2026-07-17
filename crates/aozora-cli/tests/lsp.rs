//! End-to-end test for `aozora lsp` — the **in-process** language server.
//!
//! Spawns `aozora lsp --stdio` and drives a real LSP session over its stdio:
//! `initialize` → `initialized` → `didOpen` (of a document with an unclosed
//! bracket), then reads until the server pushes `textDocument/publishDiagnostics`
//! for that document. This proves the whole in-process pipeline — argv handling,
//! the tower-lsp runloop over stdio, parse, and diagnostic projection — without
//! the retired exec-delegate to a separate `aozora-lsp` binary.

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdout, Command, Stdio, abort};
use std::thread;
use std::time::Duration;

use serde_json::{Value, json};

/// The `aozora` binary under test.
const BIN: &str = env!("CARGO_BIN_EXE_aozora");

/// The document handle used for every handshake here — an opaque identifier the
/// server only echoes back on `publishDiagnostics`, so one value serves all
/// tests (each drives its own single-document server).
const DOC_URI: &str = "file:///doc.aozora";

/// Arm a watchdog that force-kills the whole test process if the server hangs,
/// converting a deadlock into a visible failure rather than a wedged run.
fn arm_watchdog() {
    thread::spawn(|| {
        thread::sleep(Duration::from_secs(30));
        eprintln!("lsp handshake watchdog fired: the server did not respond in time");
        abort();
    });
}

/// Drive `initialize` (handshaking as `locale`) → `initialized` → `didOpen` (of
/// `text`) and return the `diagnostics` array the server pushes for the opened
/// document.
fn open_and_collect_diagnostics(
    stdin: &mut impl Write,
    reader: &mut BufReader<ChildStdout>,
    text: &str,
    locale: &str,
) -> Value {
    send(
        stdin,
        &json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": { "capabilities": {}, "locale": locale }
        }),
    );
    let init = recv(reader);
    assert_eq!(
        init["id"],
        json!(1),
        "initialize response carries id 1: {init}"
    );
    assert!(
        init["result"]["capabilities"].is_object(),
        "initialize result advertises capabilities: {init}"
    );

    send(
        stdin,
        &json!({ "jsonrpc": "2.0", "method": "initialized", "params": {} }),
    );
    send(
        stdin,
        &json!({
            "jsonrpc": "2.0", "method": "textDocument/didOpen",
            "params": { "textDocument": {
                "uri": DOC_URI, "languageId": "aozora", "version": 1, "text": text
            } }
        }),
    );

    // The server may interleave `window/logMessage` or other traffic first.
    loop {
        let msg = recv(reader);
        if msg["method"] == json!("textDocument/publishDiagnostics")
            && msg["params"]["uri"] == json!(DOC_URI)
        {
            break msg["params"]["diagnostics"].clone();
        }
    }
}

/// Frame a JSON-RPC message with its `Content-Length` header and write it to
/// the server's stdin.
fn send(stdin: &mut impl Write, msg: &Value) {
    let body = serde_json::to_vec(msg).expect("serialize JSON-RPC message");
    write!(stdin, "Content-Length: {}\r\n\r\n", body.len()).expect("write header");
    stdin.write_all(&body).expect("write body");
    stdin.flush().expect("flush stdin");
}

/// Read one `Content-Length`-framed JSON-RPC message off the server's stdout.
fn recv(reader: &mut BufReader<ChildStdout>) -> Value {
    let mut content_length: Option<usize> = None;
    loop {
        let mut line = String::new();
        let n = reader.read_line(&mut line).expect("read header line");
        assert!(n != 0, "server closed stdout before a full message arrived");
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break; // blank line terminates the header block
        }
        if let Some(rest) = trimmed.strip_prefix("Content-Length:") {
            content_length = Some(rest.trim().parse().expect("Content-Length is a number"));
        }
    }
    let len = content_length.expect("a Content-Length header preceded the body");
    let mut body = vec![0u8; len];
    reader.read_exact(&mut body).expect("read message body");
    serde_json::from_slice(&body).expect("body is valid JSON")
}

/// A killed-on-drop child, so a hung server never wedges the test run.
struct Daemon(Child);
impl Drop for Daemon {
    fn drop(&mut self) {
        let _drop = self.0.kill();
        let _drop = self.0.wait();
    }
}

#[test]
fn in_process_lsp_completes_the_initialize_didopen_diagnostics_handshake() {
    arm_watchdog();

    let mut child = Command::new(BIN)
        .args(["lsp", "--stdio"])
        .env("AOZORA_LANG", "en")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn `aozora lsp`");
    let mut stdin = child.stdin.take().expect("child stdin");
    let mut reader = BufReader::new(child.stdout.take().expect("child stdout"));
    let mut daemon = Daemon(child);

    // Handshake + didOpen of a document with an unclosed bracket — the server
    // must push at least one diagnostic back for it.
    let diagnostics =
        open_and_collect_diagnostics(&mut stdin, &mut reader, "本文［＃改ページ", "en");
    let diags = diagnostics.as_array().expect("diagnostics is an array");
    assert!(
        !diags.is_empty(),
        "the unclosed bracket produces at least one diagnostic: {diagnostics}"
    );
    // The code is the dotted `aozora::lex::*` contract the CLI shares.
    let codes: Vec<&str> = diags.iter().filter_map(|d| d["code"].as_str()).collect();
    assert!(
        codes.iter().any(|c| c.starts_with("aozora::lex::")),
        "diagnostic codes are the dotted aozora::lex::* contract: {codes:?}"
    );

    // Orderly shutdown, then confirm a clean exit with no server panic.
    send(
        &mut stdin,
        &json!({ "jsonrpc": "2.0", "id": 2, "method": "shutdown", "params": null }),
    );
    let _shutdown = recv(&mut reader);
    send(
        &mut stdin,
        &json!({ "jsonrpc": "2.0", "method": "exit", "params": null }),
    );
    drop(stdin);

    let status = daemon.0.wait().expect("await server exit");
    assert!(
        status.success(),
        "the server exits cleanly after shutdown/exit: {status:?}"
    );

    let mut stderr = String::new();
    if let Some(mut err) = daemon.0.stderr.take() {
        let _drop = err.read_to_string(&mut stderr);
    }
    assert!(
        !stderr.to_lowercase().contains("panic"),
        "the server logged no panic: {stderr}"
    );
}

/// Spawn a fresh server that handshakes as `locale`, open a document that always
/// yields a diagnostic, and return that diagnostic's localized `message`.
/// `AOZORA_LANG` is cleared so the client's handshake locale — not the daemon's
/// environment — is what resolves the UI language (it sits above the OS `LANG`
/// in the precedence chain, so it decides regardless of the test host's `LANG`).
fn first_diagnostic_message(locale: &str) -> String {
    let mut child = Command::new(BIN)
        .args(["lsp", "--stdio"])
        .env_remove("AOZORA_LANG")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn `aozora lsp`");
    let mut stdin = child.stdin.take().expect("child stdin");
    let mut reader = BufReader::new(child.stdout.take().expect("child stdout"));
    let mut daemon = Daemon(child);

    let diagnostics =
        open_and_collect_diagnostics(&mut stdin, &mut reader, "本文［＃改ページ", locale);
    let message = diagnostics
        .as_array()
        .and_then(|diags| diags.first())
        .and_then(|diag| diag["message"].as_str())
        .expect("the server reports a diagnostic with a message")
        .to_owned();

    // Closing stdin is EOF to the stdio transport, so the server leaves its read
    // loop and exits; the `Daemon` drop still guards against a hang.
    drop(stdin);
    let _exit = daemon.0.wait();
    message
}

/// The client's `initialize` locale — not the daemon's own environment — decides
/// the server's UI language. Two fresh servers open the same document; the one
/// that handshakes as Japanese must answer in a different language than the one
/// that handshakes as English. Each server is its own process, so the resolved
/// language — a process-global fixed by the first handshake — stays
/// deterministic; a single shared process would let test order decide it.
#[test]
fn initialize_locale_decides_the_ui_language() {
    arm_watchdog();

    let ja = first_diagnostic_message("ja");
    let en = first_diagnostic_message("en");

    assert!(
        !ja.is_empty() && !en.is_empty(),
        "both servers report a non-empty diagnostic message (ja: {ja:?}, en: {en:?})",
    );
    assert_ne!(
        ja, en,
        "the client's locale must decide the server's UI language",
    );
}
