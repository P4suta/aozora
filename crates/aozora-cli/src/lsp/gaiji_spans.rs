use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GaijiSpan {
    pub start_byte: u32,
    pub end_byte: u32,
    pub description: Arc<str>,
    pub mencode: Option<Arc<str>>,
}

#[must_use]
pub(super) fn extract_gaiji_spans(snapshot: &aozora::Snapshot) -> Arc<[Arc<GaijiSpan>]> {
    snapshot
        .gaiji_resolutions()
        .iter()
        .map(|resolution| {
            let span = resolution.span();
            Arc::new(GaijiSpan {
                start_byte: span.start,
                end_byte: span.end,
                description: Arc::from(resolution.description()),
                mencode: resolution.mencode().map(Arc::from),
            })
        })
        .collect::<Vec<_>>()
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(src: &str) -> aozora::Snapshot {
        aozora::parse(src).expect("small source").snapshot()
    }

    #[test]
    fn empty_source_yields_no_spans() {
        assert!(extract_gaiji_spans(&parse("")).is_empty());
    }

    #[test]
    fn extracts_one_gaiji_span() {
        let src = "前※［＃「desc」、第3水準1-85-54］後";
        let spans = extract_gaiji_spans(&parse(src));
        assert_eq!(spans.len(), 1);
        let span = &spans[0];
        assert_eq!(&*span.description, "desc");
        assert_eq!(span.mencode.as_deref(), Some("第3水準1-85-54"));
        assert_eq!(span.start_byte as usize, src.find('※').unwrap());
    }

    #[test]
    fn extracts_multiple_spans_in_source_order() {
        let src = "※［＃「a」、第3水準1-85-54］\n※［＃「b」、第3水準1-85-9］";
        let spans = extract_gaiji_spans(&parse(src));
        assert_eq!(spans.len(), 2);
        assert!(spans[0].start_byte < spans[1].start_byte);
        assert_eq!(&*spans[0].description, "a");
        assert_eq!(&*spans[1].description, "b");
    }

    #[test]
    fn description_only_form_yields_none_mencode() {
        let src = "※［＃「desc-only」］";
        let spans = extract_gaiji_spans(&parse(src));
        assert_eq!(spans.len(), 1);
        assert_eq!(&*spans[0].description, "desc-only");
        assert!(spans[0].mencode.is_none());
    }
}
