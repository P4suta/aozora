//! Black-box coverage of the public minimal-diff splice surface (#202),
//! exercised through the `aozora` front door exactly as an editor
//! integration would — proving the re-exports are usable from outside
//! the crate.

use aozora::{CoupledKind, Document, RegionRole, SpliceError, SpliceSafety};

#[test]
fn owned_regions_cover_the_whole_source() {
    let src = "序章\n｜青梅《おうめ》の実、青空［＃「青空」に傍点］。";
    let doc = Document::new(src);
    let tree = doc.parse();
    let verbatim = tree.to_source_verbatim();

    let regions = tree.owned_regions();
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
    let tree = doc.parse();

    let region = tree
        .owned_regions()
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
    let tree = doc.parse();

    let region = tree
        .owned_regions()
        .into_iter()
        .find(|r| r.role == RegionRole::ForwardReferenced)
        .expect("a referenced forward bouten");
    assert_eq!(
        region.safety,
        SpliceSafety::Coupled(CoupledKind::ForwardReference)
    );

    // An attribute-only change keeps the target, so the forward re-forms and
    // the edit is accepted.
    let spliced = tree
        .splice(region, "［＃「青空」に傍線］")
        .expect("attribute-only change is coherent");
    assert_eq!(spliced, "青空がひろがる、その［＃「青空」に傍線］");

    // A target-text change without touching the upstream literal would desync,
    // so it is declined as unverifiable (a coupled edit lands in a later #202
    // phase).
    let err = tree
        .splice(region, "［＃「海」に傍点］")
        .expect_err("target change desyncs the reference");
    assert!(matches!(err, SpliceError::Unverifiable { .. }));
    assert!(err.to_string().contains("could not be verified"));
}

#[test]
fn container_open_couples_to_its_close() {
    let src = "前\n［＃ここから2字下げ］\n本文\n［＃ここで字下げ終わり］\n後";
    let doc = Document::new(src);
    let tree = doc.parse();

    let open = tree
        .owned_regions()
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
