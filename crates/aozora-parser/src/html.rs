//! HTML rendering front-door.
//!
//! Fully fleshed out once the comrak fork is wired up. Today this is just the public
//! surface that downstream consumers (afm-cli, afm-book) can refer to without waiting.

/// Render `input` to HTML with afm defaults.
///
/// Stub — returns the input unchanged until the parser is fully wired.
#[must_use]
pub fn render_to_string(input: &str) -> String {
    input.to_owned()
}
