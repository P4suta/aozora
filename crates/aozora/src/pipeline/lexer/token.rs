use crate::syntax::Span;

pub(crate) use crate::spec::TriggerKind;

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub(crate) enum Token {
    Text { range: Span },

    Trigger { kind: TriggerKind, span: Span },

    Newline { pos: u32 },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_char_trigger_byte_lens_match_utf8() {
        assert_eq!(TriggerKind::Bar.source_byte_len(), 3);
        assert_eq!(TriggerKind::AngleQuoteOpen.source_byte_len(), 3);
    }
}
