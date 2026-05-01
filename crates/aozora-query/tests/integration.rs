//! End-to-end query tests: parse a real document, compile a real
//! DSL string, assert the captures cover what we expect.

use aozora::Document;
use aozora::pipeline::lexer::sanitize;
use aozora_cst::build_cst;
use aozora_query::compile;

fn cst_for(src: &str) -> aozora_cst::SyntaxNode {
    let sanitized = sanitize(src);
    let doc = Document::new(src);
    let tree = doc.parse();
    build_cst(&sanitized.text, tree.source_nodes())
}

#[test]
fn captures_every_construct_in_a_document() {
    let cst = cst_for("｜青梅《おうめ》と｜青空《あおぞら》");
    let q = compile("(Construct @c)").expect("compile");
    let caps = q.captures(&cst);
    assert_eq!(caps.len(), 2, "expected 2 ruby Constructs, got {caps:?}");
    for c in &caps {
        assert_eq!(c.name, "c");
        let node = c.node.as_node().expect("Construct is a branch");
        assert_eq!(node.kind(), aozora_cst::SyntaxKind::Construct);
    }
}

#[test]
fn captures_every_container_open_close() {
    let src = "［＃ここから2字下げ］\n本文\n［＃ここで字下げ終わり］";
    let cst = cst_for(src);
    let q = compile("(ContainerOpen @open)\n(ContainerClose @close)").expect("compile");
    let caps = q.captures(&cst);
    let opens = caps.iter().filter(|c| c.name == "open").count();
    let closes = caps.iter().filter(|c| c.name == "close").count();
    assert_eq!(opens, 1);
    assert_eq!(closes, 1);
}

#[test]
fn wildcard_matches_every_node() {
    // Plain text input — only Document + Plain nodes (Plain is a
    // token, not a node — `descendants()` walks nodes only). So
    // the wildcard captures only Document.
    let cst = cst_for("hello");
    let q = compile("(_ @any)").expect("compile");
    let caps = q.captures(&cst);
    assert!(!caps.is_empty(), "wildcard captured nothing");
    assert!(
        caps.iter().any(|c| c
            .node
            .as_node()
            .is_some_and(|n| n.kind() == aozora_cst::SyntaxKind::Document)),
        "Document root not captured"
    );
}

#[test]
fn captures_have_no_name_without_at_clause() {
    let cst = cst_for("｜青梅《おうめ》");
    let q = compile("(Construct)").expect("compile");
    let caps = q.captures(&cst);
    assert_eq!(caps.len(), 1);
    assert_eq!(caps[0].name, "");
}

#[test]
fn empty_query_yields_no_captures() {
    let cst = cst_for("｜青梅《おうめ》");
    let q = compile("").expect("compile empty");
    assert!(q.captures(&cst).is_empty());
}
