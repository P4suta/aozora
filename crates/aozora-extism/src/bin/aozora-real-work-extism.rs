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

fn call(plugin: &mut Plugin, name: &str, source: &str) -> anyhow::Result<String> {
    let output: &str = plugin
        .call(name, source)
        .with_context(|| format!("call {name}"))?;
    Ok(output.to_owned())
}

fn projection(plugin: &mut Plugin, name: &str, source: &str) -> anyhow::Result<Value> {
    let envelope: Value = serde_json::from_str(&call(plugin, name, source)?)?;
    envelope
        .get("data")
        .cloned()
        .context("projection envelope lacks data")
}

fn render(plugin: &mut Plugin, input: &Request) -> anyhow::Result<Value> {
    if input.protocol_version != 1 || input.operation != "render" || input.id.is_empty() {
        anyhow::bail!("invalid request fields");
    }
    Ok(json!({
        "version": call(plugin, "version", "")?,
        "schemaVersion": call(plugin, "schema_version", "")?.parse::<u32>()?,
        "html": call(plugin, "to_html", &input.source)?,
        "diagnostics": projection(plugin, "diagnostics_json", &input.source)?,
        "gaiji": projection(plugin, "gaiji_json", &input.source)?,
        "nodes": projection(plugin, "nodes_json", &input.source)?,
        "pairs": projection(plugin, "pairs_json", &input.source)?,
        "containerPairs": projection(plugin, "container_pairs_json", &input.source)?,
        "source": call(plugin, "to_source", &input.source)?,
    }))
}

fn main() -> anyhow::Result<()> {
    let artifact = env::args_os()
        .nth(1)
        .context("usage: aozora-real-work-extism <aozora.wasm>")?;
    let manifest = Manifest::new([Wasm::data(fs::read(&artifact).context("read plugin")?)]);
    let mut plugin = Plugin::new(&manifest, [], false).context("instantiate plugin")?;
    let stdin = io::stdin();
    let mut stdout = io::BufWriter::new(io::stdout().lock());
    for line in stdin.lock().lines() {
        let line = line.context("read JSONL request")?;
        let response = match serde_json::from_str::<Request>(&line) {
            Ok(input) => match render(&mut plugin, &input) {
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
        serde_json::to_writer(&mut stdout, &response).context("write JSONL response")?;
        stdout.write_all(b"\n")?;
        stdout.flush()?;
    }
    Ok(())
}
