//! WASM driver for the aozora parser.
//!
//! Compiles to a `wasm32-unknown-unknown` artifact suitable for
//! `wasm-pack build --target web`, exposing `aozora::Document` /
//! `aozora::AozoraTree` equivalents that JS / TypeScript consumers
//! can `import { Document } from "aozora-wasm"`.
//!
//! ## Build targeting
//!
//! The wasm-bindgen exports below are gated on
//! `cfg(target_arch = "wasm32")` so host builds of the cargo
//! workspace (`x86_64`, `aarch64`) skip them entirely. Add
//! `wasm32-unknown-unknown` via `rustup target add` before invoking
//! `wasm-pack build --target web --release crates/aozora-wasm`.
//!
//! When `aozora-scan` grows `wasm_simd` backend support, this crate's
//! release build picks it up via `-Ctarget-feature=+simd128`. The
//! size budget for the resulting `.wasm` artifact (post `wasm-opt
//! -O3 --enable-simd`) is ≤ 500 KiB.
//!
//! ## Wire format
//!
//! Every JSON-returning method delegates to [`aozora::wire`], the
//! single authority for the cross-driver wire shape. `aozora-ffi` /
//! `aozora-wasm` / `aozora-py` emit byte-identical envelopes:
//!
//! ```json
//! { "schema_version": 1, "data": [ … ] }
//! ```
//!
//! [`aozora::wire::SCHEMA_VERSION`] bumps on any breaking change to
//! that shape.

#![forbid(unsafe_code)]

#[cfg(target_arch = "wasm32")]
mod bindings {
    use aozora::{Document as AozoraDoc, SLUGS, SlugFamily, encoding::gaiji, wire};
    use wasm_bindgen::prelude::*;

    /// All canonical slugs from the spec, packaged in the standard
    /// wire envelope so JS completion menus can drive a
    /// `［＃...］` catalogue without re-implementing the table.
    ///
    /// Each `data[]` entry: `{ canonical, family, accepts_param, doc, partner }`.
    /// `family` is the camelCase form of the Rust enum variant.
    #[wasm_bindgen]
    #[must_use]
    pub fn slugs_json() -> String {
        let entries: Vec<serde_json::Value> = SLUGS
            .iter()
            .map(|s| {
                let family = match s.family {
                    SlugFamily::PageBreak => "pageBreak",
                    SlugFamily::Section => "section",
                    SlugFamily::BlockContainerOpen => "blockContainerOpen",
                    SlugFamily::BlockContainerClose => "blockContainerClose",
                    SlugFamily::LeafAlign => "leafAlign",
                    SlugFamily::Bouten => "bouten",
                    SlugFamily::Sashie => "sashie",
                    SlugFamily::Keigakomi => "keigakomi",
                    SlugFamily::Warichu => "warichu",
                    SlugFamily::TateChuYoko => "tateChuYoko",
                    SlugFamily::KaeritenSingle => "kaeritenSingle",
                    SlugFamily::KaeritenCompound => "kaeritenCompound",
                    // SlugFamily is `#[non_exhaustive]`: any family
                    // added in a newer spec version surfaces as
                    // "unknown" so JS clients can ignore unfamiliar
                    // entries without crashing.
                    _ => "unknown",
                };
                serde_json::json!({
                    "canonical": s.canonical,
                    "family": family,
                    "accepts_param": s.accepts_param,
                    "doc": s.doc,
                    "partner": s.partner,
                })
            })
            .collect();
        serde_json::json!({
            "schema_version": 1,
            "data": entries,
        })
        .to_string()
    }

    /// Force one-time parser-table initialisation (SIMD backend choice +
    /// annotation-classifier DFA) off the first-keystroke critical path.
    /// Idempotent. The playground calls this right after `init()`
    /// resolves — before the editor is created — so the first keystroke
    /// parse does not pay the DFA build.
    #[wasm_bindgen]
    pub fn prewarm() {
        aozora::prewarm();
    }

    const GAIJI_OPEN: &str = "※［＃";
    const GAIJI_CLOSE: &str = "］";
    // Bounded window for the cursor-pinned hover variant. A real
    // ※［＃...］ span is at most a few hundred bytes; capping the
    // search makes per-keystroke resolution O(window) rather than
    // O(doc).
    const MAX_GAIJI_SPAN_LEN: usize = 512;

    /// High-resolution wall-clock for the profile helper. Returns
    /// milliseconds (f64) using the browser `performance.now()` so
    /// sub-millisecond precision is preserved. Falls back to 0.0 if
    /// the host has no `Performance` interface (e.g. a Worker that
    /// hasn't been told to expose one) — the deltas come out as
    /// constant 0 in that case, which is the least-surprising
    /// fallback for the profile UI.
    fn now_ms() -> f64 {
        web_sys::window()
            .and_then(|w| w.performance())
            .map(|p| p.now())
            .unwrap_or(0.0)
    }

    /// JS-facing handle to a parsed Aozora document.
    ///
    /// Wraps an [`aozora::Document`] (which owns both the source and
    /// the bumpalo arena that backs the borrowed AST). Drop is
    /// automatic when the JS-side handle is GC'd.
    #[wasm_bindgen]
    pub struct Document {
        inner: AozoraDoc,
    }

    #[wasm_bindgen]
    impl Document {
        /// Construct from a UTF-16 JS string. The string is copied
        /// once into the Document's internal `Box<str>`; subsequent
        /// renders reuse the bumpalo arena.
        #[wasm_bindgen(constructor)]
        #[must_use]
        pub fn new(source: String) -> Self {
            Self {
                inner: AozoraDoc::new(source),
            }
        }

        /// Render the document to a semantic-HTML5 string.
        #[wasm_bindgen]
        #[must_use]
        pub fn to_html(&self) -> String {
            self.inner.parse().to_html()
        }

        /// Re-emit Aozora source text from the parse tree.
        #[wasm_bindgen]
        #[must_use]
        pub fn serialize(&self) -> String {
            self.inner.parse().serialize()
        }

        /// Diagnostics as JSON. Empty parse →
        /// `{"schema_version":1,"data":[]}`. Wire format defined in
        /// [`aozora::wire`].
        #[wasm_bindgen]
        #[must_use]
        pub fn diagnostics_json(&self) -> String {
            wire::serialize_diagnostics(self.inner.parse().diagnostics())
        }

        /// Source-keyed Aozora-node spans as JSON. Each entry is
        /// `{ kind, span: { start, end } }` where `kind` is the
        /// camelCase [`aozora::AozoraNode`] discriminant
        /// (`"ruby"` / `"bouten"` / `"gaiji"` / …) plus
        /// `"containerOpen"` / `"containerClose"` for container
        /// open / close markers. `span` covers source bytes, sorted
        /// by `span.start`.
        ///
        /// Stream-friendly for the aozora-obsidian Lezer-Tree builder
        /// — the underlying `source_nodes` table tiles spans
        /// contiguously by construction.
        #[wasm_bindgen]
        #[must_use]
        pub fn nodes_json(&self) -> String {
            wire::serialize_nodes(&self.inner.parse())
        }

        /// Matched open/close pair links as JSON. Each entry is
        /// `{ kind, open: { start, end }, close: { start, end } }` in
        /// sanitized-source coordinates. Useful for LSP requests like
        /// `textDocument/linkedEditingRange` and
        /// `textDocument/documentHighlight`.
        ///
        /// Unmatched closes and unclosed opens are excluded — they
        /// have no partner span and would only confuse editor
        /// surfaces.
        #[wasm_bindgen]
        #[must_use]
        pub fn pairs_json(&self) -> String {
            wire::serialize_pairs(&self.inner.parse())
        }

        /// Source byte length. Useful for JS-side progress UI.
        #[wasm_bindgen]
        #[must_use]
        pub fn source_byte_len(&self) -> usize {
            self.inner.source().len()
        }

        /// Per-method timing snapshot for the current source.
        ///
        /// Each entry is `{ "name": string, "duration_ms": number }`.
        /// Timings are taken via `performance.now()` on the host
        /// (`Instant::now()` panics on `wasm32-unknown-unknown`).
        ///
        /// `parse` is the cost of `Document::parse()` alone
        /// (constructing the borrowed AST). The render entries
        /// (`to_html`, `serialize`, `*_json`) are wall-clock for
        /// that single method call against the already-built tree —
        /// so summing them is the cost of "produce every output JS
        /// might want", and `parse` shows how much of the work
        /// happens up-front in the parser core itself.
        #[wasm_bindgen]
        #[must_use]
        pub fn profile_json(&self) -> String {
            let p0 = now_ms();
            let tree = self.inner.parse();
            let p1 = now_ms();

            let h0 = now_ms();
            let _html = tree.to_html();
            let h1 = now_ms();

            let s0 = now_ms();
            let _serialized = tree.serialize();
            let s1 = now_ms();

            let d0 = now_ms();
            let _diag = wire::serialize_diagnostics(tree.diagnostics());
            let d1 = now_ms();

            let n0 = now_ms();
            let _nodes = wire::serialize_nodes(&tree);
            let n1 = now_ms();

            let pa0 = now_ms();
            let _pairs = wire::serialize_pairs(&tree);
            let pa1 = now_ms();

            // gaiji_resolutions_json scans the source string directly
            // (not the parse tree), so we time it separately by
            // invoking the same code path the JS-side method uses.
            let g0 = now_ms();
            let _gaiji = self.gaiji_resolutions_json();
            let g1 = now_ms();

            let entries = serde_json::json!([
                { "name": "parse",             "duration_ms": p1  - p0  },
                { "name": "to_html",           "duration_ms": h1  - h0  },
                { "name": "serialize",         "duration_ms": s1  - s0  },
                { "name": "diagnostics_json",  "duration_ms": d1  - d0  },
                { "name": "nodes_json",        "duration_ms": n1  - n0  },
                { "name": "pairs_json",        "duration_ms": pa1 - pa0 },
                { "name": "gaiji_resolutions", "duration_ms": g1  - g0  },
            ]);
            serde_json::json!({
                "schema_version": 1,
                "byte_len": self.inner.source().len(),
                "data": entries,
            })
            .to_string()
        }

        /// Resolve the gaiji reference at `byte_offset` (if any) and
        /// return a JSON object describing it. Returns the literal
        /// string `"null"` if the offset is not inside a gaiji span
        /// or the body cannot be parsed.
        ///
        /// JSON shape on hit:
        /// ```json
        /// { "span": { "start": u, "end": u },
        ///   "description": string,
        ///   "mencode": string | null,
        ///   "codepoint": u32 | null,
        ///   "resolved": string | null }
        /// ```
        ///
        /// Locality: the scan is bounded to a 512-byte window either
        /// side of `byte_offset`, so the cost is independent of
        /// document size. Editors call this on every cursor move.
        #[wasm_bindgen]
        #[must_use]
        pub fn resolve_gaiji_at(&self, byte_offset: usize) -> String {
            let source = self.inner.source();
            find_gaiji_span_local(source, byte_offset)
                .and_then(|span| build_resolution_value(source, span.0, span.1))
                .map(|v| v.to_string())
                .unwrap_or_else(|| "null".to_owned())
        }

        /// All gaiji resolutions found in the document, packaged in
        /// the standard `{schema_version, data:[...]}` wire envelope.
        /// Powers inlay-hint UIs that show `→GLYPH` after every
        /// `※［＃…］` span.
        ///
        /// Walks the source linearly once; cost is `O(source)`.
        #[wasm_bindgen]
        #[must_use]
        pub fn gaiji_resolutions_json(&self) -> String {
            let source = self.inner.source();
            let mut entries: Vec<serde_json::Value> = Vec::new();
            let mut cursor = 0usize;
            while let Some(rel) = source[cursor..].find(GAIJI_OPEN) {
                let span_start = cursor + rel;
                let body_start = span_start + GAIJI_OPEN.len();
                let Some(close_rel) = source[body_start..].find(GAIJI_CLOSE) else {
                    break;
                };
                let span_end = body_start + close_rel + GAIJI_CLOSE.len();
                if let Some(val) = build_resolution_value(source, span_start, span_end) {
                    entries.push(val);
                }
                cursor = span_end;
            }
            serde_json::json!({
                "schema_version": 1,
                "data": entries,
            })
            .to_string()
        }
    }

    /// Find the byte-range of the `※［＃…］` span containing
    /// `byte_offset`, scanning only a bounded window around the
    /// cursor (kept independent of document size).
    fn find_gaiji_span_local(source: &str, byte_offset: usize) -> Option<(usize, usize)> {
        if source.is_empty() {
            return None;
        }
        let win_start =
            snap_to_char_boundary_left(source, byte_offset.saturating_sub(MAX_GAIJI_SPAN_LEN));
        let win_end = snap_to_char_boundary_right(
            source,
            byte_offset
                .saturating_add(MAX_GAIJI_SPAN_LEN)
                .min(source.len()),
        );
        let window = &source[win_start..win_end];
        let win_offset = byte_offset.saturating_sub(win_start);

        for (start_in_win, _) in window.match_indices(GAIJI_OPEN) {
            let after_open = start_in_win + GAIJI_OPEN.len();
            let Some(end_rel) = window.get(after_open..).and_then(|s| s.find(GAIJI_CLOSE)) else {
                continue;
            };
            let end_in_win = after_open + end_rel + GAIJI_CLOSE.len();
            if (start_in_win..end_in_win).contains(&win_offset) {
                return Some((win_start + start_in_win, win_start + end_in_win));
            }
        }
        None
    }

    const fn snap_to_char_boundary_left(s: &str, mut idx: usize) -> usize {
        while idx > 0 && !s.is_char_boundary(idx) {
            idx -= 1;
        }
        idx
    }

    const fn snap_to_char_boundary_right(s: &str, mut idx: usize) -> usize {
        let len = s.len();
        while idx < len && !s.is_char_boundary(idx) {
            idx += 1;
        }
        idx
    }

    /// Split a gaiji body (`「description」、mencode[、page-line]`)
    /// into `(description, mencode?)`. Tail fields after the mencode
    /// (page-line refs) are informational only — drop them.
    fn parse_gaiji_body(body: &str) -> (String, Option<String>) {
        let body = body.trim();
        let (description, rest) = body.find('「').map_or_else(
            || (body.to_owned(), ""),
            |open_idx| {
                let after_open = &body[open_idx + '「'.len_utf8()..];
                after_open.find('」').map_or_else(
                    || (body.to_owned(), ""),
                    |close_rel| {
                        let desc = after_open[..close_rel].to_owned();
                        let rest = &after_open[close_rel + '」'.len_utf8()..];
                        (desc, rest)
                    },
                )
            },
        );
        let rest = rest.trim_start_matches('、').trim();
        let mencode = rest
            .split('、')
            .next()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
        (description, mencode)
    }

    /// Build the JSON resolution object for a `※［＃…］` span at
    /// `[start..end)`. Returns `None` if the body cannot be parsed.
    fn build_resolution_value(source: &str, start: usize, end: usize) -> Option<serde_json::Value> {
        // Defensive: `end` should always come from the same scan, but
        // out-of-band callers could pass arbitrary offsets, so
        // validate boundaries.
        let body_start = start.checked_add(GAIJI_OPEN.len())?;
        let body_end = end.checked_sub(GAIJI_CLOSE.len())?;
        if body_end <= body_start || body_end > source.len() {
            return None;
        }
        let body = source.get(body_start..body_end)?;
        let (description, mencode) = parse_gaiji_body(body);
        let resolved = gaiji::lookup(None, mencode.as_deref(), &description);
        let (resolved_str, codepoint) = match resolved {
            Some(r) => {
                let mut s = String::new();
                _ = r.write_to(&mut s);
                let cp = r.as_char().map(|c| c as u32);
                (Some(s), cp)
            }
            None => (None, None),
        };
        Some(serde_json::json!({
            "span": { "start": start, "end": end },
            "description": description,
            "mencode": mencode,
            "codepoint": codepoint,
            "resolved": resolved_str,
        }))
    }
}

#[cfg(test)]
mod tests {
    use aozora::{Document, wire};

    /// Diagnostics for plain input is the empty envelope.
    #[test]
    fn diagnostics_json_is_empty_envelope_for_clean_input() {
        let doc = Document::new("plain".to_owned());
        let json = wire::serialize_diagnostics(doc.parse().diagnostics());
        assert_eq!(json, r#"{"schema_version":1,"data":[]}"#);
    }

    /// PUA collision shows up as a `kind:"source_contains_pua"` entry
    /// inside the envelope.
    #[test]
    fn diagnostics_json_emits_pua_diagnostic() {
        let doc = Document::new("abc\u{E001}def".to_owned());
        let json = wire::serialize_diagnostics(doc.parse().diagnostics());
        assert!(
            json.contains(r#""kind":"source_contains_pua""#),
            "json missing diag kind: {json}"
        );
        assert!(
            json.contains(r#""schema_version":1"#),
            "json missing schema_version: {json}"
        );
    }

    /// Round-trip JSON parse: every wire output must be valid JSON
    /// that decodes to a `{ schema_version, data }` object.
    #[test]
    fn diagnostics_json_round_trips_envelope() {
        let doc = Document::new("abc\u{E001}def".to_owned());
        let json = wire::serialize_diagnostics(doc.parse().diagnostics());
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
        assert!(parsed.is_object(), "wire root must be object");
        assert_eq!(
            parsed
                .get("schema_version")
                .and_then(serde_json::Value::as_u64),
            Some(1)
        );
        assert!(parsed.get("data").is_some_and(serde_json::Value::is_array));
    }

    /// Plain input has no Aozora-classified spans → empty envelope.
    #[test]
    fn nodes_json_is_empty_envelope_for_plain_text() {
        let doc = Document::new("hello, world".to_owned());
        let json = wire::serialize_nodes(&doc.parse());
        assert_eq!(json, r#"{"schema_version":1,"data":[]}"#);
    }

    /// Ruby span emits a `kind:"ruby"` entry.
    #[test]
    fn nodes_json_classifies_ruby() {
        let doc = Document::new("｜青梅《おうめ》".to_owned());
        let json = wire::serialize_nodes(&doc.parse());
        assert!(
            json.contains(r#""kind":"ruby""#),
            "json should mark ruby: {json}"
        );
    }

    /// Round-trip: every wire output is valid JSON with the expected
    /// envelope shape.
    #[test]
    fn nodes_json_round_trips_as_envelope() {
        let doc = Document::new("｜山《やま》や［＃改ページ］\n《《秘密》》".to_owned());
        let json = wire::serialize_nodes(&doc.parse());
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
        let arr = parsed
            .get("data")
            .and_then(|v| v.as_array())
            .expect("data is array");
        assert!(!arr.is_empty(), "should have classified at least one node");
        for entry in arr {
            assert!(entry.get("kind").is_some(), "entry missing kind");
            let span = entry.get("span").expect("entry missing span");
            assert!(span.get("start").is_some(), "span missing start");
            assert!(span.get("end").is_some(), "span missing end");
        }
    }

    /// Source-order property: `data` entries are sorted by
    /// `span.start` ascending.
    #[test]
    fn nodes_json_spans_are_in_source_order() {
        let doc = Document::new("｜山《やま》。｜川《かわ》。｜空《そら》。".to_owned());
        let json = wire::serialize_nodes(&doc.parse());
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
        let arr = parsed
            .get("data")
            .and_then(|v| v.as_array())
            .expect("data is array");
        let starts: Vec<u64> = arr
            .iter()
            .filter_map(|v| {
                v.get("span")
                    .and_then(|s| s.get("start"))
                    .and_then(serde_json::Value::as_u64)
            })
            .collect();
        let mut sorted = starts.clone();
        sorted.sort_unstable();
        assert_eq!(starts, sorted, "spans must be emitted in source order");
    }

    /// Ruby pair appears in `pairs_json`.
    #[test]
    fn pairs_json_emits_ruby_pair() {
        let doc = Document::new("｜青梅《おうめ》".to_owned());
        let json = wire::serialize_pairs(&doc.parse());
        assert!(json.contains(r#""kind":"ruby""#), "pairs json: {json}");
        assert!(json.contains(r#""open":"#), "pairs json: {json}");
        assert!(json.contains(r#""close":"#), "pairs json: {json}");
    }
}
