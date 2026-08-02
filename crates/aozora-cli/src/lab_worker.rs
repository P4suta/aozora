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

fn serve<R: BufRead, W: Write>(reader: R, writer: &mut W) -> anyhow::Result<()> {
    for line in reader.lines() {
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
        serde_json::to_writer(&mut *writer, &response).context("write JSONL response")?;
        writer.write_all(b"\n")?;
        writer.flush()?;
    }
    Ok(())
}

// The CLI integration smoke owns process stdio; protocol behavior is tested
// without global handles through `serve` below.
#[cfg_attr(test, mutants::skip)]
pub(crate) fn run() -> anyhow::Result<ExitCode> {
    let stdin = io::stdin();
    let mut stdout = io::BufWriter::new(io::stdout().lock());
    serve(stdin.lock(), &mut stdout)?;
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::{Request, projection, render, serve, wire};
    use serde_json::{Value, json};

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
        for name in ["diagnostics", "gaiji", "nodes", "pairs", "containerPairs"] {
            assert!(result[name].is_array(), "{name} must be an array");
        }
        assert!(
            result["nodes"]
                .as_array()
                .is_some_and(|nodes| !nodes.is_empty())
        );
        assert!(
            result["pairs"]
                .as_array()
                .is_some_and(|pairs| !pairs.is_empty())
        );
        assert_eq!(result["source"], "青空《あおぞら》");
    }

    #[test]
    fn projection_requires_a_decodable_data_field() {
        assert_eq!(
            projection(r#"{"schemaVersion":1,"data":["node"]}"#).expect("valid projection"),
            json!(["node"])
        );
        projection(r#"{"schemaVersion":1}"#).expect_err("missing data must fail");
        projection("not JSON").expect_err("malformed JSON must fail");
    }

    #[test]
    fn rejects_each_invalid_request_field() {
        let valid = || Request {
            protocol_version: 1,
            id: "cli-0".to_owned(),
            operation: "render".to_owned(),
            source: "青空".to_owned(),
        };
        let mut invalid_protocol = valid();
        invalid_protocol.protocol_version = 2;
        let mut invalid_operation = valid();
        invalid_operation.operation = "check".to_owned();
        let mut missing_id = valid();
        missing_id.id.clear();

        for input in [invalid_protocol, invalid_operation, missing_id] {
            render(&input).expect_err("invalid request must fail");
        }
    }

    #[test]
    fn serves_success_and_malformed_json_as_framed_jsonl() {
        let input = concat!(
            r#"{"protocolVersion":1,"requestId":"cli-0","operation":"render","source":"｜青空《あおぞら》"}"#,
            "\n{\n"
        );
        let mut output = Vec::new();
        serve(input.as_bytes(), &mut output).expect("serve JSONL");
        let responses = String::from_utf8(output)
            .expect("UTF-8 output")
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).expect("response JSON"))
            .collect::<Vec<_>>();

        assert_eq!(responses.len(), 2);
        assert_eq!(responses[0]["protocolVersion"], 1);
        assert_eq!(responses[0]["requestId"], "cli-0");
        assert_eq!(responses[0]["ok"], true);
        assert_eq!(
            responses[0]["result"]["schemaVersion"],
            wire::SCHEMA_VERSION
        );
        assert!(
            responses[0]["result"]["html"]
                .as_str()
                .is_some_and(|html| html.contains("<ruby>"))
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
