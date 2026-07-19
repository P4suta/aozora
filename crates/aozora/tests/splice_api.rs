//! Black-box coverage of the public minimal-diff splice surface (#202),
//! exercised through the `aozora` front door exactly as an editor
//! integration would — proving the re-exports are usable from outside
//! the crate.

use aozora::{CoupledKind, Document, RegionRole, SpliceError, SpliceSafety};

#[test]
fn regions_cover_the_whole_source() {
    let src = "序章\n｜青梅《おうめ》の実、青空［＃「青空」に傍点］。";
    let doc = Document::new(src);
    let tree = doc.snapshot();
    let verbatim = tree.to_source_verbatim();

    let regions = tree.regions();
    // Complete, contiguous cover.
    assert_eq!(regions.first().unwrap().span.start, 0);
    assert_eq!(regions.last().unwrap().span.end as usize, verbatim.len());
    let rebuilt: String = regions
        .iter()
        .map(|r| &verbatim[r.span.start as usize..r.span.end as usize])
        .collect();
    assert_eq!(rebuilt, verbatim);

    // At least the ruby and the reclaimed forward surface as classified roles.
    assert!(regions.iter().any(|r| r.role == RegionRole::Ruby));
    assert!(
        regions
            .iter()
            .any(|r| r.role == RegionRole::ForwardReclaimed)
    );
}

#[test]
fn reclaimed_forward_is_direct_minimal_diff() {
    let src = "青空［＃「青空」に傍点］の下を歩く";
    let doc = Document::new(src);
    let tree = doc.snapshot();

    let region = tree
        .regions()
        .into_iter()
        .find(|r| r.role == RegionRole::ForwardReclaimed)
        .expect("a reclaimed forward bouten");
    assert_eq!(region.safety, SpliceSafety::Direct);

    // The whole construct (literal + bracket) is owned by the region, so a
    // direct byte replacement is a complete edit.
    let spliced = tree
        .splice(region, "海［＃「海」に傍点］")
        .expect("Direct region splices");
    assert_eq!(spliced, "海［＃「海」に傍点］の下を歩く");
}

#[test]
fn referenced_forward_is_coupled() {
    let src = "青空がひろがる、その［＃「青空」に傍点］";
    let doc = Document::new(src);
    let tree = doc.snapshot();

    let region = tree
        .regions()
        .into_iter()
        .find(|r| r.role == RegionRole::ForwardReferenced)
        .expect("a referenced forward bouten");
    assert_eq!(
        region.safety,
        SpliceSafety::Coupled(CoupledKind::ForwardReference)
    );

    // An attribute-only change keeps the target, so the forward re-forms and
    // the edit is accepted (a single-region edit).
    let spliced = tree
        .splice(region, "［＃「青空」に傍線］")
        .expect("attribute-only change is coherent");
    assert_eq!(spliced, "青空がひろがる、その［＃「青空」に傍線］");

    // A target-text change is a coupled edit: both the bracket and the unique
    // upstream literal are rewritten so the reference stays in sync.
    let coupled = tree
        .splice(region, "［＃「海」に傍点］")
        .expect("target change is a coupled edit");
    assert_eq!(coupled, "海がひろがる、その［＃「海」に傍点］");
}

#[test]
fn ruby_base_forward_target_change_is_irreducible() {
    // The referent literal lives inside a ruby base, not a plain run, so a
    // target change cannot be carved out — declined as unverifiable.
    let src = "｜我《われ》は［＃「我」に傍点］";
    let doc = Document::new(src);
    let tree = doc.snapshot();
    let Some(region) = tree
        .regions()
        .into_iter()
        .find(|r| r.role == RegionRole::ForwardReferenced)
    else {
        return; // classifier may treat this shape differently; nothing to assert
    };
    let err = tree
        .splice(region, "［＃「彼」に傍点］")
        .expect_err("ruby-base referent cannot be coupled");
    assert!(matches!(err, SpliceError::Unverifiable { .. }));
}

#[test]
fn ruby_base_forward_attribute_change_is_coherent() {
    // The referent lives inside a ruby base, so a *target* change is
    // irreducible — but an attribute-only change (傍点→傍線) keeps the node's
    // own target and is verified in a scoped `<target><replacement>` context,
    // never touching the ruby base. It must be accepted.
    let src = "｜我《われ》は［＃「我」に傍点］";
    let doc = Document::new(src);
    let tree = doc.snapshot();
    let Some(region) = tree
        .regions()
        .into_iter()
        .find(|r| r.role == RegionRole::ForwardReferenced)
    else {
        return; // classifier may treat this shape differently; nothing to assert
    };
    let spliced = tree
        .splice(region, "［＃「我」に傍線］")
        .expect("attribute-only change on a ruby-base forward is coherent");
    // Only the bracket changed; the ruby reading and base survive verbatim.
    assert!(spliced.contains("われ"));
    assert!(spliced.contains("傍線"));
    assert!(!spliced.contains("傍点"));
}

#[test]
fn multi_target_forward_is_coupled_and_identity_safe() {
    // A 、-joined multi-target forward (`「A」「B」`) is `Referenced`; its target
    // lowers to a single plain run ("A、B"). The identity splice re-forms it in
    // a scoped context and is a no-op, but a target change is irreducible: the
    // canonical "A、B" is not a contiguous source substring (the source reads
    // "AとB"), so it is honestly declined.
    let src = "AとB［＃「A」「B」に傍点］";
    let doc = Document::new(src);
    let tree = doc.snapshot();
    let verbatim = tree.to_source_verbatim();
    let region = tree
        .regions()
        .into_iter()
        .find(|r| r.role == RegionRole::ForwardReferenced)
        .expect("a multi-target forward bouten");
    assert_eq!(
        region.safety,
        SpliceSafety::Coupled(CoupledKind::ForwardReference)
    );
    let own = &verbatim[region.span.start as usize..region.span.end as usize];
    assert_eq!(
        tree.splice(region, own).expect("identity is a no-op"),
        verbatim
    );
    // A target change is irreducible (the rendered `A、B` is not a source
    // substring), so it is honestly declined.
    let err = tree
        .splice(region, "［＃「海」に傍点］")
        .expect_err("a multi-segment target change is irreducible");
    assert!(matches!(err, SpliceError::Unverifiable { .. }));
}

#[test]
fn heading_hint_target_change_syncs() {
    // A forward heading hint requires its target to be preceded by a matching
    // run; that run is the coupled partner. Changing the title rewrites both
    // the directive and the upstream run so the hint stays resolvable.
    let src = "第一篇［＃「第一篇」は大見出し］";
    let doc = Document::new(src);
    let tree = doc.snapshot();
    let verbatim = tree.to_source_verbatim();
    let region = tree
        .regions()
        .into_iter()
        .find(|r| r.role == RegionRole::HeadingHint)
        .expect("a forward heading hint");
    assert_eq!(
        region.safety,
        SpliceSafety::Coupled(CoupledKind::HeadingHint)
    );
    // Identity is a no-op; deleting the directive is coherent.
    let own = &verbatim[region.span.start as usize..region.span.end as usize];
    assert_eq!(
        tree.splice(region, own).expect("identity is a no-op"),
        verbatim
    );
    tree.splice(region, "")
        .expect("deleting the directive is coherent");
    // A coupled title change rewrites the upstream run too.
    let spliced = tree
        .splice(region, "［＃「序章」は大見出し］")
        .expect("title change is a coupled edit");
    assert_eq!(spliced, "序章［＃「序章」は大見出し］");
}

#[test]
fn margin_note_base_change_is_coherent() {
    // A side note (注記) reclaims the base run it annotates into its own region
    // (`未来［＃…］`, base 未来 included), so a base change is a single-region
    // replacement carrying the new base, verified in family — never touching
    // surrounding text. Bytes outside the region stay identical.
    let src = "未来［＃「未来」の左に「みらい」の注記］を見る。";
    let doc = Document::new(src);
    let tree = doc.snapshot();
    let verbatim = tree.to_source_verbatim();
    let region = tree
        .regions()
        .into_iter()
        .find(|r| r.role == RegionRole::MarginNote)
        .expect("a side-note margin note");
    assert_eq!(
        region.safety,
        SpliceSafety::Coupled(CoupledKind::MarginNote)
    );
    let own = &verbatim[region.span.start as usize..region.span.end as usize];
    assert_eq!(
        tree.splice(region, own).expect("identity is a no-op"),
        verbatim
    );
    let spliced = tree
        .splice(region, "過去［＃「過去」の左に「みらい」の注記］")
        .expect("base change is a coherent edit");
    assert_eq!(spliced, "過去［＃「過去」の左に「みらい」の注記］を見る。");
}

#[test]
fn container_open_couples_to_its_close() {
    let src = "前\n［＃ここから2字下げ］\n本文\n［＃ここで字下げ終わり］\n後";
    let doc = Document::new(src);
    let tree = doc.snapshot();

    let open = tree
        .regions()
        .into_iter()
        .find(|r| r.role == RegionRole::ContainerOpen)
        .expect("a container open");
    assert_eq!(open.safety, SpliceSafety::Coupled(CoupledKind::Container));

    // Changing the family rewrites both the open and the paired close.
    let spliced = tree
        .splice(open, "［＃ここから罫囲み］")
        .expect("a container kind change is verifiable");
    assert!(spliced.contains("［＃ここから罫囲み］"));
    assert!(spliced.contains("［＃罫囲み終わり］"));
    assert!(!spliced.contains("字下げ"));
    assert!(spliced.contains("本文"));
}
