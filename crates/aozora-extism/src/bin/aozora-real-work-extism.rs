//! Serves line-delimited JSON requests through the Extism plugin artifact.

use std::env;
use std::fs;
use std::io;

use anyhow::Context;
use aozora_extism::worker::{Engine, serve};
use extism::{Manifest, Plugin, Wasm};

struct ExtismEngine(Plugin);

impl Engine for ExtismEngine {
    // mutants::skip: `host_smoke` exercises the external runtime behind `Plugin::call`.
    #[cfg_attr(test, mutants::skip)]
    fn invoke(&mut self, name: &str, source: &str) -> anyhow::Result<String> {
        let output: &str = self
            .0
            .call(name, source)
            .with_context(|| format!("call {name}"))?;
        Ok(output.to_owned())
    }
}

// mutants::skip: the release worker smoke covers arguments and stdio, and library tests cover the protocol.
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
