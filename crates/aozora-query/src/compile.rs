//! Query DSL parser. Parses a string into a [`Query`] of
//! pattern atoms.

use aozora_cst::SyntaxKind;
use thiserror::Error;

/// Errors produced by [`compile`].
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum QueryError {
    /// Hit end-of-input before completing a token.
    #[error("query: unexpected end of input at offset {offset}")]
    UnexpectedEnd {
        /// Byte offset into the source string.
        offset: usize,
    },
    /// Saw a character the grammar does not allow at this point.
    #[error("query: unexpected `{found}` at offset {offset}")]
    Unexpected {
        /// The offending character.
        found: char,
        /// Byte offset into the source string.
        offset: usize,
    },
    /// `(SomeKind)` referenced a `SyntaxKind` aozora-cst does not
    /// declare.
    #[error("query: unknown SyntaxKind `{name}` at offset {offset}")]
    UnknownKind {
        /// The unrecognised kind identifier.
        name: String,
        /// Byte offset into the source string.
        offset: usize,
    },
}

/// A compiled query — list of pattern atoms.
#[derive(Debug, Clone)]
pub struct Query {
    pub(crate) patterns: Vec<Pattern>,
}

#[derive(Debug, Clone)]
pub(crate) struct Pattern {
    pub(crate) kind: PatternKind,
    pub(crate) capture: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum PatternKind {
    /// `(SyntaxKindIdent)` — match exact node kind.
    Kind(SyntaxKind),
    /// `(_)` — match any node.
    Any,
}

/// Parse a DSL string into a [`Query`].
///
/// # Errors
///
/// Returns [`QueryError`] for empty queries, malformed syntax, or
/// references to unknown `SyntaxKind` identifiers.
pub fn compile(src: &str) -> Result<Query, QueryError> {
    let mut p = Parser::new(src);
    let mut patterns = Vec::new();
    p.skip_trivia();
    while !p.eof() {
        patterns.push(p.parse_pattern()?);
        p.skip_trivia();
    }
    Ok(Query { patterns })
}

struct Parser<'a> {
    src: &'a str,
    pos: usize,
}

impl<'a> Parser<'a> {
    fn new(src: &'a str) -> Self {
        Self { src, pos: 0 }
    }

    fn eof(&self) -> bool {
        self.pos >= self.src.len()
    }

    fn peek(&self) -> Option<char> {
        self.src[self.pos..].chars().next()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += c.len_utf8();
        Some(c)
    }

    fn skip_trivia(&mut self) {
        while let Some(c) = self.peek() {
            if c.is_whitespace() {
                self.bump();
            } else {
                break;
            }
        }
    }

    fn expect(&mut self, want: char) -> Result<(), QueryError> {
        let offset = self.pos;
        match self.peek() {
            Some(c) if c == want => {
                self.bump();
                Ok(())
            }
            Some(found) => Err(QueryError::Unexpected { found, offset }),
            None => Err(QueryError::UnexpectedEnd { offset }),
        }
    }

    fn parse_pattern(&mut self) -> Result<Pattern, QueryError> {
        self.expect('(')?;
        self.skip_trivia();
        let kind_offset = self.pos;
        let kind = if self.peek() == Some('_') {
            self.bump();
            PatternKind::Any
        } else {
            let ident = self.read_ident(kind_offset)?;
            let parsed = parse_kind(&ident).ok_or(QueryError::UnknownKind {
                name: ident,
                offset: kind_offset,
            })?;
            PatternKind::Kind(parsed)
        };
        self.skip_trivia();
        let capture = if self.peek() == Some('@') {
            self.bump();
            let cap_offset = self.pos;
            Some(self.read_ident(cap_offset)?)
        } else {
            None
        };
        self.skip_trivia();
        self.expect(')')?;
        Ok(Pattern { kind, capture })
    }

    fn read_ident(&mut self, offset: usize) -> Result<String, QueryError> {
        let start = self.pos;
        while let Some(c) = self.peek() {
            if c == '_' || c == '-' || c.is_ascii_alphanumeric() {
                self.bump();
            } else {
                break;
            }
        }
        if self.pos == start {
            return Err(self.peek().map_or_else(
                || QueryError::UnexpectedEnd { offset },
                |found| QueryError::Unexpected { found, offset },
            ));
        }
        Ok(self.src[start..self.pos].to_owned())
    }
}

fn parse_kind(name: &str) -> Option<SyntaxKind> {
    match name {
        "Document" => Some(SyntaxKind::Document),
        "Container" => Some(SyntaxKind::Container),
        "Construct" => Some(SyntaxKind::Construct),
        "Plain" => Some(SyntaxKind::Plain),
        "ConstructText" => Some(SyntaxKind::ConstructText),
        "ContainerOpen" => Some(SyntaxKind::ContainerOpen),
        "ContainerClose" => Some(SyntaxKind::ContainerClose),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_single_kind_pattern() {
        let q = compile("(Construct)").unwrap();
        assert_eq!(q.patterns.len(), 1);
        assert!(matches!(
            q.patterns[0].kind,
            PatternKind::Kind(SyntaxKind::Construct)
        ));
        assert!(q.patterns[0].capture.is_none());
    }

    #[test]
    fn parses_capture() {
        let q = compile("(Construct @c)").unwrap();
        assert_eq!(q.patterns[0].capture.as_deref(), Some("c"));
    }

    #[test]
    fn parses_wildcard() {
        let q = compile("(_ @any)").unwrap();
        assert!(matches!(q.patterns[0].kind, PatternKind::Any));
    }

    #[test]
    fn parses_multiple_patterns() {
        let q = compile("(Construct @c)\n(Container @blk)").unwrap();
        assert_eq!(q.patterns.len(), 2);
    }

    #[test]
    fn unknown_kind_errors() {
        let err = compile("(NotAKind)").unwrap_err();
        match err {
            QueryError::UnknownKind { name, .. } => assert_eq!(name, "NotAKind"),
            other => panic!("expected UnknownKind, got {other:?}"),
        }
    }

    #[test]
    fn unexpected_eof_errors() {
        let err = compile("(Construct").unwrap_err();
        assert!(matches!(err, QueryError::UnexpectedEnd { .. }));
    }

    #[test]
    fn expect_rejects_wrong_open_char() {
        // `expect('(')` must reject a leading non-`(`. If the match guard
        // `c == want` is short-circuited to `true`, the parser would accept
        // `C`, bump it, and then fail later with `UnknownKind` instead.
        let err = compile("Construct)").unwrap_err();
        match err {
            QueryError::Unexpected { found, offset } => {
                assert_eq!(found, 'C');
                assert_eq!(offset, 0);
            }
            other => panic!("expected Unexpected, got {other:?}"),
        }
    }

    #[test]
    fn read_ident_accepts_underscore_and_dash() {
        // The identifier char-class admits `_` and `-` via `||`. If the first
        // `||` becomes `&&`, the class collapses to `is_ascii_alphanumeric()`
        // and the capture is truncated at the first `_`, then `expect(')')`
        // fails. Pin the full multi-punctuation capture to keep both `||`s.
        let q = compile("(_ @a_b-c)").unwrap();
        assert_eq!(q.patterns[0].capture.as_deref(), Some("a_b-c"));
    }

    #[test]
    fn parse_kind_maps_each_name() {
        // Pin every `parse_kind` arm so deleting any single arm (which would
        // fall through to `None`) is observable.
        assert!(matches!(parse_kind("Document"), Some(SyntaxKind::Document)));
        assert!(matches!(
            parse_kind("Container"),
            Some(SyntaxKind::Container)
        ));
        assert!(matches!(
            parse_kind("Construct"),
            Some(SyntaxKind::Construct)
        ));
        assert!(matches!(parse_kind("Plain"), Some(SyntaxKind::Plain)));
        assert!(matches!(
            parse_kind("ConstructText"),
            Some(SyntaxKind::ConstructText)
        ));
        assert!(matches!(
            parse_kind("ContainerOpen"),
            Some(SyntaxKind::ContainerOpen)
        ));
        assert!(matches!(
            parse_kind("ContainerClose"),
            Some(SyntaxKind::ContainerClose)
        ));
        assert!(parse_kind("Nope").is_none());
    }
}
