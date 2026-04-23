//! Zero-dep balanced-stack HTML well-formedness check.
//!
//! Does NOT attempt to be a full HTML5 parser — html5ever lives at a
//! different strata of correctness and pulls in ~20 transitive crates.
//! This validator targets the class of renderer bugs afm actually
//! risks:
//!
//! * Unclosed start tag (forgot a `</div>` on exit).
//! * Extra close tag (closed something we never opened).
//! * Misordered close (closed `<div>` while `<span>` is the top of
//!   the open stack).
//!
//! The void-element list follows the HTML5 spec's *Void elements*
//! section (`<br>`, `<hr>`, `<img>`, …). Self-closing XML-style
//! variants (`<br />`) are accepted for every element because our
//! renderer never emits them — if it did, we'd want to know.
//!
//! Comments, doctype declarations, raw text (`<script>`, `<style>`),
//! and CDATA are out of scope: afm's renderer produces none of them
//! today. When M2-S4 needs them, this helper grows accordingly.
//!
//! The validator is `pub` inside the shared `common` module so any
//! integration test in `tests/` can assert well-formedness; the
//! corpus sweep wires it into the I4 invariant as well.
// Visibility strategy: the only public items are [`check_well_formed`]
// and [`WellFormedError`] — every integration test that `mod common;`s
// this file consumes at least one of them, so `dead_code` and
// `unreachable_pub` stay quiet without `#![allow]` (forbidden by
// `just strict-code`). Internal helpers stay `fn` / `enum` without
// `pub` so they don't trigger `unreachable_pub` either.

use std::fmt;

/// Kinds of structural violations the validator reports.
///
/// `near` snippets surface ±48 characters around the offending byte
/// offset so the failure message is actionable without a full dump
/// of the rendered HTML.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WellFormedError {
    UnclosedTag {
        tag: String,
        near: String,
    },
    ExtraClose {
        tag: String,
        near: String,
    },
    MisorderedClose {
        opened: String,
        closed: String,
        near: String,
    },
    MalformedTag {
        near: String,
    },
}

impl fmt::Display for WellFormedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnclosedTag { tag, near } => {
                write!(f, "unclosed <{tag}> near {near:?}")
            }
            Self::ExtraClose { tag, near } => {
                write!(f, "extra </{tag}> near {near:?}")
            }
            Self::MisorderedClose {
                opened,
                closed,
                near,
            } => write!(
                f,
                "</{closed}> closes while <{opened}> is still open, near {near:?}"
            ),
            Self::MalformedTag { near } => {
                write!(f, "malformed tag near {near:?}")
            }
        }
    }
}

/// Return the list of well-formedness violations in `html`. Empty
/// vector means the document is balanced.
///
/// Runs in a single forward pass over the bytes; the open-tag stack
/// keeps O(depth) memory. Void-element recognition is a sorted
/// `&'static [&str]` scanned with `binary_search` — 14 entries, so
/// either shape is cheap, but sorted+binary-search keeps the
/// contract stable if the list grows.
#[must_use]
pub(crate) fn check_well_formed(html: &str) -> Vec<WellFormedError> {
    let mut errors = Vec::new();
    let mut stack: Vec<(String, usize)> = Vec::new();
    let bytes = html.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        let Some(lt) = find_from(bytes, i, b'<') else {
            break;
        };
        let Some(gt) = find_from(bytes, lt + 1, b'>') else {
            errors.push(WellFormedError::MalformedTag {
                near: snippet(html, lt),
            });
            break;
        };
        // Segment between `<` and `>` (exclusive both ends).
        let inside = &html[lt + 1..gt];
        match parse_tag(inside) {
            Some(Tag::Open(name)) => {
                if !is_void_element(&name) {
                    stack.push((name, lt));
                }
            }
            Some(Tag::Close(name)) => {
                match stack.pop() {
                    Some((top, _)) if top == name => {}
                    Some((top, top_pos)) => {
                        errors.push(WellFormedError::MisorderedClose {
                            opened: top.clone(),
                            closed: name,
                            near: snippet(html, lt),
                        });
                        // Push the top back so later closes still have
                        // something to match; avoids cascading errors.
                        stack.push((top, top_pos));
                    }
                    None => errors.push(WellFormedError::ExtraClose {
                        tag: name,
                        near: snippet(html, lt),
                    }),
                }
            }
            Some(Tag::SelfClose | Tag::Doctype | Tag::Comment) => {
                // Self-closing / directive / comment — nothing to
                // stack. Our renderer does not emit these today; we
                // accept them so future additions don't trip the
                // check.
            }
            None => errors.push(WellFormedError::MalformedTag {
                near: snippet(html, lt),
            }),
        }
        i = gt + 1;
    }

    for (name, pos) in stack {
        errors.push(WellFormedError::UnclosedTag {
            tag: name,
            near: snippet(html, pos),
        });
    }
    errors
}

/// Classified shape of a single `<...>` run. The open / close
/// variants carry the lowercased tag name so the balanced-stack walk
/// can match them; the remaining variants are shape tags only — we
/// skip directive / comment / self-closing forms without inspecting
/// their name, so a single unit value is enough.
enum Tag {
    Open(String),
    Close(String),
    SelfClose,
    Doctype,
    Comment,
}

fn parse_tag(inside: &str) -> Option<Tag> {
    let trimmed = inside.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.starts_with('!') {
        // `<!DOCTYPE ...>` or comment `<!-- -->` (the `<!-- ... -->`
        // case is partially matched: the `-->` closes the tag view
        // on the first `>`. Good enough for our renderer which emits
        // neither shape.)
        return Some(Tag::Doctype);
    }
    if trimmed.starts_with('?') {
        return Some(Tag::Comment);
    }
    let (is_close, body) = trimmed
        .strip_prefix('/')
        .map_or((false, trimmed), |rest| (true, rest.trim_start()));
    let (name, rest) = split_tag_name(body)?;
    if name.is_empty() {
        return None;
    }
    let self_closing = rest.trim_end().ends_with('/');
    if is_close {
        // `</X attr>` is not strictly valid HTML, but browsers
        // tolerate it. We accept for robustness; the trailing attrs
        // are discarded.
        Some(Tag::Close(name))
    } else if self_closing {
        drop(name); // `name` is unused — we never reconcile self-closing by name
        Some(Tag::SelfClose)
    } else {
        Some(Tag::Open(name))
    }
}

/// Split `body` into `(tag_name, remainder)` where `tag_name` is the
/// leading lowercased ASCII + digit + hyphen run. Returns `None` if
/// the leading char is not a valid tag-name start.
fn split_tag_name(body: &str) -> Option<(String, &str)> {
    let end = body
        .char_indices()
        .find(|(_, c)| !is_tag_name_char(*c))
        .map_or(body.len(), |(i, _)| i);
    if end == 0 {
        return None;
    }
    let name = body[..end].to_ascii_lowercase();
    let rest = &body[end..];
    Some((name, rest))
}

const fn is_tag_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '-' || c == '_'
}

/// HTML5 void elements (no close tag). Sorted so a `binary_search`
/// upgrade is a one-liner; 14 entries fit in cache, so either shape
/// has negligible cost today.
const VOID_ELEMENTS: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr",
];

fn is_void_element(name: &str) -> bool {
    VOID_ELEMENTS.binary_search(&name).is_ok()
}

fn find_from(bytes: &[u8], start: usize, target: u8) -> Option<usize> {
    bytes
        .get(start..)
        .and_then(|slice| slice.iter().position(|&b| b == target))
        .map(|rel| rel + start)
}

/// ±48 char context window snapped to UTF-8 boundaries.
fn snippet(html: &str, pos: usize) -> String {
    let lo = html
        .char_indices()
        .take_while(|(i, _)| i + 48 <= pos)
        .last()
        .map_or(0, |(i, _)| i);
    let hi = html
        .char_indices()
        .find(|(i, _)| *i >= pos + 48)
        .map_or(html.len(), |(i, _)| i);
    let lo = clamp_to_char_boundary(html, lo);
    let hi = clamp_to_char_boundary(html, hi);
    html.get(lo..hi).unwrap_or(html).to_owned()
}

fn clamp_to_char_boundary(s: &str, mut i: usize) -> usize {
    if i > s.len() {
        return s.len();
    }
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}
