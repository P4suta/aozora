//! Walks `AOZORA_CORPUS_ROOT` and verifies the source-region ownership
//! tiling and minimal-diff splice (#202) hold for every real document:
//!
//! * [`aozora::Tree::regions`] is a complete, gap-free, ordered,
//!   non-overlapping cover of the verbatim (sanitized) source — the
//!   region byte-slices concatenate back to it exactly.
//! * No region is unclassified (`Opaque`) — every real construct is editable.
//! * The **identity splice** reproduces the verbatim source. `check_tiling`
//!   already proves the byte-level tiling for *every* region; this additionally
//!   runs the real `Tree::splice` machinery (a `Direct` byte replacement, or a
//!   `Coupled` partner-derivation + scoped verification) on a representative
//!   sample — one region per safety class per document. The splice itself is
//!   already `O(marker)` (it verifies in a scoped context, never re-parsing the
//!   whole document); sampling bounds the *number* of splice calls so the
//!   full-corpus sweep stays well under a minute rather than calling `splice`
//!   for every one of the corpus's millions of coupled regions.
//!
//! This extends the property-test coverage in `aozora-cst`'s lossless
//! invariant to the full 青空文庫 corpus. Skipped silently when
//! `AOZORA_CORPUS_ROOT` is unset; never hard-fails on missing corpus.

use aozora::encoding::decode_auto;
use aozora::{CoupledKind, Document, Region, RegionRole, SpliceSafety};

/// Cap the collected failures so a systemic regression does not produce
/// a multi-megabyte assertion message.
const MAX_REPORTED: usize = 50;

#[test]
fn corpus_regions_tile_the_source() {
    let Some(source) = aozora_corpus::from_env() else {
        eprintln!("AOZORA_CORPUS_ROOT not set; skipping splice tiling sweep");
        return;
    };

    let mut count: usize = 0;
    let mut failures: Vec<String> = Vec::new();

    for item in source.iter() {
        let item = item.expect("corpus iteration must not error");
        let Ok(utf8) = decode_auto(&item.bytes) else {
            eprintln!("skip (neither UTF-8 nor Shift_JIS): {}", item.label);
            continue;
        };

        let doc = Document::new(utf8);
        let tree = doc.parse();
        let verbatim = tree.to_source_verbatim();
        let regions = tree.regions();

        if let Err(why) = check_tiling(&verbatim, &regions) {
            failures.push(format!("{}: {why}", item.label));
        } else if let Err(why) = check_splice_sampled(&tree, &verbatim, &regions) {
            failures.push(format!("{}: {why}", item.label));
        }

        count += 1;
        if failures.len() >= MAX_REPORTED {
            break;
        }
    }

    eprintln!("splice tiling sweep: {count} docs walked");
    assert!(
        failures.is_empty(),
        "{} tiling/splice failure(s):\n  {}",
        failures.len(),
        failures.join("\n  "),
    );
    eprintln!("splice tiling sweep: owned-region tiling + identity splice hold");
}

/// Verify the regions form a complete, ordered, gap-free, non-overlapping
/// cover whose byte-slices concatenate back to `verbatim`.
fn check_tiling(verbatim: &str, regions: &[Region]) -> Result<(), String> {
    if verbatim.is_empty() {
        return if regions.is_empty() {
            Ok(())
        } else {
            Err("empty source yielded regions".to_owned())
        };
    }
    let Some(first) = regions.first() else {
        return Err("non-empty source yielded no regions".to_owned());
    };
    if first.span.start != 0 {
        return Err(format!("tiling starts at {} not 0", first.span.start));
    }
    let end = regions[regions.len() - 1].span.end as usize;
    if end != verbatim.len() {
        return Err(format!("tiling ends at {end} not {}", verbatim.len()));
    }
    for pair in regions.windows(2) {
        if pair[0].span.end != pair[1].span.start {
            return Err(format!(
                "gap/overlap between {}..{} and {}..{}",
                pair[0].span.start, pair[0].span.end, pair[1].span.start, pair[1].span.end,
            ));
        }
    }
    let rebuilt: String = regions
        .iter()
        .map(|r| &verbatim[r.span.start as usize..r.span.end as usize])
        .collect();
    if rebuilt != verbatim {
        return Err("region concatenation != verbatim source".to_owned());
    }
    Ok(())
}

/// One representative region per safety class — containers split into open vs
/// close, the two marker shapes the edit path handles differently.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Sample {
    Direct,
    ContainerOpen,
    ContainerClose,
    Forward,
    HeadingHint,
    MarginNote,
}

/// The sample bucket for a region, or `None` for ones covered structurally
/// elsewhere (e.g. a leaf container).
fn sample_of(r: &Region) -> Option<Sample> {
    match (r.safety, r.role) {
        (SpliceSafety::Direct, _) => Some(Sample::Direct),
        (SpliceSafety::Coupled(CoupledKind::Container), RegionRole::ContainerOpen) => {
            Some(Sample::ContainerOpen)
        }
        (SpliceSafety::Coupled(CoupledKind::Container), RegionRole::ContainerClose) => {
            Some(Sample::ContainerClose)
        }
        (SpliceSafety::Coupled(CoupledKind::ForwardReference), _) => Some(Sample::Forward),
        (SpliceSafety::Coupled(CoupledKind::HeadingHint), _) => Some(Sample::HeadingHint),
        (SpliceSafety::Coupled(CoupledKind::MarginNote), _) => Some(Sample::MarginNote),
        _ => None,
    }
}

/// No region is `Opaque` (checked for *every* region), and the identity splice
/// of one representative region per safety class reproduces the verbatim source
/// through the real `Tree::splice` machinery. `check_tiling` already proves the
/// byte-level tiling for all regions; this proves the *machinery* is sound on
/// real constructs while bounding the number of `splice` calls (≤ 6 per doc).
fn check_splice_sampled(
    tree: &aozora::Tree<'_>,
    verbatim: &str,
    regions: &[Region],
) -> Result<(), String> {
    let mut sampled: Vec<Sample> = Vec::with_capacity(6);
    for r in regions {
        if r.safety == SpliceSafety::Opaque {
            return Err(format!("region {:?} classified Opaque", r.role));
        }
        let Some(bucket) = sample_of(r) else { continue };
        if sampled.contains(&bucket) {
            continue;
        }
        sampled.push(bucket);

        let same = &verbatim[r.span.start as usize..r.span.end as usize];
        match tree.splice(*r, same) {
            Ok(out) if out == verbatim => {}
            Ok(_) => return Err(format!("identity splice of {:?} changed bytes", r.role)),
            Err(e) => return Err(format!("identity splice of {:?} failed: {e}", r.role)),
        }
    }
    Ok(())
}
