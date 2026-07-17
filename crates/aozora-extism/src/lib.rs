//! Extism plugin driver for the aozora parser.
//!
//! Compiles to a single portable `wasm32-unknown-unknown` artifact that
//! any language with an Extism host SDK (Go / Java / PHP / Ruby / … —
//! ~15 languages) can load. Each exported `#[plugin_fn]` takes the
//! Aozora source text as input bytes and returns HTML or a wire-format
//! JSON envelope as output bytes — the same "text in → bytes out"
//! contract as the C ABI driver ([`aozora-ffi`]), and byte-identical to
//! it because every JSON path delegates to [`aozora::json`], the single
//! cross-driver authority.
//!
//! ## Why Extism (and not just the C ABI)
//!
//! The C ABI ([`aozora-ffi`]) reaches any language too, but every
//! consuming language must ship a native library built for every
//! `(OS × arch)` pair. This crate collapses that matrix to **one**
//! portable `.wasm`: the bytes are identical on every platform, and the
//! per-language work shrinks to a thin host-SDK wrapper plus types
//! generated from the wire JSON Schema.
//!
//! ## Build targeting
//!
//! The `#[plugin_fn]` exports below are gated on
//! `cfg(target_arch = "wasm32")` so host builds of the cargo workspace
//! (`x86_64`, `aarch64`) skip them entirely and compile only the
//! host-testable `logic` module. Build the artifact with
//! `cargo build --release --target wasm32-unknown-unknown -p
//! aozora-extism` (see `just extism-build`).
//!
//! ## Difference from `aozora-wasm`
//!
//! Both compile to `wasm32-unknown-unknown`, but they speak different
//! ABIs and serve disjoint consumers:
//!
//! - `aozora-wasm` uses the **wasm-bindgen** ABI for the **browser
//!   playground** and carries browser-only primitives (gaiji-at-offset,
//!   profiling, prewarm). Extism cannot serve those, so the playground
//!   keeps wasm-bindgen.
//! - `aozora-extism` (this crate) uses the **Extism host/plugin
//!   protocol** for **polyglot host SDKs**. Browser-only primitives are
//!   intentionally NOT duplicated here.
//!
//! Both share the same parser core and the same `aozora::json`
//! serialization, so neither duplicates parsing or rendering logic.
//!
//! ## Wire format
//!
//! Every JSON-returning plugin function delegates to [`aozora::json`]:
//!
//! ```json
//! { "schemaVersion": 2, "data": [ … ] }
//! ```
//!
//! [`aozora::json::SCHEMA_VERSION`] bumps on any breaking change to that
//! shape. The `schema_version` plugin export lets a host assert wasm/SDK
//! compatibility at load time.
//!
//! [`aozora-ffi`]: https://p4suta.github.io/aozora/api/aozora_ffi/index.html

#![forbid(unsafe_code)]

/// Stateless parse-and-serialize logic, shared by the `wasm32` plugin
/// exports and the host unit tests.
///
/// Gated on `cfg(any(target_arch = "wasm32", test))` so a plain host
/// build — which compiles neither the [`plugin`] exports nor the tests —
/// does not trip the workspace `dead_code = "deny"` lint. Mirrors how
/// `aozora-wasm` gates its `MAX_SOURCE_BYTES` guard.
#[cfg(any(target_arch = "wasm32", test))]
mod logic {
    use aozora::{Document, json};

    /// Largest input the parser core accepts, in bytes. Its span
    /// offsets are `u32`, so a longer source trips a `u32::MAX` assert
    /// inside the lexer; under `panic = "abort"` that tears down the
    /// whole Wasm instance instead of surfacing a recoverable error.
    /// Mirrors `aozora-ffi`'s `SourceTooLarge` guard and `aozora-wasm`'s
    /// `MAX_SOURCE_BYTES`.
    pub(crate) const MAX_SOURCE_BYTES: usize = u32::MAX as usize;

    /// Error message returned when an input exceeds [`MAX_SOURCE_BYTES`].
    pub(crate) const OVERSIZE_MSG: &str = "source exceeds 4 GiB (u32::MAX) span limit";

    /// Reject an oversize source before the parser core's `u32`-span
    /// assert can fire (and, under `panic = "abort"`, abort the host).
    ///
    /// # Errors
    ///
    /// `Err(OVERSIZE_MSG)` when `byte_len > MAX_SOURCE_BYTES`.
    pub(crate) const fn guard_len(byte_len: usize) -> Result<(), &'static str> {
        // Compared in `u64` so the bound bites on 64-bit hosts yet is not
        // a tautology on 32-bit wasm32 — there `usize == u32` already
        // caps the length, so the guard is vacuously satisfied. A direct
        // `usize` comparison would make clippy reject it as an "absurd
        // extreme comparison" (the RHS would be `usize::MAX` on wasm32).
        if byte_len as u64 > MAX_SOURCE_BYTES as u64 {
            return Err(OVERSIZE_MSG);
        }
        Ok(())
    }

    /// Parse `source` and render it to semantic HTML5.
    ///
    /// # Errors
    ///
    /// `Err` when the source exceeds the parser's span limit (see
    /// [`guard_len`]).
    pub(crate) fn render_html(source: String) -> Result<String, &'static str> {
        guard_len(source.len())?;
        Ok(Document::new(source).parse().to_html())
    }

    /// Parse `source` and re-emit it as Aozora source text (round-trip
    /// serialization).
    ///
    /// # Errors
    ///
    /// `Err` when the source exceeds the parser's span limit (see
    /// [`guard_len`]).
    pub(crate) fn render_source(source: String) -> Result<String, &'static str> {
        guard_len(source.len())?;
        Ok(Document::new(source).parse().to_source())
    }

    /// Parse `source` and serialize its diagnostics through the shared
    /// [`aozora::json`] authority. Empty document →
    /// `{"schemaVersion":2,"data":[]}`.
    ///
    /// # Errors
    ///
    /// `Err` when the source exceeds the parser's span limit (see
    /// [`guard_len`]).
    pub(crate) fn render_diagnostics_json(source: String) -> Result<String, &'static str> {
        guard_len(source.len())?;
        Ok(json::diagnostics(
            Document::new(source).parse().diagnostics(),
        ))
    }

    /// Parse `source` and serialize its source-keyed nodes through the
    /// shared [`aozora::json`] authority.
    ///
    /// # Errors
    ///
    /// `Err` when the source exceeds the parser's span limit (see
    /// [`guard_len`]).
    pub(crate) fn render_nodes_json(source: String) -> Result<String, &'static str> {
        guard_len(source.len())?;
        let doc = Document::new(source);
        Ok(json::nodes(&doc.parse()))
    }

    /// Parse `source` and serialize its matched open/close pairs through
    /// the shared [`aozora::json`] authority.
    ///
    /// # Errors
    ///
    /// `Err` when the source exceeds the parser's span limit (see
    /// [`guard_len`]).
    pub(crate) fn render_pairs_json(source: String) -> Result<String, &'static str> {
        guard_len(source.len())?;
        let doc = Document::new(source);
        Ok(json::pairs(&doc.parse()))
    }

    /// Parse `source` and serialize its container open/close pairs
    /// through the shared [`aozora::json`] authority.
    ///
    /// # Errors
    ///
    /// `Err` when the source exceeds the parser's span limit (see
    /// [`guard_len`]).
    pub(crate) fn render_container_pairs_json(source: String) -> Result<String, &'static str> {
        guard_len(source.len())?;
        let doc = Document::new(source);
        Ok(json::container_pairs(&doc.parse()))
    }

    /// Parse `source` and serialize its resolved `※［＃…］` gaiji
    /// references through the shared [`aozora::json`] authority.
    ///
    /// # Errors
    ///
    /// `Err` when the source exceeds the parser's span limit (see
    /// [`guard_len`]).
    pub(crate) fn render_gaiji_json(source: &str) -> Result<String, &'static str> {
        guard_len(source.len())?;
        Ok(json::gaiji(source))
    }

    /// Serialize the static spec slug catalogue through the shared
    /// [`aozora::json`] authority. Input-independent — the same envelope
    /// every call — so hosts can cache it. Powers `［＃…］` completion.
    pub(crate) fn slugs_json() -> String {
        json::slugs()
    }

    /// The parser's channel-aware build version (e.g. `0.5.0`,
    /// `0.5.0-dev+g3672e3f`). Single authority: the
    /// `AOZORA_VERSION_STRING` this crate's `build.rs` injects — never a
    /// hard-coded literal. Mirrors `aozora-wasm`'s `version()`.
    pub(crate) fn version() -> &'static str {
        env!("AOZORA_VERSION_STRING")
    }

    /// The wire-format schema version this plugin emits. Hosts assert it
    /// against their own expected [`aozora::json::SCHEMA_VERSION`] at
    /// load time to catch a plugin/SDK version skew.
    pub(crate) const fn schema_version() -> u32 {
        json::SCHEMA_VERSION
    }
}

// The Extism plugin exports (`wasm32`-only) live in their own file so the
// mutation sweep can exclude the whole module by glob: on a host build it is
// `cfg`-dead and every thin-wrapper mutant would be a vacuous survivor. The
// real, host-testable code stays in `logic` above. See `mutants.toml`.
#[cfg(target_arch = "wasm32")]
mod plugin;

#[cfg(test)]
mod tests {
    use super::logic::{
        MAX_SOURCE_BYTES, OVERSIZE_MSG, guard_len, render_container_pairs_json,
        render_diagnostics_json, render_gaiji_json, render_html, render_nodes_json,
        render_pairs_json, render_source, schema_version, slugs_json, version,
    };
    use aozora::{Document, json};

    /// Inputs that, between them, exercise every serializer with
    /// non-empty data: plain text, a ruby span (nodes + pairs), a
    /// PUA-collision (diagnostics), an indent container (container
    /// pairs), and a gaiji reference (gaiji). The first three mirror the
    /// cross-driver `wire.rs` tests.
    const CORPUS: [&str; 5] = [
        "plain",
        "｜青梅《おうめ》",
        "abc\u{E001}def",
        "［＃ここから2字下げ］あ［＃ここで字下げ終わり］",
        "※［＃「弓＋鰐のつくり」、第4水準2-84-40］",
    ];

    /// The whole point of routing through `aozora::json`: every plugin
    /// output must be byte-identical to calling the parser / wire
    /// authority directly — which is in turn byte-identical to the FFI /
    /// WASM / `PyO3` drivers. One assertion per serializer, per input.
    #[test]
    fn every_serializer_is_byte_identical_to_the_shared_authority() {
        for src in CORPUS {
            let doc = Document::new(src.to_owned());
            let tree = doc.parse();
            assert_eq!(
                render_html(src.to_owned()).expect("within span limit"),
                tree.to_html(),
                "to_html src: {src}"
            );
            assert_eq!(
                render_source(src.to_owned()).expect("within span limit"),
                tree.to_source(),
                "to_source src: {src}"
            );
            assert_eq!(
                render_diagnostics_json(src.to_owned()).expect("within span limit"),
                json::diagnostics(tree.diagnostics()),
                "diagnostics_json src: {src}"
            );
            assert_eq!(
                render_nodes_json(src.to_owned()).expect("within span limit"),
                json::nodes(&tree),
                "nodes_json src: {src}"
            );
            assert_eq!(
                render_pairs_json(src.to_owned()).expect("within span limit"),
                json::pairs(&tree),
                "pairs_json src: {src}"
            );
            assert_eq!(
                render_container_pairs_json(src.to_owned()).expect("within span limit"),
                json::container_pairs(&tree),
                "container_pairs_json src: {src}"
            );
            assert_eq!(
                render_gaiji_json(src).expect("within span limit"),
                json::gaiji(src),
                "gaiji_json src: {src}"
            );
        }
    }

    /// The static-catalogue exports (`slugs`, `version`) take no source,
    /// so they sit outside the per-input loop. Each must be byte-identical
    /// to its shared authority.
    #[test]
    fn static_exports_match_their_authority() {
        assert_eq!(slugs_json(), json::slugs());
        assert_eq!(version(), env!("AOZORA_VERSION_STRING"));
    }

    /// The gaiji corpus input must actually resolve — otherwise the gaiji
    /// serializer is only ever exercised on empty data and a regression
    /// there would pass silently.
    #[test]
    fn gaiji_input_produces_non_empty_gaiji_envelope() {
        let out = render_gaiji_json("※［＃「弓＋鰐のつくり」、第4水準2-84-40］")
            .expect("within span limit");
        assert!(
            out.contains(r#""mencode""#),
            "expected a resolved gaiji record: {out}"
        );
    }

    /// Guard against the corpus silently failing to exercise the
    /// container path: the indent input must yield a non-empty
    /// container-pairs envelope.
    #[test]
    fn container_input_produces_non_empty_container_pairs() {
        let out = render_container_pairs_json(
            "［＃ここから2字下げ］あ［＃ここで字下げ終わり］".to_owned(),
        )
        .expect("within span limit");
        assert!(
            out.contains(r#""kind":"indent""#),
            "expected an indent container pair: {out}"
        );
    }

    #[test]
    fn schema_version_matches_wire() {
        assert_eq!(schema_version(), json::SCHEMA_VERSION);
    }

    #[test]
    fn oversize_guard_rejects_above_limit_and_allows_at_limit() {
        assert_eq!(
            guard_len(MAX_SOURCE_BYTES.saturating_add(1)),
            Err(OVERSIZE_MSG)
        );
        assert_eq!(guard_len(MAX_SOURCE_BYTES), Ok(()));
        assert_eq!(guard_len(0), Ok(()));
    }
}
