//! Walk the parsed AST and report how often each `AozoraNode` variant appears,
//! plus the number of lexer diagnostics for the input.
//!
//! Run:
//!
//!     cargo run --example ast-walk -p afm-parser -- input.md

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::process::ExitCode;

use afm_parser::{Options, parse};
use afm_syntax::AozoraNode;
use comrak::Arena;
use comrak::nodes::{AstNode, NodeValue};

fn main() -> ExitCode {
    let Some(path) = env::args().nth(1) else {
        eprintln!("usage: ast-walk <path/to/input.md>");
        return ExitCode::from(2);
    };

    let input = match fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("failed to read {path}: {e}");
            return ExitCode::FAILURE;
        }
    };

    let arena = Arena::new();
    let options = Options::afm_default();
    let parsed = parse(&arena, &input, &options);

    let mut counts: BTreeMap<&'static str, usize> = BTreeMap::new();
    walk(parsed.root, &mut counts);

    let width = counts
        .values()
        .copied()
        .max()
        .unwrap_or(0)
        .to_string()
        .len()
        .max(1);
    for (kind, n) in &counts {
        println!("{n:>width$}  {kind}");
    }
    let diag_count = parsed.diagnostics.len();
    println!("{diag_count:>width$}  lexer diagnostics");
    ExitCode::SUCCESS
}

fn walk<'a>(node: &'a AstNode<'a>, counts: &mut BTreeMap<&'static str, usize>) {
    if let NodeValue::Aozora(az) = &node.data.borrow().value {
        // `az` is `&Box<AozoraNode>`; deref to match on the enum.
        // AozoraNode is `#[non_exhaustive]`, so the wildcard arm keeps
        // this example compiling across future variant additions.
        let kind = match &**az {
            AozoraNode::Ruby(_) => "Ruby",
            AozoraNode::Bouten(_) => "Bouten",
            AozoraNode::TateChuYoko(_) => "TateChuYoko",
            AozoraNode::Gaiji(_) => "Gaiji",
            AozoraNode::Annotation(_) => "Annotation",
            AozoraNode::Indent(_) => "Indent",
            AozoraNode::PageBreak => "PageBreak",
            _ => "Other",
        };
        *counts.entry(kind).or_insert(0) += 1;
    }

    for child in node.children() {
        walk(child, counts);
    }
}
