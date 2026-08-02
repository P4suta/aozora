//! Long-lived JSONL adapter for the exact Extism plugin artifact.

#![allow(
    clippy::print_stdout,
    reason = "the worker owns the JSONL stdout transport"
)]

use std::env;
use std::fs;
use std::io::{self, BufRead, Write};

use anyhow::Context;
use extism::{Manifest, Plugin, Wasm};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Request {
    protocol_version: u32,
    #[serde(rename = "requestId")]
    id: String,
    operation: String,
    source: String,
}

trait Engine {
    fn invoke(&mut self, name: &str, source: &str) -> anyhow::Result<String>;
}

impl Engine for Plugin {
    // The exact plugin ABI is exercised by `tests/host_smoke.rs`; a unit test
    // cannot replace the external Extism runtime behind `Plugin::call`.
    #[cfg_attr(test, mutants::skip)]
    fn invoke(&mut self, name: &str, source: &str) -> anyhow::Result<String> {
        let output: &str = self
            .call(name, source)
            .with_context(|| format!("call {name}"))?;
        Ok(output.to_owned())
    }
}

fn projection<E: Engine>(engine: &mut E, name: &str, source: &str) -> anyhow::Result<Value> {
    let envelope: Value = serde_json::from_str(&engine.invoke(name, source)?)?;
    envelope
        .get("data")
        .cloned()
        .context("projection envelope lacks data")
}

fn render<E: Engine>(engine: &mut E, input: &Request) -> anyhow::Result<Value> {
    if input.protocol_version != 1 || input.operation != "render" || input.id.is_empty() {
        anyhow::bail!("invalid request fields");
    }
    Ok(json!({
        "version": engine.invoke("version", "")?,
        "schemaVersion": engine.invoke("schema_version", "")?.parse::<u32>()?,
        "html": engine.invoke("to_html", &input.source)?,
        "diagnostics": projection(engine, "diagnostics_json", &input.source)?,
        "gaiji": projection(engine, "gaiji_json", &input.source)?,
        "nodes": projection(engine, "nodes_json", &input.source)?,
        "pairs": projection(engine, "pairs_json", &input.source)?,
        "containerPairs": projection(engine, "container_pairs_json", &input.source)?,
        "source": engine.invoke("to_source", &input.source)?,
    }))
}

fn serve<E: Engine, R: BufRead, W: Write>(
    engine: &mut E,
    reader: R,
    writer: &mut W,
) -> anyhow::Result<()> {
    for line in reader.lines() {
        let line = line.context("read JSONL request")?;
        let response = match serde_json::from_str::<Request>(&line) {
            Ok(input) => match render(engine, &input) {
                Ok(result) => json!({
                    "protocolVersion": 1,
                    "requestId": input.id,
                    "ok": true,
                    "result": result,
                }),
                Err(error) => json!({
                    "protocolVersion": 1,
                    "requestId": input.id,
                    "ok": false,
                    "error": error.to_string(),
                }),
            },
            Err(error) => json!({
                "protocolVersion": 1,
                "requestId": "invalid",
                "ok": false,
                "error": error.to_string(),
            }),
        };
        serde_json::to_writer(&mut *writer, &response).context("write JSONL response")?;
        writer.write_all(b"\n")?;
        writer.flush()?;
    }
    Ok(())
}

// Argument and stdio ownership are covered by the release worker smoke; the
// request protocol itself is unit-tested through `serve` below.
#[cfg_attr(test, mutants::skip)]
fn main() -> anyhow::Result<()> {
    let artifact = env::args_os()
        .nth(1)
        .context("usage: aozora-real-work-extism <aozora.wasm>")?;
    let manifest = Manifest::new([Wasm::data(fs::read(&artifact).context("read plugin")?)]);
    let mut plugin = Plugin::new(&manifest, [], false).context("instantiate plugin")?;
    let stdin = io::stdin();
    let mut stdout = io::BufWriter::new(io::stdout().lock());
    serve(&mut plugin, stdin.lock(), &mut stdout)
}

#[cfg(test)]
mod tests {
    use super::{Engine, Request, render, serve};
    use serde_json::{Value, json};

    #[derive(Default)]
    struct FakeEngine {
        calls: Vec<(String, String)>,
    }

    impl Engine for FakeEngine {
        fn invoke(&mut self, name: &str, source: &str) -> anyhow::Result<String> {
            self.calls.push((name.to_owned(), source.to_owned()));
            let output = match name {
                "version" => "1.2.3".to_owned(),
                "schema_version" => "7".to_owned(),
                "to_html" => format!("<p>{source}</p>"),
                "diagnostics_json" => r#"{"data":["diagnostic"]}"#.to_owned(),
                "gaiji_json" => r#"{"data":["gaiji"]}"#.to_owned(),
                "nodes_json" => r#"{"data":["node"]}"#.to_owned(),
                "pairs_json" => r#"{"data":["pair"]}"#.to_owned(),
                "container_pairs_json" => r#"{"data":["container"]}"#.to_owned(),
                "to_source" => format!("canonical:{source}"),
                _ => anyhow::bail!("unexpected export {name}"),
            };
            Ok(output)
        }
    }

    fn request() -> Request {
        Request {
            protocol_version: 1,
            id: "extism-0".to_owned(),
            operation: "render".to_owned(),
            source: "青空".to_owned(),
        }
    }

    fn expected_result() -> Value {
        json!({
            "version": "1.2.3",
            "schemaVersion": 7,
            "html": "<p>青空</p>",
            "diagnostics": ["diagnostic"],
            "gaiji": ["gaiji"],
            "nodes": ["node"],
            "pairs": ["pair"],
            "containerPairs": ["container"],
            "source": "canonical:青空",
        })
    }

    #[test]
    fn renders_every_release_projection() {
        let mut engine = FakeEngine::default();
        assert_eq!(
            render(&mut engine, &request()).expect("valid request"),
            expected_result()
        );
        assert_eq!(
            engine.calls,
            [
                ("version", ""),
                ("schema_version", ""),
                ("to_html", "青空"),
                ("diagnostics_json", "青空"),
                ("gaiji_json", "青空"),
                ("nodes_json", "青空"),
                ("pairs_json", "青空"),
                ("container_pairs_json", "青空"),
                ("to_source", "青空"),
            ]
            .map(|(name, source)| (name.to_owned(), source.to_owned()))
        );
    }

    #[test]
    fn rejects_each_invalid_request_field() {
        let mut invalid_protocol = request();
        invalid_protocol.protocol_version = 2;
        let mut invalid_operation = request();
        invalid_operation.operation = "check".to_owned();
        let mut missing_id = request();
        missing_id.id.clear();

        for input in [invalid_protocol, invalid_operation, missing_id] {
            let mut engine = FakeEngine::default();
            render(&mut engine, &input).expect_err("invalid request must fail");
            assert!(engine.calls.is_empty());
        }
    }

    #[test]
    fn serves_success_and_malformed_json_as_framed_jsonl() {
        let input = concat!(
            r#"{"protocolVersion":1,"requestId":"extism-0","operation":"render","source":"青空"}"#,
            "\n{\n"
        );
        let mut output = Vec::new();
        serve(&mut FakeEngine::default(), input.as_bytes(), &mut output).expect("serve JSONL");
        let responses = String::from_utf8(output)
            .expect("UTF-8 output")
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).expect("response JSON"))
            .collect::<Vec<_>>();

        assert_eq!(responses.len(), 2);
        assert_eq!(
            responses[0],
            json!({
                "protocolVersion": 1,
                "requestId": "extism-0",
                "ok": true,
                "result": expected_result(),
            })
        );
        assert_eq!(responses[1]["protocolVersion"], 1);
        assert_eq!(responses[1]["requestId"], "invalid");
        assert_eq!(responses[1]["ok"], false);
        assert!(
            responses[1]["error"]
                .as_str()
                .is_some_and(|error| !error.is_empty())
        );
    }
}
