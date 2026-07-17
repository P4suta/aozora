//! Extism plugin exports. Compiled only when targeting `wasm32`; each
//! function is a thin wrapper that delegates to [`logic`](super::logic) and
//! maps the span-limit error onto the Extism error channel.
//!
//! This module lives in its own file (rather than inline in `lib.rs`) so the
//! mutation sweep can exclude it by glob: on a host (`x86_64` / `aarch64`)
//! build the whole module is `cfg`-dead, so cargo-mutants would otherwise
//! enumerate every wrapper and report each mutant as a vacuous survivor. The
//! real, host-testable logic stays in [`super::logic`] and is swept normally.
//! See `mutants.toml`'s `exclude_globs`.
#![allow(
    clippy::unnecessary_wraps,
    reason = "the #[plugin_fn] macro requires every export to return FnResult, even an infallible one"
)]

use super::logic;
use extism_pdk::{Error, FnResult, plugin_fn};

/// Parse the input source and return semantic HTML5.
#[plugin_fn]
pub fn to_html(input: String) -> FnResult<String> {
    Ok(logic::render_html(input).map_err(Error::msg)?)
}

/// Parse the input source and re-emit it as Aozora source text
/// (round-trip serialization).
#[plugin_fn]
pub fn to_source(input: String) -> FnResult<String> {
    Ok(logic::render_source(input).map_err(Error::msg)?)
}

/// Parse the input source and return the diagnostics wire envelope
/// (`{ "schemaVersion": 2, "data": [ … ] }`).
#[plugin_fn]
pub fn diagnostics_json(input: String) -> FnResult<String> {
    Ok(logic::render_diagnostics_json(input).map_err(Error::msg)?)
}

/// Parse the input source and return the source-keyed nodes wire
/// envelope.
#[plugin_fn]
pub fn nodes_json(input: String) -> FnResult<String> {
    Ok(logic::render_nodes_json(input).map_err(Error::msg)?)
}

/// Parse the input source and return the matched open/close pairs
/// wire envelope.
#[plugin_fn]
pub fn pairs_json(input: String) -> FnResult<String> {
    Ok(logic::render_pairs_json(input).map_err(Error::msg)?)
}

/// Parse the input source and return the container open/close pairs
/// wire envelope.
#[plugin_fn]
pub fn container_pairs_json(input: String) -> FnResult<String> {
    Ok(logic::render_container_pairs_json(input).map_err(Error::msg)?)
}

/// Parse the input source and return the resolved `※［＃…］` gaiji
/// references as a wire envelope.
#[plugin_fn]
pub fn gaiji_json(input: String) -> FnResult<String> {
    Ok(logic::render_gaiji_json(&input).map_err(Error::msg)?)
}

/// Return the static spec slug catalogue as a wire envelope. Input is
/// ignored; the same envelope every call, so hosts can cache it. Powers
/// `［＃…］` annotation completion.
#[plugin_fn]
pub fn slugs_json(_input: String) -> FnResult<String> {
    Ok(logic::slugs_json())
}

/// Return the parser's channel-aware build version (e.g. `0.5.0`).
/// Input is ignored; hosts call this to surface the plugin build in a
/// footer / diagnostics, distinct from the wire `schema_version`.
#[plugin_fn]
pub fn version(_input: String) -> FnResult<String> {
    Ok(logic::version().to_owned())
}

/// Return the wire-format schema version as a decimal string. Input
/// is ignored; hosts call this with empty input to assert
/// plugin/SDK compatibility before parsing.
#[plugin_fn]
pub fn schema_version(_input: String) -> FnResult<String> {
    Ok(logic::schema_version().to_string())
}
