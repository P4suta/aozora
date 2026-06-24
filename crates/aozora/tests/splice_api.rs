//! Black-box coverage of the public minimal-diff splice surface (#202),
//! exercised through the `aozora` front door exactly as an editor
//! integration would — proving the re-exports are usable from outside
//! the crate.

use aozora::{Document, RegionRole, SpliceError, SpliceSafety};

#[test]
fn owned_regions_cover_the_whole_source() {
    let src = "序章\n｜青梅《おうめ》の実、青空［＃「青空」に傍点］。";
    let doc = Document::new(src);
    let tree = doc.parse();
    let verbatim = tree.to_source_verbatim();

    let regions = tree.owned_regions();
    // Complete, contiguous cover.
    assert_eq!(regions.first().unwrap().span.start, 0);
    assert_eq!(regions.last().unwrap().span.end as usize, verbatim.len(),);
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
fn reclaimed_forward_minimal_diff_splice() {
    let src = "青空［＃「青空」に傍点］の下を歩く";
    let doc = Document::new(src);
    let tree = doc.parse();

    let region = tree
        .owned_regions()
        .into_iter()
        .find(|r| r.role == RegionRole::ForwardReclaimed)
        .expect("a reclaimed forward bouten");
    assert_eq!(region.safety, SpliceSafety::Safe);

    let spliced = tree
        .splice_source(region, "海［＃「海」に傍点］")
        .expect("Safe region splices");
    assert_eq!(spliced, "海［＃「海」に傍点］の下を歩く");
}

#[test]
fn referenced_forward_declines_splice() {
    let src = "青空がひろがる、その［＃「青空」に傍点］";
    let doc = Document::new(src);
    let tree = doc.parse();

    let region = tree
        .owned_regions()
        .into_iter()
        .find(|r| r.role == RegionRole::ForwardReferenced)
        .expect("a referenced forward bouten");
    assert!(matches!(region.safety, SpliceSafety::Deferred(_)));

    let err = tree
        .splice_source(region, "x")
        .expect_err("Deferred region declines");
    assert!(matches!(err, SpliceError::Deferred { .. }));
    // The error renders a human-readable reason.
    assert!(err.to_string().contains("cannot be spliced"));
}
