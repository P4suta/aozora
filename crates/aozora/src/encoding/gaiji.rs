#![expect(
    clippy::expect_used,
    reason = "matched gaiji delimiters establish the checked source offsets"
)]

//! Resolves gaiji references through [`lookup`] and scans them through [`gaiji_resolutions`].

use core::fmt;

const DECIMAL_RADIX: u32 = 10;
const HEX_RADIX: u32 = 16;
const MAX_UNICODE_HEX_DIGITS: usize = 6;
const JIS_ROW_CELL_MAX: u8 = 94;

use crate::encoding::jisx0213_table::{
    DESCRIPTION_TO_CHAR, JISX0213_MENCODE_TO_CHAR, JISX0213_MENCODE_TO_STR, ROMAN_NUMERAL_LOWER,
    ROMAN_NUMERAL_UPPER,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Resolved {
    Char(char),
    Multi(&'static str),
}

impl Resolved {
    pub(crate) fn write_to<W: fmt::Write>(self, w: &mut W) -> fmt::Result {
        match self {
            Self::Char(c) => w.write_char(c),
            Self::Multi(s) => w.write_str(s),
        }
    }

    #[must_use]
    pub(crate) fn as_char(self) -> Option<char> {
        match self {
            Self::Char(c) => Some(c),
            Self::Multi(_) => None,
        }
    }

    #[must_use]
    #[cfg(test)]
    pub(crate) fn utf8_len(self) -> usize {
        match self {
            Self::Char(c) => c.len_utf8(),
            Self::Multi(s) => s.len(),
        }
    }
}

#[must_use]
pub(crate) fn lookup(
    existing: Option<char>,
    mencode: Option<&str>,
    description: &str,
) -> Option<Resolved> {
    if let Some(ch) = existing {
        return Some(Resolved::Char(ch));
    }
    if let Some(m) = mencode {
        if let Some(&s) = JISX0213_MENCODE_TO_STR.get(m) {
            return Some(Resolved::Multi(s));
        }
        if let Some(&ch) = JISX0213_MENCODE_TO_CHAR.get(m) {
            return Some(Resolved::Char(ch));
        }
        if let Some(ch) = parse_u_plus(m) {
            return Some(Resolved::Char(ch));
        }
    }
    if let Some(&ch) = DESCRIPTION_TO_CHAR.get(description) {
        return Some(Resolved::Char(ch));
    }
    if let Some(s) = roman_numeral_glyph(description) {
        return Some(Resolved::Multi(s));
    }
    let mut chars = description.chars();
    if let Some(only) = chars.next()
        && chars.next().is_none()
    {
        return Some(Resolved::Char(only));
    }
    None
}

#[must_use]
fn roman_numeral_glyph(description: &str) -> Option<&'static str> {
    let rest = description.strip_prefix("ローマ数字")?;
    let (digits, lower) = rest
        .strip_suffix("小文字")
        .map_or((rest, false), |d| (d, true));
    if digits.is_empty() {
        return None;
    }
    let mut n: usize = 0;
    for ch in digits.chars() {
        let d = ch
            .to_digit(DECIMAL_RADIX)
            .or_else(|| ('０'..='９').contains(&ch).then(|| ch as u32 - '０' as u32))?;
        n = n
            .checked_mul(DECIMAL_RADIX as usize)?
            .checked_add(d as usize)?;
    }
    let table = if lower {
        &ROMAN_NUMERAL_LOWER
    } else {
        &ROMAN_NUMERAL_UPPER
    };
    table.get(n).copied().filter(|s| !s.is_empty())
}

#[must_use]
fn parse_u_plus(mencode: &str) -> Option<char> {
    let hex = mencode.strip_prefix("U+")?;
    if hex.is_empty() || hex.len() > MAX_UNICODE_HEX_DIGITS {
        return None;
    }
    let code = u32::from_str_radix(hex, HEX_RADIX).ok()?;
    char::from_u32(code)
}

#[must_use]
#[cfg(test)]
pub(crate) fn table_sizes() -> (usize, usize, usize) {
    (
        JISX0213_MENCODE_TO_CHAR.len(),
        JISX0213_MENCODE_TO_STR.len(),
        DESCRIPTION_TO_CHAR.len(),
    )
}

pub(crate) const GAIJI_OPEN: &str = "※［＃";
pub(crate) const BRACKET_HASH: &str = "［＃";
pub(crate) const GAIJI_REFMARK: &str = "※";
pub(crate) const GAIJI_CLOSE: &str = "］";
/// A source gaiji reference and its optional Unicode resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GaijiResolution {
    span: crate::Span,
    description: String,
    mencode: Option<String>,
    codepoint: Option<u32>,
    resolved: Option<String>,
}

impl GaijiResolution {
    /// Byte span of the complete reference in the original source.
    #[must_use]
    pub const fn span(&self) -> crate::Span {
        self.span
    }

    /// Description text, without surrounding quotes in the simple form.
    #[must_use]
    pub fn description(&self) -> &str {
        &self.description
    }

    /// Mencode tail, when present.
    #[must_use]
    pub fn mencode(&self) -> Option<&str> {
        self.mencode.as_deref()
    }

    /// Resolved scalar, or `None` for unresolved references and combining sequences.
    #[must_use]
    pub const fn codepoint(&self) -> Option<u32> {
        self.codepoint
    }

    /// Resolved Unicode text, or `None` when lookup fails.
    #[must_use]
    pub fn resolved(&self) -> Option<&str> {
        self.resolved.as_deref()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct GaijiBody<'a> {
    pub description: &'a str,
    pub mencode: Option<&'a str>,
    pub quoted: bool,
    pub mencode_separator: bool,
}

#[must_use]
pub(crate) fn parse_gaiji_body(body: &str) -> GaijiBody<'_> {
    let body = body.trim();
    if let Some(rest) = body.strip_prefix('「')
        && let Some(close) = rest.find('」')
    {
        let desc = &rest[..close];
        let tail = rest[close + '」'.len_utf8()..].trim();
        let separated = tail.strip_prefix('、');
        let bare_mencode_tail = separated.is_none()
            && !tail.is_empty()
            && is_mencode_shaped(mencode_resolution_token(tail));
        if !desc.is_empty()
            && !desc.contains(['「', '」'])
            && (tail.is_empty() || separated.is_some() || bare_mencode_tail)
        {
            let mencode = separated.map_or_else(
                || (!tail.is_empty()).then_some(tail),
                |m| {
                    let m = m.trim();
                    (!m.is_empty()).then_some(m)
                },
            );
            return GaijiBody {
                description: desc,
                mencode,
                quoted: true,
                mencode_separator: separated.is_some(),
            };
        }
    }
    let shaped = |t: &str| is_mencode_shaped(t) || is_near_miss_page_line_shaped(t);
    let commas: Vec<usize> = body.match_indices('、').map(|(i, _)| i).collect();
    let tokens: Vec<&str> = body.split('、').map(str::trim).collect();
    let mut run_start = tokens.len();
    while run_start > 0 && shaped(tokens[run_start - 1]) {
        let previous = run_start;
        run_start = run_start
            .checked_sub(1)
            .expect("loop guard guarantees a predecessor");
        assert!(run_start < previous, "gaiji run scan must move backward");
    }
    let run = &tokens[run_start..];
    let uses_near_miss = run
        .iter()
        .any(|t| is_near_miss_page_line_shaped(t) && !is_page_line_shaped(t));
    let anchored = run.iter().any(|t| is_mencode_shaped(t));
    if run_start == tokens.len() || run_start == 0 || (uses_near_miss && !anchored) {
        return GaijiBody {
            description: body,
            mencode: None,
            quoted: false,
            mencode_separator: true,
        };
    }
    let boundary = commas[run_start - 1];
    GaijiBody {
        description: body[..boundary].trim(),
        mencode: Some(body[boundary + '、'.len_utf8()..].trim()),
        quoted: false,
        mencode_separator: true,
    }
}

#[must_use]
pub(crate) fn gaiji_description_serializable(description: &str, has_mencode: bool) -> bool {
    if description.contains("［＃") {
        return false;
    }
    if description.contains(['「', '」']) {
        let balanced = description.matches('「').count() == description.matches('」').count();
        return balanced && has_mencode;
    }
    true
}

#[must_use]
pub(crate) fn recognize_gaiji_body(body: &str) -> Option<GaijiBody<'_>> {
    let parsed = parse_gaiji_body(body);
    let bare_unanchored = !parsed.quoted && parsed.mencode.is_none();
    let bare_resolvable = DESCRIPTION_TO_CHAR.contains_key(parsed.description)
        || roman_numeral_glyph(parsed.description).is_some();
    if parsed.description.is_empty()
        || (bare_unanchored && !bare_resolvable)
        || !gaiji_description_serializable(parsed.description, parsed.mencode.is_some())
    {
        return None;
    }
    Some(parsed)
}

#[must_use]
pub(crate) fn mencode_resolution_token(mencode: &str) -> &str {
    mencode
        .split_once('、')
        .map_or(mencode, |(token, _)| token.trim())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MenKuTen {
    pub plane: u8,
    pub ku: u8,
    pub ten: u8,
}

impl MenKuTen {
    #[must_use]
    pub(crate) fn level(self) -> u8 {
        self.plane + 2
    }
}

impl fmt::Display for MenKuTen {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "第{}水準{}-{}-{}",
            self.level(),
            self.plane,
            self.ku,
            self.ten
        )
    }
}

#[must_use]
pub(crate) fn parse_menkuten(token: &str) -> Option<MenKuTen> {
    let after = token.strip_prefix('第')?;
    let suijun = after.find("水準")?;
    let _level: u8 = after[..suijun].parse().ok()?;
    let mut parts = after[suijun + "水準".len()..].split('-');
    let plane: u8 = parts.next()?.parse().ok()?;
    let ku: u8 = parts.next()?.parse().ok()?;
    let ten: u8 = parts.next()?.parse().ok()?;
    let mkt = MenKuTen { plane, ku, ten };
    if !(1..=2).contains(&plane)
        || !(1..=JIS_ROW_CELL_MAX).contains(&ku)
        || !(1..=JIS_ROW_CELL_MAX).contains(&ten)
    {
        return None;
    }
    (mkt.to_string() == token).then_some(mkt)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GaijiCanonical<'src> {
    MenKuTen(MenKuTen),
    Unicode(char),
    Unresolved { mencode: Option<&'src str> },
}

impl<'src> GaijiCanonical<'src> {
    #[must_use]
    pub(crate) fn from_mencode(mencode: Option<&'src str>) -> Self {
        if let Some(m) = mencode {
            if let Some(mkt) = parse_menkuten(m) {
                return Self::MenKuTen(mkt);
            }
            if let Some(c) = parse_u_plus(m) {
                return Self::Unicode(c);
            }
        }
        Self::Unresolved { mencode }
    }

    #[must_use]
    pub(crate) fn resolve(self, description: &str) -> Option<Resolved> {
        match self {
            Self::MenKuTen(m) => lookup(None, Some(&m.to_string()), description),
            Self::Unicode(c) => Some(Resolved::Char(c)),
            Self::Unresolved { mencode } => {
                lookup(None, mencode.map(mencode_resolution_token), description)
            }
        }
    }

    #[must_use]
    #[cfg(test)]
    pub(crate) fn has_mencode(self) -> bool {
        !matches!(self, Self::Unresolved { mencode: None })
    }

    #[cfg(any(feature = "pandoc", test))]
    pub(crate) fn write_mencode<W: fmt::Write>(self, w: &mut W) -> fmt::Result {
        match self {
            Self::MenKuTen(m) => write!(w, "{m}"),
            Self::Unicode(c) => write!(w, "U+{:04X}", c as u32),
            Self::Unresolved { mencode } => mencode.map_or(Ok(()), |m| w.write_str(m)),
        }
    }
}

#[must_use]
pub(crate) fn is_mencode_shaped(s: &str) -> bool {
    if let Some(hex) = s.strip_prefix("U+") {
        return !hex.is_empty()
            && hex.len() <= MAX_UNICODE_HEX_DIGITS
            && hex.chars().all(|c| c.is_ascii_hexdigit());
    }
    let rest = s
        .strip_prefix('第')
        .and_then(|after_dai| {
            let nondigit = after_dai.find(|c: char| !c.is_ascii_digit())?;
            let (_digits, tail) = after_dai.split_at(nondigit);
            tail.strip_prefix("水準")
        })
        .unwrap_or(s);
    !rest.is_empty()
        && rest.chars().all(|c| c.is_ascii_digit() || c == '-')
        && rest.chars().any(|c| c.is_ascii_digit())
}

#[must_use]
pub(crate) fn is_page_line_shaped(s: &str) -> bool {
    !s.is_empty() && s.split('-').all(is_page_line_part)
}

fn is_page_line_part(p: &str) -> bool {
    if let Some(volume) = p.strip_suffix('巻') {
        return matches!(volume, "上" | "中" | "下" | "前" | "後") || is_digit_run(volume);
    }
    matches!(p, "上" | "中" | "下") || is_digit_run(p)
}

const COLUMN_MARKERS: [&str; 6] = ["上段", "中段", "下段", "上", "中", "下"];

fn is_near_miss_page_line_shaped(s: &str) -> bool {
    !s.is_empty() && s.split(['-', '－']).all(is_near_miss_page_line_part)
}

fn is_near_miss_page_line_part(p: &str) -> bool {
    if let Some(volume) = p.strip_suffix('巻') {
        return matches!(volume, "上" | "中" | "下" | "前" | "後") || is_digit_run(volume);
    }
    let core = p.strip_prefix('P').unwrap_or(p);
    let core = core.strip_suffix("首目").unwrap_or(core);
    if matches!(core, "上" | "中" | "下") {
        return true;
    }
    if let Some(rest) = COLUMN_MARKERS.iter().find_map(|m| core.strip_prefix(m)) {
        return is_digit_run(rest);
    }
    if let Some(rest) = COLUMN_MARKERS.iter().find_map(|m| core.strip_suffix(m)) {
        return is_digit_run(rest);
    }
    is_digit_run(core)
}

fn is_digit_run(p: &str) -> bool {
    !p.is_empty()
        && p.chars()
            .all(|c| c.is_ascii_digit() || ('０'..='９').contains(&c))
}

#[must_use]
pub(crate) fn resolve_at(source: &str, start: usize, end: usize) -> Option<GaijiResolution> {
    let span = source.get(start..end)?;
    let (open, standalone) = if span.starts_with(GAIJI_OPEN) {
        (GAIJI_OPEN, false)
    } else if span.starts_with(BRACKET_HASH) {
        (BRACKET_HASH, true)
    } else {
        return None;
    };
    let body_start = start.checked_add(open.len())?;
    let body_end = end.checked_sub(GAIJI_CLOSE.len())?;
    if body_end <= body_start {
        return None;
    }
    let body = source.get(body_start..body_end)?;
    if body.contains(['\r', '\n']) {
        return None;
    }
    let GaijiBody {
        description,
        mencode,
        ..
    } = if standalone {
        recognize_gaiji_body(body)?
    } else {
        parse_gaiji_body(body)
    };
    let (resolved, codepoint) = lookup(None, mencode.map(mencode_resolution_token), description)
        .map_or((None, None), |r| {
            let mut s = String::new();
            _ = r.write_to(&mut s);
            (Some(s), r.as_char().map(|c| c as u32))
        });
    Some(GaijiResolution {
        span: crate::Span::new(u32::try_from(start).ok()?, u32::try_from(end).ok()?),
        description: description.to_owned(),
        mencode: mencode.map(str::to_owned),
        codepoint,
        resolved,
    })
}

#[must_use]
pub(crate) fn gaiji_resolutions(source: &str) -> Vec<GaijiResolution> {
    #[derive(Clone, Copy)]
    struct Candidate {
        start: usize,
        end: Option<usize>,
        parent: Option<usize>,
    }

    #[derive(Clone, Copy)]
    struct BracketFrame {
        candidate: Option<usize>,
        previous_candidate: Option<usize>,
    }

    let mut candidates = Vec::new();
    let mut stack = Vec::new();
    let mut active_candidate = None;
    for (offset, ch) in source.char_indices() {
        if ch == '［' {
            let previous_candidate = active_candidate;
            let candidate = source[offset..].starts_with(BRACKET_HASH).then(|| {
                let start = if source[..offset].ends_with(GAIJI_REFMARK) {
                    offset.saturating_sub(GAIJI_REFMARK.len())
                } else {
                    offset
                };
                let index = candidates.len();
                candidates.push(Candidate {
                    start,
                    end: None,
                    parent: previous_candidate,
                });
                active_candidate = Some(index);
                index
            });
            stack.push(BracketFrame {
                candidate,
                previous_candidate,
            });
        } else if matches!(ch, '\r' | '\n') {
            stack.clear();
            active_candidate = None;
        } else if ch == '］'
            && let Some(frame) = stack.pop()
        {
            if let Some(index) = frame.candidate {
                candidates[index].end = Some(offset.saturating_add(ch.len_utf8()));
            }
            active_candidate = frame.previous_candidate;
        }
    }

    let mut out = Vec::new();
    let mut covered = vec![false; candidates.len()];
    for (index, candidate) in candidates.into_iter().enumerate() {
        if candidate.parent.is_some_and(|parent| covered[parent]) {
            covered[index] = true;
            continue;
        }
        if let Some(end) = candidate.end
            && let Some(resolution) = resolve_at(source, candidate.start, end)
        {
            covered[index] = true;
            out.push(resolution);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_gaiji_body_splits_quoted_composed_and_bare() {
        assert_eq!(
            parse_gaiji_body("「木＋吶のつくり」、第3水準1-85-54"),
            GaijiBody {
                description: "木＋吶のつくり",
                mencode: Some("第3水準1-85-54"),
                quoted: true,
                mencode_separator: true,
            }
        );
        assert_eq!(
            parse_gaiji_body("「々」"),
            GaijiBody {
                description: "々",
                mencode: None,
                quoted: true,
                mencode_separator: false,
            }
        );
        assert_eq!(
            parse_gaiji_body("「廰」の「广」を「厂」に、第3水準1-15-94"),
            GaijiBody {
                description: "「廰」の「广」を「厂」に",
                mencode: Some("第3水準1-15-94"),
                quoted: false,
                mencode_separator: true,
            }
        );
        assert_eq!(
            parse_gaiji_body("面から一、二画目をとったもの、第3水準1-15-94"),
            GaijiBody {
                description: "面から一、二画目をとったもの",
                mencode: Some("第3水準1-15-94"),
                quoted: false,
                mencode_separator: true,
            }
        );
        assert_eq!(
            parse_gaiji_body("二の字点、1-2-23"),
            GaijiBody {
                description: "二の字点",
                mencode: Some("1-2-23"),
                quoted: false,
                mencode_separator: true,
            }
        );
        assert_eq!(
            parse_gaiji_body("「※」、第3水準1-84-27、144-上-9"),
            GaijiBody {
                description: "※",
                mencode: Some("第3水準1-84-27、144-上-9"),
                quoted: true,
                mencode_separator: true,
            }
        );
        assert_eq!(
            parse_gaiji_body("二の字点"),
            GaijiBody {
                description: "二の字点",
                mencode: None,
                quoted: false,
                mencode_separator: true,
            }
        );
    }

    #[test]
    fn near_miss_page_line_is_a_superset_of_canonical() {
        for s in ["144-上-9", "7巻-42-下-10", "372-10"] {
            assert!(is_page_line_shaped(s), "canonical {s}");
            assert!(is_near_miss_page_line_shaped(s), "near-miss superset {s}");
        }
        for s in [
            "383-下8",
            "2-下3",
            "323-上1",
            "94－11",
            "92－３",
            "109下－4",
            "69-下段9首目",
            "P61-下段5首目",
        ] {
            assert!(!is_page_line_shaped(s), "canonical must reject {s}");
            assert!(is_near_miss_page_line_shaped(s), "near-miss accepts {s}");
        }
        for s in ["上段", "下段", "中段", "38-上段-1"] {
            assert!(!is_page_line_shaped(s), "canonical rejects bare 段 {s}");
            assert!(
                !is_near_miss_page_line_shaped(s),
                "near-miss rejects bare 段 {s}"
            );
        }
        for s in ["U+304B", "巻-3-4", "X1－1", "漢字", ""] {
            assert!(!is_page_line_shaped(s));
            assert!(!is_near_miss_page_line_shaped(s));
        }
    }

    #[test]
    fn parse_gaiji_body_handles_near_miss_locators() {
        assert_eq!(
            parse_gaiji_body("「※」は「てへん＋劣」、第3水準1-84-77、383-下8"),
            GaijiBody {
                description: "「※」は「てへん＋劣」",
                mencode: Some("第3水準1-84-77、383-下8"),
                quoted: false,
                mencode_separator: true,
            }
        );
        assert_eq!(
            parse_gaiji_body("「金＋夫」第3水準1-93-4"),
            GaijiBody {
                description: "金＋夫",
                mencode: Some("第3水準1-93-4"),
                quoted: true,
                mencode_separator: false,
            }
        );
        assert_eq!(
            parse_gaiji_body(
                "「※」は「いしへん」＋「乏」、読みは「いしばり」、第3水準1-88-93、94－11"
            ),
            GaijiBody {
                description: "「※」は「いしへん」＋「乏」、読みは「いしばり」",
                mencode: Some("第3水準1-88-93、94－11"),
                quoted: false,
                mencode_separator: true,
            }
        );
        assert_eq!(
            parse_gaiji_body("「※」は「王へん」に「干」、第3水準1-87-83、P61-下段5首目"),
            GaijiBody {
                description: "「※」は「王へん」に「干」",
                mencode: Some("第3水準1-87-83、P61-下段5首目"),
                quoted: false,
                mencode_separator: true,
            }
        );
    }

    #[test]
    fn near_miss_locators_resolve_to_the_menkuten_glyph() {
        let body = recognize_gaiji_body("「金＋夫」第3水準1-93-4").expect("shape 2 is a gaiji");
        assert_eq!(
            lookup(
                None,
                body.mencode.map(mencode_resolution_token),
                body.description
            ),
            Some(Resolved::Char('\u{9207}'))
        );
    }

    #[test]
    fn near_miss_page_line_requires_a_mencode_anchor() {
        assert!(recognize_gaiji_body("「あ」に「い」、94－11").is_none());
        assert!(recognize_gaiji_body("「あ」に「い」、383-下8").is_none());
        assert!(recognize_gaiji_body("ここから２段組、上段").is_none());
        assert!(recognize_gaiji_body("底本の閉じ括弧は「』」、58-下15").is_none());
        assert!(recognize_gaiji_body("底本ルビは「もら」と誤記、175-上段-4").is_none());
        assert!(recognize_gaiji_body("「あ」に「い」、第3水準1-15-94、383-下8").is_some());
        assert!(recognize_gaiji_body("小書き片仮名ン、500-下-19").is_some());
        assert!(recognize_gaiji_body("改ページ").is_none());
        assert!(recognize_gaiji_body("ここから2字下げ").is_none());
    }

    #[test]
    fn gaiji_resolutions_empty_for_plain_text() {
        assert_eq!(
            gaiji_resolutions("plain text, no gaiji"),
            Vec::<GaijiResolution>::new()
        );
    }

    #[test]
    fn gaiji_resolutions_resolves_single_char_description() {
        let src = "前※［＃「々」］後";
        let res = gaiji_resolutions(src);
        assert_eq!(res.len(), 1);
        let g = &res[0];
        assert_eq!(g.description(), "々");
        assert_eq!(g.resolved(), Some("々"));
        assert_eq!(g.codepoint(), Some('々' as u32));
        assert_eq!(g.span().slice(src), "※［＃「々」］");
    }

    #[test]
    fn gaiji_resolutions_includes_standalone_form() {
        let src = "前［＃「木＋吶のつくり」、第3水準1-85-54］後";
        let res = gaiji_resolutions(src);
        assert_eq!(res.len(), 1);
        let g = &res[0];
        assert_eq!(g.description(), "木＋吶のつくり");
        assert_eq!(g.mencode(), Some("第3水準1-85-54"));
        assert_eq!(g.resolved(), Some("枘"));
        assert_eq!(
            g.span().slice(src),
            "［＃「木＋吶のつくり」、第3水準1-85-54］"
        );
    }

    #[test]
    fn gaiji_resolutions_rejects_line_crossing_candidates() {
        for line_ending in ["\r", "\n", "\r\n"] {
            let source = format!("前※［＃「々」{line_ending}］後");
            assert!(
                gaiji_resolutions(&source).is_empty(),
                "line ending: {line_ending:?}"
            );
        }
    }

    #[test]
    fn gaiji_resolutions_does_not_pair_with_a_close_on_the_next_line() {
        for line_ending in ["\r", "\n", "\r\n"] {
            let source = format!("※［＃「未閉じ」{line_ending}］※［＃「々」］");
            let resolutions = gaiji_resolutions(&source);
            assert_eq!(resolutions.len(), 1, "line ending: {line_ending:?}");
            assert_eq!(resolutions[0].span().slice(&source), "※［＃「々」］");
        }
    }

    #[test]
    fn gaiji_resolutions_keeps_same_line_candidates() {
        for line_ending in ["\r", "\n", "\r\n"] {
            let source = format!("※［＃「々」］{line_ending}後");
            let resolutions = gaiji_resolutions(&source);
            assert_eq!(resolutions.len(), 1, "line ending: {line_ending:?}");
            assert_eq!(resolutions[0].span().slice(&source), "※［＃「々」］");
        }
    }

    #[test]
    fn gaiji_resolutions_excludes_plain_directives() {
        assert_eq!(
            gaiji_resolutions("本文［＃改ページ］続き"),
            Vec::<GaijiResolution>::new()
        );
        assert_eq!(
            gaiji_resolutions("［＃ここから2字下げ］字下げ"),
            Vec::<GaijiResolution>::new()
        );
        assert_eq!(
            gaiji_resolutions("東京［＃「東京」に傍点］へ"),
            Vec::<GaijiResolution>::new()
        );
    }

    #[test]
    fn gaiji_resolutions_mixes_refmark_and_standalone_in_order() {
        let src = "※［＃「々」］と［＃「木＋吶のつくり」、第3水準1-85-54］";
        let res = gaiji_resolutions(src);
        assert_eq!(res.len(), 2);
        assert_eq!(res[0].span().slice(src), "※［＃「々」］");
        assert_eq!(
            res[1].span().slice(src),
            "［＃「木＋吶のつくり」、第3水準1-85-54］"
        );
    }

    #[test]
    fn gaiji_resolutions_finds_nested_refmark_in_directive() {
        let src = "未［＃「未」の左に「さ※［＃「々」］い」のルビ］";
        let resolutions = gaiji_resolutions(src);
        assert_eq!(resolutions.len(), 1);
        assert_eq!(resolutions[0].span().slice(src), "※［＃「々」］");
        assert_eq!(resolutions[0].resolved(), Some("々"));
    }

    #[test]
    fn gaiji_resolutions_tracks_plain_bracket_nesting() {
        let src = "※［＃「foo［bar］」、U+0061］";
        let resolutions = gaiji_resolutions(src);
        assert_eq!(resolutions.len(), 1);
        assert_eq!(resolutions[0].span().slice(src), src);
        assert_eq!(resolutions[0].description(), "foo［bar］");
        assert_eq!(resolutions[0].resolved(), Some("a"));
        assert_eq!(resolutions[0].codepoint(), Some(0x61));
    }

    #[test]
    fn gaiji_resolutions_keeps_nested_and_adjacent_source_order() {
        let src = "［＃外側［＃内側※［＃「々」］］］※［＃「〆」］［＃「木＋吶のつくり」、第3水準1-85-54］";
        let resolutions = gaiji_resolutions(src);
        assert_eq!(resolutions.len(), 3);
        assert_eq!(resolutions[0].span().slice(src), "※［＃「々」］");
        assert_eq!(resolutions[1].span().slice(src), "※［＃「〆」］");
        assert_eq!(
            resolutions[2].span().slice(src),
            "［＃「木＋吶のつくり」、第3水準1-85-54］"
        );
    }

    #[test]
    fn gaiji_resolutions_handles_malformed_outer_directive() {
        let src = "［＃閉じない外側※［＃「々」］";
        let resolutions = gaiji_resolutions(src);
        assert_eq!(resolutions.len(), 1);
        assert_eq!(resolutions[0].span().slice(src), "※［＃「々」］");
        assert_eq!(
            gaiji_resolutions("［＃外側［＃内側"),
            Vec::<GaijiResolution>::new()
        );
    }

    #[test]
    fn gaiji_resolutions_does_not_duplicate_nested_body() {
        let src = "※［＃外字の説明に［＃「々」］を含む］";
        let resolutions = gaiji_resolutions(src);
        assert_eq!(resolutions.len(), 1);
        assert_eq!(resolutions[0].span().slice(src), src);
    }

    #[test]
    fn gaiji_resolutions_handles_deep_nesting() {
        let depth = 4_096;
        let mut src = "※［＃".repeat(depth);
        src.push_str("「々」");
        src.push_str(&"］".repeat(depth));
        let resolutions = gaiji_resolutions(&src);
        assert_eq!(resolutions.len(), 1);
        assert_eq!(resolutions[0].span().slice(&src), src);
    }

    #[test]
    fn recognize_gaiji_body_gates_directives() {
        assert!(recognize_gaiji_body("改ページ").is_none());
        assert!(recognize_gaiji_body("ここから2字下げ").is_none());
        assert!(recognize_gaiji_body("「々」").is_some());
        assert!(recognize_gaiji_body("「desc」、第3水準1-85-54").is_some());
        assert!(recognize_gaiji_body("ローマ数字17").is_some());
        assert!(recognize_gaiji_body("ローマ数字23").is_some());
        assert!(recognize_gaiji_body("ローマ数字").is_none());
        assert!(recognize_gaiji_body("ローマ数字0").is_none());
    }

    #[test]
    fn roman_numeral_glyph_composes_from_the_u2160_block() {
        assert_eq!(
            roman_numeral_glyph("ローマ数字17"),
            Some("\u{2169}\u{2164}\u{2160}\u{2160}")
        );
        assert_eq!(
            roman_numeral_glyph("ローマ数字13"),
            Some("\u{2169}\u{2160}\u{2160}\u{2160}")
        );
        assert_eq!(
            roman_numeral_glyph("ローマ数字20"),
            Some("\u{2169}\u{2169}")
        );
        assert_eq!(
            roman_numeral_glyph("ローマ数字4小文字"),
            Some("\u{2170}\u{2174}")
        );
        assert_eq!(
            roman_numeral_glyph("ローマ数字１７"),
            Some("\u{2169}\u{2164}\u{2160}\u{2160}")
        );
        assert_eq!(roman_numeral_glyph("二重かっこ開く"), None);
        assert_eq!(roman_numeral_glyph("ローマ数字"), None);
        assert_eq!(roman_numeral_glyph("ローマ数字0"), None);
    }

    #[test]
    fn lookup_prefers_existing_ucs_when_already_set() {
        assert_eq!(
            lookup(Some('\u{1234}'), Some("第3水準1-85-54"), "木＋吶のつくり"),
            Some(Resolved::Char('\u{1234}'))
        );
    }

    #[test]
    fn lookup_via_mencode_table_when_ucs_missing() {
        assert_eq!(
            lookup(None, Some("第3水準1-85-54"), "木＋吶のつくり"),
            Some(Resolved::Char('\u{6798}'))
        );
    }

    #[test]
    fn lookup_via_combo_table_returns_multi() {
        assert_eq!(
            lookup(None, Some("第3水準1-4-87"), ""),
            Some(Resolved::Multi("\u{304B}\u{309A}"))
        );
    }

    #[test]
    fn combo_resolution_writes_both_codepoints() {
        let resolved = lookup(None, Some("第3水準1-4-87"), "").expect("combo resolves");
        let mut s = String::new();
        resolved
            .write_to(&mut s)
            .expect("write to String never fails");
        assert_eq!(s, "\u{304B}\u{309A}");
        assert_eq!(s.chars().count(), 2);
    }

    #[test]
    fn lookup_via_u_plus_form() {
        assert_eq!(
            lookup(None, Some("U+01F5"), "Latin Small Letter G With Acute"),
            Some(Resolved::Char('\u{01F5}'))
        );
    }

    #[test]
    fn lookup_via_u_plus_max_six_hex_digits() {
        assert_eq!(
            lookup(None, Some("U+10FFFF"), ""),
            Some(Resolved::Char('\u{10FFFF}'))
        );
    }

    #[test]
    fn lookup_rejects_u_plus_beyond_seven_hex_digits() {
        assert_eq!(lookup(None, Some("U+1234567"), ""), None);
    }

    #[test]
    fn lookup_rejects_u_plus_surrogate() {
        assert_eq!(lookup(None, Some("U+D800"), ""), None);
    }

    #[test]
    fn lookup_rejects_u_plus_non_hex() {
        assert_eq!(lookup(None, Some("U+GG12"), ""), None);
    }

    #[test]
    fn lookup_rejects_u_plus_without_digits() {
        assert_eq!(lookup(None, Some("U+"), ""), None);
    }

    #[test]
    fn lookup_via_description_fallback_when_mencode_absent() {
        assert_eq!(lookup(None, None, "〓"), Some(Resolved::Char('\u{3013}')));
    }

    #[test]
    fn lookup_returns_none_when_all_paths_miss() {
        assert_eq!(
            lookup(None, Some("not-in-any-table"), "unresolved gaiji"),
            None
        );
    }

    #[test]
    fn lookup_falls_back_to_description_self_when_single_char() {
        assert_eq!(
            lookup(None, Some("第4水準2-16-1"), "丂"),
            Some(Resolved::Char('\u{4E02}'))
        );
        assert_eq!(lookup(None, None, "畺"), Some(Resolved::Char('\u{757A}')));
        assert_eq!(lookup(None, None, "龔"), Some(Resolved::Char('\u{9F94}')));
    }

    #[test]
    fn single_char_fallback_does_not_override_dictionary_hit() {
        assert_eq!(lookup(None, None, "〓"), Some(Resolved::Char('\u{3013}')));
    }

    #[test]
    fn single_char_fallback_does_not_fire_for_multi_char_descriptions() {
        assert_eq!(lookup(None, None, "未知の字形"), None);
        assert_eq!(lookup(None, None, "ab"), None);
    }

    #[test]
    fn mencode_table_covers_the_fixture_gaiji() {
        assert_eq!(
            JISX0213_MENCODE_TO_CHAR.get("第3水準1-85-54"),
            Some(&'\u{6798}')
        );
    }

    #[test]
    fn table_sizes_match_jisx0213_2004_spec() {
        use crate::encoding::jisx0213_table::{
            DESCRIPTION_COUNT, JISX0213_COMBO_COUNT, JISX0213_PLANE1_COUNT, JISX0213_PLANE2_COUNT,
        };
        let (single, combo, description) = table_sizes();
        assert_eq!(single, JISX0213_PLANE1_COUNT + JISX0213_PLANE2_COUNT);
        assert_eq!(combo, JISX0213_COMBO_COUNT);
        assert_eq!(description, DESCRIPTION_COUNT);
        assert_eq!(
            JISX0213_PLANE1_COUNT, 1893,
            "第3水準 must equal the spec count",
        );
        assert_eq!(
            JISX0213_PLANE2_COUNT, 2436,
            "第4水準 must equal the spec count",
        );
        assert_eq!(
            JISX0213_COMBO_COUNT, 25,
            "combining-sequence cells must equal spec",
        );
        assert!(
            description >= 8_000,
            "description-fallback table looks too small ({description}) — \
             did the gaiji-chuki extraction drop entries?",
        );
    }

    #[test]
    fn description_table_resolves_a_known_dictionary_entry() {
        assert_eq!(
            lookup(None, None, "木＋吶のつくり"),
            Some(Resolved::Char('\u{6798}')),
        );
    }

    #[test]
    fn description_table_preserves_special_placeholders() {
        assert_eq!(lookup(None, None, "〓"), Some(Resolved::Char('\u{3013}')));
        assert_eq!(lookup(None, None, "〻"), Some(Resolved::Char('\u{303B}')));
    }

    #[test]
    fn full_jisx0213_table_covers_a_known_plane1_third_tier_kanji() {
        assert_eq!(
            JISX0213_MENCODE_TO_CHAR.get("第3水準1-85-9"),
            Some(&'\u{6567}')
        );
    }

    #[test]
    fn full_jisx0213_table_covers_a_known_plane2_fourth_tier_entry() {
        assert_eq!(
            JISX0213_MENCODE_TO_CHAR.get("第4水準2-1-1"),
            Some(&'\u{20089}')
        );
    }

    #[test]
    fn resolved_utf8_len_matches_actual_encoding() {
        assert_eq!(Resolved::Char('A').utf8_len(), 1);
        assert_eq!(Resolved::Char('あ').utf8_len(), 3);
        assert_eq!(Resolved::Char('𠂉').utf8_len(), 4);
        assert_eq!(Resolved::Multi("\u{304B}\u{309A}").utf8_len(), 6);
    }

    #[test]
    fn resolved_as_char_returns_none_for_combos() {
        assert_eq!(Resolved::Char('A').as_char(), Some('A'));
        assert_eq!(Resolved::Multi("か゚").as_char(), None);
    }

    #[test]
    fn lookup_is_identity_on_the_ucs_input_when_set() {
        assert_eq!(
            lookup(Some('あ'), Some("anything"), "anything"),
            Some(Resolved::Char('あ'))
        );
    }

    #[test]
    fn menkuten_round_trips_through_display() {
        let m = parse_menkuten("第3水準1-85-54").expect("clean men-ku-ten parses");
        assert_eq!(
            m,
            MenKuTen {
                plane: 1,
                ku: 85,
                ten: 54
            }
        );
        assert_eq!(m.level(), 3);
        assert_eq!(m.to_string(), "第3水準1-85-54");
        let m4 = parse_menkuten("第4水準2-1-1").expect("plane-2 parses");
        assert_eq!(
            m4,
            MenKuTen {
                plane: 2,
                ku: 1,
                ten: 1
            }
        );
        assert_eq!(m4.to_string(), "第4水準2-1-1");
    }

    #[test]
    fn parse_menkuten_rejects_non_canonical_forms() {
        assert!(parse_menkuten("1-2-23").is_none());
        assert!(parse_menkuten("第3水準1-84-27、144-上-9").is_none());
        assert!(parse_menkuten("第3水準2-1-1").is_none());
        assert!(parse_menkuten("第3水準1-05-4").is_none());
        assert!(parse_menkuten("U+74FC").is_none());
    }

    #[test]
    fn gaiji_canonical_classifies_only_clean_forms() {
        assert_eq!(
            GaijiCanonical::from_mencode(Some("第3水準1-85-54")),
            GaijiCanonical::MenKuTen(MenKuTen {
                plane: 1,
                ku: 85,
                ten: 54
            })
        );
        assert_eq!(
            GaijiCanonical::from_mencode(Some("U+74FC")),
            GaijiCanonical::Unicode('\u{74FC}')
        );
        assert_eq!(
            GaijiCanonical::from_mencode(Some("U+74FC、372-10")),
            GaijiCanonical::Unresolved {
                mencode: Some("U+74FC、372-10")
            }
        );
        assert_eq!(
            GaijiCanonical::from_mencode(Some("1-2-23")),
            GaijiCanonical::Unresolved {
                mencode: Some("1-2-23")
            }
        );
        assert_eq!(
            GaijiCanonical::from_mencode(None),
            GaijiCanonical::Unresolved { mencode: None }
        );
    }

    #[test]
    fn gaiji_canonical_resolve_matches_legacy_lookup() {
        assert_eq!(
            GaijiCanonical::from_mencode(Some("第3水準1-85-54")).resolve("木＋吶のつくり"),
            Some(Resolved::Char('\u{6798}'))
        );
        assert_eq!(
            GaijiCanonical::from_mencode(Some("U+74FC")).resolve(""),
            Some(Resolved::Char('\u{74FC}'))
        );
        assert_eq!(
            GaijiCanonical::from_mencode(Some("U+74FC、372-10")).resolve(""),
            Some(Resolved::Char('\u{74FC}'))
        );
    }

    #[test]
    fn gaiji_canonical_write_mencode_reproduces_source() {
        let render = |c: GaijiCanonical<'_>| {
            let mut s = String::new();
            c.write_mencode(&mut s).unwrap();
            s
        };
        assert_eq!(
            render(GaijiCanonical::from_mencode(Some("第3水準1-85-54"))),
            "第3水準1-85-54"
        );
        assert_eq!(
            render(GaijiCanonical::from_mencode(Some("U+74FC"))),
            "U+74FC"
        );
        assert_eq!(
            render(GaijiCanonical::from_mencode(Some(
                "第3水準1-84-27、144-上-9"
            ))),
            "第3水準1-84-27、144-上-9"
        );
        GaijiCanonical::from_mencode(None)
            .write_mencode(&mut String::new())
            .unwrap();
        assert!(!GaijiCanonical::from_mencode(None).has_mencode());
    }

    #[test]
    fn parse_u_plus_length_guard_is_all_or_nothing() {
        assert_eq!(parse_u_plus("U+41"), Some('A'));
        assert_eq!(parse_u_plus("U+10FFFF"), Some('\u{10FFFF}'));
        assert_eq!(parse_u_plus("U+"), None);
        assert_eq!(parse_u_plus("U+00010FF"), None);
        assert_eq!(parse_u_plus("第3水準1-85-54"), None);
    }

    #[test]
    fn parse_gaiji_body_run_consuming_the_whole_body_keeps_it_as_description() {
        assert_eq!(
            parse_gaiji_body("1-2-3"),
            GaijiBody {
                description: "1-2-3",
                mencode: None,
                quoted: false,
                mencode_separator: true,
            }
        );
    }

    #[test]
    fn gaiji_description_serializable_rejects_leaky_and_unbalanced() {
        assert!(gaiji_description_serializable("木＋吶のつくり", false));
        assert!(gaiji_description_serializable("木＋吶のつくり", true));
        assert!(!gaiji_description_serializable("外字［＃注記", false));
        assert!(!gaiji_description_serializable("外字［＃注記", true));
        assert!(!gaiji_description_serializable("「廰」の「广」", false));
        assert!(gaiji_description_serializable("「廰」の「广」", true));
        assert!(!gaiji_description_serializable("「廰", true));
    }

    #[test]
    fn parse_menkuten_enforces_coordinate_bounds() {
        assert!(parse_menkuten("第2水準0-1-1").is_none(), "plane 0 rejected");
        assert!(parse_menkuten("第3水準1-0-1").is_none(), "ku 0 rejected");
        assert!(parse_menkuten("第3水準1-1-0").is_none(), "ten 0 rejected");
        assert!(parse_menkuten("第5水準3-1-1").is_none(), "plane 3 rejected");
        assert!(parse_menkuten("第3水準1-95-1").is_none(), "ku 95 rejected");
        assert!(parse_menkuten("第3水準1-1-95").is_none(), "ten 95 rejected");
        assert_eq!(
            parse_menkuten("第3水準1-1-1"),
            Some(MenKuTen {
                plane: 1,
                ku: 1,
                ten: 1
            })
        );
        assert_eq!(
            parse_menkuten("第4水準2-94-94"),
            Some(MenKuTen {
                plane: 2,
                ku: 94,
                ten: 94
            })
        );
    }

    #[test]
    fn gaiji_canonical_has_mencode_tracks_the_tail() {
        assert!(GaijiCanonical::from_mencode(Some("第3水準1-85-54")).has_mencode());
        assert!(GaijiCanonical::from_mencode(Some("U+74FC")).has_mencode());
        assert!(GaijiCanonical::from_mencode(Some("1-2-23")).has_mencode());
        assert!(!GaijiCanonical::from_mencode(None).has_mencode());
    }

    #[test]
    fn is_mencode_shaped_u_plus_branch_boundaries() {
        assert!(is_mencode_shaped("U+41"));
        assert!(is_mencode_shaped("U+10FFFF"));
        assert!(!is_mencode_shaped("U+"));
        assert!(!is_mencode_shaped("U+1234567"));
        assert!(!is_mencode_shaped("U+GG12"));
    }

    #[test]
    fn resolve_at_rejects_empty_body_span() {
        let src = "※［＃］";
        assert!(resolve_at(src, 0, src.len()).is_none());
    }

    #[test]
    fn resolve_at_rejects_line_crossing_spans() {
        for line_ending in ["\r", "\n", "\r\n"] {
            let source = format!("※［＃「々」{line_ending}］");
            assert!(
                resolve_at(&source, 0, source.len()).is_none(),
                "line ending: {line_ending:?}"
            );
        }
    }
}
