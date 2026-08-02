use std::io::{self, BufRead, Write};
use std::process::ExitCode;

use anyhow::Context;
use aozora::json as wire;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::buildstamp::VERSION;

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Request {
    protocol_version: u32,
    #[serde(rename = "requestId")]
    id: String,
    operation: String,
    source: String,
}

fn projection(value: &str) -> anyhow::Result<Value> {
    let envelope: Value = serde_json::from_str(value).context("decode projection envelope")?;
    envelope
        .get("data")
        .cloned()
        .context("projection envelope lacks data")
}

fn render(input: &Request) -> anyhow::Result<Value> {
    if input.protocol_version != 1 || input.operation != "render" || input.id.is_empty() {
        anyhow::bail!("invalid request fields");
    }
    let document = aozora::parse(input.source.clone())?;
    let snapshot = document.snapshot();
    Ok(json!({
        "version": VERSION,
        "schemaVersion": wire::SCHEMA_VERSION,
        "html": snapshot.to_html(),
        "diagnostics": projection(&wire::diagnostics(snapshot.diagnostics()))?,
        "gaiji": projection(&wire::gaiji(&snapshot))?,
        "nodes": projection(&wire::nodes(&snapshot))?,
        "pairs": projection(&wire::pairs(&snapshot))?,
        "containerPairs": projection(&wire::container_pairs(&snapshot))?,
        "source": snapshot.to_source(),
    }))
}

pub(crate) fn run() -> anyhow::Result<ExitCode> {
    let stdin = io::stdin();
    let mut stdout = io::BufWriter::new(io::stdout().lock());
    for line in stdin.lock().lines() {
        let line = line.context("read JSONL request")?;
        let response = match serde_json::from_str::<Request>(&line) {
            Ok(input) => match render(&input) {
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
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::{Request, render, wire};

    #[test]
    fn renders_every_release_projection() {
        let result = render(&Request {
            protocol_version: 1,
            id: "cli-0".to_owned(),
            operation: "render".to_owned(),
            source: "｜青空《あおぞら》".to_owned(),
        })
        .expect("valid request");
        assert_eq!(result["schemaVersion"], wire::SCHEMA_VERSION);
        assert!(
            result["html"]
                .as_str()
                .is_some_and(|html| html.contains("<ruby>"))
        );
        assert_eq!(result["source"], "青空《あおぞら》");
    }
}
