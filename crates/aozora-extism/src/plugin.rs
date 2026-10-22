//! Exposes parsing and metadata functions through the Extism plugin protocol.
use super::logic;
use extism_pdk::{Error, FnResult, plugin_fn};

/// Parse the input source and return semantic HTML5.
///
/// # Errors
/// Returns an error if the source exceeds [`u32::MAX`] bytes.
#[plugin_fn]
pub fn to_html(input: String) -> FnResult<String> {
    Ok(logic::render_html(input).map_err(Error::msg)?)
}

/// Parse the input source and return canonical Aozora source text.
///
/// # Errors
/// Returns an error if the source exceeds [`u32::MAX`] bytes.
#[plugin_fn]
pub fn to_source(input: String) -> FnResult<String> {
    Ok(logic::render_source(input).map_err(Error::msg)?)
}

/// Parse the input source and return the diagnostics wire envelope.
///
/// # Errors
/// Returns an error if the source exceeds [`u32::MAX`] bytes.
#[plugin_fn]
pub fn diagnostics_json(input: String) -> FnResult<String> {
    Ok(logic::render_diagnostics_json(input).map_err(Error::msg)?)
}

/// Parse the input source and return the source-keyed nodes wire envelope.
///
/// # Errors
/// Returns an error if the source exceeds [`u32::MAX`] bytes.
#[plugin_fn]
pub fn nodes_json(input: String) -> FnResult<String> {
    Ok(logic::render_nodes_json(input).map_err(Error::msg)?)
}

/// Parse the input source and return the matched open/close pairs wire envelope.
///
/// # Errors
/// Returns an error if the source exceeds [`u32::MAX`] bytes.
#[plugin_fn]
pub fn pairs_json(input: String) -> FnResult<String> {
    Ok(logic::render_pairs_json(input).map_err(Error::msg)?)
}

/// Parse the input source and return the container open/close pairs wire envelope.
///
/// # Errors
/// Returns an error if the source exceeds [`u32::MAX`] bytes.
#[plugin_fn]
pub fn container_pairs_json(input: String) -> FnResult<String> {
    Ok(logic::render_container_pairs_json(input).map_err(Error::msg)?)
}

/// Parse the input source and return the resolved `※［＃…］` gaiji references as a wire envelope.
///
/// # Errors
/// Returns an error if the source exceeds [`u32::MAX`] bytes.
#[plugin_fn]
pub fn gaiji_json(input: String) -> FnResult<String> {
    Ok(logic::render_gaiji_json(&input).map_err(Error::msg)?)
}

#[expect(
    clippy::unnecessary_wraps,
    reason = "plugin_fn requires FnResult and discards attributes on each input function"
)]
mod metadata {
    use super::{FnResult, logic, plugin_fn};

    /// Return the static slug catalogue as a wire envelope.
    #[plugin_fn]
    pub fn slugs_json(_input: String) -> FnResult<String> {
        Ok(logic::slugs_json())
    }

    /// Return the parser's channel-aware build version.
    #[plugin_fn]
    pub fn version(_input: String) -> FnResult<String> {
        Ok(logic::version().to_owned())
    }

    /// Return the wire-format schema version as a decimal string.
    #[plugin_fn]
    pub fn schema_version(_input: String) -> FnResult<String> {
        Ok(logic::schema_version().to_string())
    }
}
