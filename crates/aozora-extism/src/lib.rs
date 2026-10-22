//! Provides a portable Extism plugin for parsing Aozora source.
//! Plugin exports accept UTF-8 source and return text or the JSON envelopes defined by [`aozora::json`].

#![forbid(unsafe_code)]

/// Exercises the Extism artifact through a host-side line-delimited JSON protocol.
#[cfg(all(not(target_arch = "wasm32"), any(test, feature = "host-smoke")))]
pub mod worker;

/// Shared parsing and serialization logic for plugin exports and host tests.
#[cfg(any(target_arch = "wasm32", test))]
#[expect(
    clippy::expect_used,
    reason = "guard_len proves the parser's span-limit error is unreachable"
)]
mod logic {
    use aozora::json;

    pub(crate) const MAX_SOURCE_BYTES: usize = u32::MAX as usize;

    pub(crate) const OVERSIZE_MSG: &str = "source exceeds 4 GiB (u32::MAX) span limit";

    pub(crate) const fn guard_len(byte_len: usize) -> Result<(), &'static str> {
        if byte_len as u64 > MAX_SOURCE_BYTES as u64 {
            return Err(OVERSIZE_MSG);
        }
        Ok(())
    }

    pub(crate) fn render_html(source: String) -> Result<String, &'static str> {
        guard_len(source.len())?;
        Ok(aozora::parse(source)
            .expect("source fits parser span limit")
            .snapshot()
            .to_html())
    }

    pub(crate) fn render_source(source: String) -> Result<String, &'static str> {
        guard_len(source.len())?;
        Ok(aozora::parse(source)
            .expect("source fits parser span limit")
            .snapshot()
            .to_source())
    }

    pub(crate) fn render_diagnostics_json(source: String) -> Result<String, &'static str> {
        guard_len(source.len())?;
        Ok(json::diagnostics(
            aozora::parse(source)
                .expect("source fits parser span limit")
                .snapshot()
                .diagnostics(),
        ))
    }

    pub(crate) fn render_nodes_json(source: String) -> Result<String, &'static str> {
        guard_len(source.len())?;
        let doc = aozora::parse(source).expect("source fits parser span limit");
        Ok(json::nodes(&doc.snapshot()))
    }

    pub(crate) fn render_pairs_json(source: String) -> Result<String, &'static str> {
        guard_len(source.len())?;
        let doc = aozora::parse(source).expect("source fits parser span limit");
        Ok(json::pairs(&doc.snapshot()))
    }

    pub(crate) fn render_container_pairs_json(source: String) -> Result<String, &'static str> {
        guard_len(source.len())?;
        let doc = aozora::parse(source).expect("source fits parser span limit");
        Ok(json::container_pairs(&doc.snapshot()))
    }

    pub(crate) fn render_gaiji_json(source: &str) -> Result<String, &'static str> {
        guard_len(source.len())?;
        let document = aozora::parse(source.to_owned()).expect("source fits parser span limit");
        Ok(json::gaiji(&document.snapshot()))
    }

    pub(crate) fn slugs_json() -> String {
        json::slugs()
    }

    pub(crate) fn version() -> &'static str {
        env!("AOZORA_VERSION_STRING")
    }

    pub(crate) const fn schema_version() -> u32 {
        json::SCHEMA_VERSION
    }
}

#[cfg(target_arch = "wasm32")]
mod plugin;

#[cfg(test)]
mod tests {
    use super::logic::{
        MAX_SOURCE_BYTES, OVERSIZE_MSG, guard_len, render_container_pairs_json,
        render_diagnostics_json, render_gaiji_json, render_html, render_nodes_json,
        render_pairs_json, render_source, schema_version, slugs_json, version,
    };
    use aozora::json;

    const CORPUS: [&str; 5] = [
        "plain",
        "｜青梅《おうめ》",
        "abc\u{E001}def",
        "［＃ここから2字下げ］あ［＃ここで字下げ終わり］",
        "※［＃「弓＋鰐のつくり」、第4水準2-84-40］",
    ];

    #[test]
    fn every_serializer_is_byte_identical_to_the_shared_authority() {
        for src in CORPUS {
            let doc = aozora::parse(src.to_owned()).expect("source fits parser span limit");
            let tree = doc.snapshot();
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
                json::gaiji(&tree),
                "gaiji_json src: {src}"
            );
        }
    }

    #[test]
    fn static_exports_match_their_authority() {
        assert_eq!(slugs_json(), json::slugs());
        assert_eq!(version(), env!("AOZORA_VERSION_STRING"));
    }

    #[test]
    fn gaiji_input_produces_non_empty_gaiji_envelope() {
        let out = render_gaiji_json("※［＃「弓＋鰐のつくり」、第4水準2-84-40］")
            .expect("within span limit");
        assert!(
            out.contains(r#""mencode""#),
            "expected a resolved gaiji record: {out}"
        );
    }

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
