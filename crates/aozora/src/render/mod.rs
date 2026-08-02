//! HTML / Aozora-source renderers over the semantic AST.
//!
//! Rendering options used by [`crate::Snapshot`] to emit semantic HTML5 or
//! canonical Aozora source text.
//!
//! # Public surface
//!
//! - [`RenderOptions`] configures [`crate::Snapshot::to_html_with`].
//! - [`SerializeOptions`] configures [`crate::Snapshot::to_source_with`].
//! - [`DirectiveNormalization`] controls opt-in canonical directive handling.

#![forbid(unsafe_code)]

mod classes;
mod html;
mod render_node;
mod serialize;
pub(crate) mod spelling;
mod walk;

pub(crate) const MAX_NESTED_SOURCE_DEPTH: usize = 64;

pub use classes::AOZORA_CLASSES;
pub use html::RenderOptions;
pub(crate) use html::{render_html, render_html_normalized};
pub use serialize::{DirectiveNormalization, SerializeOptions};
pub(crate) use serialize::{requires_verbatim_recovery, serialize, serialize_with};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::lex;

    #[test]
    fn html_renders_plain_text_in_paragraph() {
        let out = lex("hello, world");
        let html = render_html(&out);
        assert!(html.contains("hello, world"), "html: {html}");
    }

    #[test]
    fn serialize_round_trips_plain_text() {
        let out = lex("plain text");
        let s = serialize(&out);
        assert_eq!(s, "plain text");
    }

    #[test]
    fn structured_ruby_base_renders_and_serializes_once() {
        let source = "｜瑞岩東畔命［＃二］軽舟［＃一］《ずいがんとうはんめいずけいしうを》";
        let out = lex(source);
        let html = render_html(&out);
        assert!(html.contains(
            "<ruby>瑞岩東畔命<sup class=\"aozora-kaeriten\">二</sup>軽舟<sup class=\"aozora-kaeriten\">一</sup>"
        ));
        assert!(!html.contains('｜'));
        assert_eq!(serialize(&out), source);
    }

    #[test]
    fn ruby_reading_forward_format_keeps_visible_styled_text() {
        let source = "折口《ツムレ［＃「ムレ」に白丸傍点］》";
        let out = lex(source);
        let html = render_html(&out);
        assert!(html.contains(
            "<rt>ツ<em class=\"aozora-bouten aozora-bouten-shiromaru aozora-bouten-right\">ムレ</em></rt>"
        ), "html: {html}");
        assert_eq!(html.matches("ムレ").count(), 1);
        assert_eq!(serialize(&out), source);
    }

    #[test]
    fn angle_quote_keeps_nested_gaiji_and_forward_format() {
        let source = "≪前※［＃「特のへん＋廴＋聿」、第3水準1-87-71］l'oiseau royal［＃「l'oiseau royal」は斜体］後≫";
        let out = lex(source);
        let html = render_html(&out);
        assert!(html.contains("<span class=\"aozora-angle-quote\">《前"));
        assert!(
            html.contains(
                ">犍</span><i class=\"aozora-shatai\">l&#x27;oiseau royal</i>後》</span>"
            ),
            "html: {html}"
        );
        assert_eq!(html.matches("oiseau royal").count(), 1);
        assert_eq!(serialize(&out), source);
    }

    #[test]
    fn block_only_annotation_inside_reading_stays_an_inline_raw_marker() {
        let source = "漢《かん［＃改ページ］じ》";
        let out = lex(source);
        let html = render_html(&out);
        assert!(html.contains(
            "<rt>かん<span class=\"aozora-directive\" hidden>［＃改ページ］</span>じ</rt>"
        ));
        assert!(!html.contains("<rt>かん<div"));
        assert_eq!(serialize(&out), source);
    }

    #[test]
    fn explicit_ruby_base_does_not_cross_a_forced_break() {
        let source = "｜A［＃改行］B《r》";
        let out = lex(source);
        let html = render_html(&out);
        assert_eq!(
            html,
            "<p>｜A<br /><ruby>B<rp>(</rp><rt>r</rt><rp>)</rp></ruby></p>\n"
        );
        assert_eq!(serialize(&out), source);
    }

    #[test]
    fn structured_explicit_ruby_keeps_preceding_bars_visible() {
        for source in ["文｜｜A［＃ママ］《r》", "文｜｜A≪B≫C《r》"] {
            let out = lex(source);
            let html = render_html(&out);
            assert!(html.starts_with("<p>文｜<ruby>"), "html: {html}");
            let serialized = serialize(&out);
            assert_eq!(serialized, source);
            assert_eq!(serialize(&lex(&serialized)), serialized);
        }
    }

    #[test]
    fn forward_bouten_reclaims_resolved_gaiji() {
        let source = "※［＃「木＋吶のつくり」、第3水準1-85-54］［＃「枘」に傍点］";
        let out = lex(source);
        let html = render_html(&out);
        assert!(html.contains(
            "<em class=\"aozora-bouten aozora-bouten-goma aozora-bouten-right\"><span class=\"aozora-gaiji\" data-codepoint=\"U+6798\">枘</span></em>"
        ));
        assert_eq!(html.matches('枘').count(), 1);
        assert_eq!(serialize(&out), source);
        assert_eq!(serialize(&lex(&serialize(&out))), source);
    }

    #[test]
    fn forward_bouten_reclaims_unresolved_gaiji_hint() {
        let source = "※［＃「架空」、未知コード］［＃「架空」に傍点］";
        let out = lex(source);
        let html = render_html(&out);
        assert!(html.contains(
            "<em class=\"aozora-bouten aozora-bouten-goma aozora-bouten-right\"><span class=\"aozora-gaiji\" data-description=\"架空\">架空</span></em>"
        ));
        assert_eq!(html.matches(">架空</span>").count(), 1);
        assert_eq!(serialize(&out), source);
        assert_eq!(serialize(&lex(&serialize(&out))), source);
    }

    #[test]
    fn forward_bouten_reclaims_mixed_gaiji_and_text() {
        let source = "※［＃「木＋吶のつくり」、第3水準1-85-54］陀多［＃「枘陀多」に傍点］";
        let out = lex(source);
        let html = render_html(&out);
        assert!(html.contains(
            "<em class=\"aozora-bouten aozora-bouten-goma aozora-bouten-right\"><span class=\"aozora-gaiji\" data-codepoint=\"U+6798\">枘</span>陀多</em>"
        ));
        assert_eq!(html.matches('枘').count(), 1);
        assert_eq!(html.matches("陀多").count(), 1);
        assert_eq!(serialize(&out), source);
        assert_eq!(serialize(&lex(&serialize(&out))), source);
    }

    #[test]
    fn forward_bouten_reclaims_angle_quote() {
        let source = "≪A≫［＃「《A》」に傍点］";
        let out = lex(source);
        let html = render_html(&out);
        assert!(html.contains(
            "<em class=\"aozora-bouten aozora-bouten-goma aozora-bouten-right\"><span class=\"aozora-angle-quote\">《A》</span></em>"
        ));
        assert_eq!(html.matches("《A》").count(), 1);
        assert_eq!(serialize(&out), source);
        assert_eq!(serialize(&lex(&serialize(&out))), source);
    }

    #[test]
    fn forward_bouten_reclaims_kaeriten() {
        let source = "［＃レ］［＃「レ」に傍点］";
        let out = lex(source);
        let html = render_html(&out);
        assert!(html.contains(
            "<em class=\"aozora-bouten aozora-bouten-goma aozora-bouten-right\"><sup class=\"aozora-kaeriten\">レ</sup></em>"
        ));
        assert_eq!(serialize(&out), source);
        assert_eq!(serialize(&lex(&serialize(&out))), source);
    }

    #[test]
    fn left_ruby_keeps_gaiji_in_its_structured_base() {
        let source = "銅※［＃「金＋拔のつくり」、第3水準1-93-6］子［＃「銅※［＃「金＋拔のつくり」、第3水準1-93-6］子」の左に「どびょうし」のルビ］";
        let out = lex(source);
        let html = render_html(&out);
        assert!(html.contains("<ruby>銅<span class=\"aozora-gaiji\""));
        assert!(html.contains("子<rp>(</rp><rt class=\"aozora-ruby-left\">どびょうし</rt>"));
        assert!(!html.contains("※［＃"));
        assert_eq!(serialize(&out), source);
    }

    #[test]
    fn left_ruby_keeps_gaiji_in_its_structured_reading() {
        let source = "未［＃「未」の左に「※［＃「特のへん＋廴＋聿」、第3水準1-87-71］」のルビ］";
        let out = lex(source);
        let html = render_html(&out);
        assert!(html.contains("<rt class=\"aozora-ruby-left\"><span class=\"aozora-gaiji\""));
        assert!(html.contains(">犍</span></rt>"), "html: {html}");
        assert!(!html.contains("※［＃"));
        assert_eq!(serialize(&out), source);
    }

    #[test]
    fn side_note_keeps_gaiji_in_its_structured_base() {
        let source = "※［＃「てへん＋僉」、第3水準1-84-94］［＃「※［＃「てへん＋僉」、第3水準1-84-94］」の左に「アラタムル」の注記］";
        let out = lex(source);
        let html = render_html(&out);
        assert!(html.contains("<ruby><span class=\"aozora-gaiji\""));
        assert!(html.contains("<rt class=\"aozora-margin-note\">アラタムル</rt>"));
        assert!(!html.contains("※［＃"));
        assert_eq!(serialize(&out), source);
    }

    #[test]
    fn caption_figure_keeps_gaiji_caption_structured() {
        let source = "［＃「※［＃ローマ数字1、1-13-21］」のキャプション付きの図（fig.png）入る］";
        let out = lex(source);
        let html = render_html(&out);
        assert!(html.contains("<figure class=\"aozora-illustration\"><img src=\"fig.png\""));
        assert!(html.contains("<figcaption><span class=\"aozora-gaiji\""));
        assert!(!html.contains("※［＃"));
        let serialized = serialize(&out);
        assert_eq!(render_html(&lex(&serialized)), html);
    }

    #[test]
    fn ruby_base_illustration_uses_an_inline_image() {
        let source =
            "｜［＃底本が「オム」とルビを付した梵字（fig1317_17.png、横23×縦22）入る］《オム》";
        let out = lex(source);
        let html = render_html(&out);
        assert!(html.contains("<ruby><span class=\"aozora-illustration\"><img src=\"fig1317_17.png\" width=\"23\" height=\"22\""));
        assert!(!html.contains("<ruby><figure"));
        assert_eq!(serialize(&out), source);
    }

    #[test]
    fn ruby_base_margin_note_keeps_both_readings_once() {
        let source = "｜短尺［＃「尺」に「（冊）」の注記］《タンシヤク》";
        let out = lex(source);
        let html = render_html(&out);
        assert!(
            html.contains("<ruby>短<ruby>尺<rp>(</rp><rt class=\"aozora-margin-note\">（冊）</rt>")
        );
        assert_eq!(html.matches('尺').count(), 1);
        assert_eq!(html.matches("タンシヤク").count(), 1);
        let serialized = serialize(&out);
        assert_eq!(render_html(&lex(&serialized)), html);
        assert_eq!(serialize(&lex(&serialized)), serialized);
    }
}
