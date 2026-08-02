//! Long-lived JSONL adapter for the exact Extism plugin artifact.

#![allow(
    clippy::print_stdout,
    reason = "the worker owns the JSONL stdout transport"
)]

use std::env;
use std::fs;
use std::io;

use anyhow::Context;
use aozora_extism::worker::{Engine, serve};
use extism::{Manifest, Plugin, Wasm};

struct ExtismEngine(Plugin);

impl Engine for ExtismEngine {
    // The exact plugin ABI is exercised by `tests/host_smoke.rs`; a unit test
    // cannot replace the external Extism runtime behind `Plugin::call`.
    #[cfg_attr(test, mutants::skip)]
    fn invoke(&mut self, name: &str, source: &str) -> anyhow::Result<String> {
        let output: &str = self
            .0
            .call(name, source)
            .with_context(|| format!("call {name}"))?;
        Ok(output.to_owned())
    }
}

// Argument and stdio ownership are covered by the release worker smoke; the
// request protocol itself is unit-tested through the library worker module.
#[cfg_attr(test, mutants::skip)]
fn main() -> anyhow::Result<()> {
    let artifact = env::args_os()
        .nth(1)
        .context("usage: aozora-real-work-extism <aozora.wasm>")?;
    let manifest = Manifest::new([Wasm::data(fs::read(&artifact).context("read plugin")?)]);
    let mut plugin = ExtismEngine(Plugin::new(&manifest, [], false).context("instantiate plugin")?);
    let stdin = io::stdin();
    let mut stdout = io::BufWriter::new(io::stdout().lock());
    serve(&mut plugin, stdin.lock(), &mut stdout)
}
