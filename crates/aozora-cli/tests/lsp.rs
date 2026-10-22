//! End-to-end tests for the language server over stdio.

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdout, Command, Stdio, abort};
use std::thread;
use std::time::Duration;

use serde_json::{Value, json};

const BIN: &str = env!("CARGO_BIN_EXE_aozora");

const DOC_URI: &str = "file:///doc.aozora";

fn arm_watchdog() {
    thread::spawn(|| {
        thread::sleep(Duration::from_secs(30));
        eprintln!("lsp handshake watchdog fired: the server did not respond in time");
        abort();
    });
}

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

    loop {
        let msg = recv(reader);
        if msg["method"] == json!("textDocument/publishDiagnostics")
            && msg["params"]["uri"] == json!(DOC_URI)
        {
            break msg["params"]["diagnostics"].clone();
        }
    }
}

fn send(stdin: &mut impl Write, msg: &Value) {
    let body = serde_json::to_vec(msg).expect("serialize JSON-RPC message");
    write!(stdin, "Content-Length: {}\r\n\r\n", body.len()).expect("write header");
    stdin.write_all(&body).expect("write body");
    stdin.flush().expect("flush stdin");
}

fn recv(reader: &mut BufReader<ChildStdout>) -> Value {
    let mut content_length: Option<usize> = None;
    loop {
        let mut line = String::new();
        let n = reader.read_line(&mut line).expect("read header line");
        assert!(n != 0, "server closed stdout before a full message arrived");
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break;
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

    let diagnostics =
        open_and_collect_diagnostics(&mut stdin, &mut reader, "本文［＃改ページ", "en");
    let diags = diagnostics.as_array().expect("diagnostics is an array");
    assert!(
        !diags.is_empty(),
        "the unclosed bracket produces at least one diagnostic: {diagnostics}"
    );
    let codes: Vec<&str> = diags.iter().filter_map(|d| d["code"].as_str()).collect();
    assert!(
        codes.iter().any(|c| c.starts_with("aozora::lex::")),
        "diagnostic codes are the dotted aozora::lex::* contract: {codes:?}"
    );

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

    drop(stdin);
    let _exit = daemon.0.wait();
    message
}

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
