//! Walks `AOZORA_CORPUS_ROOT` and verifies the source-region ownership
//! tiling (#202) holds for every real document:
//!
//! * [`aozora::Tree::owned_regions`] is a complete, gap-free, ordered,
//!   non-overlapping cover of the verbatim (sanitized) source — the
//!   region byte-slices concatenate back to it exactly.
//! * Replacing any self-contained region's bytes with themselves (an
//!   identity splice) reproduces the verbatim source, and a split /
//!   paired region declines the splice.
//!
//! This extends the property-test coverage in `aozora-cst`'s lossless
//! invariant to the full 青空文庫 corpus. Skipped silently when
//! `AOZORA_CORPUS_ROOT` is unset; never hard-fails on missing corpus.

use aozora::{Document, OwnedRegion, SpliceSafety};
use aozora_encoding::decode_auto;

/// Cap the collected failures so a systemic regression does not produce
/// a multi-megabyte assertion message.
const MAX_REPORTED: usize = 50;

#[test]
fn corpus_owned_regions_tile_the_source() {
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
        let regions = tree.owned_regions();

        if let Err(why) = check_tiling(&verbatim, &regions) {
            failures.push(format!("{}: {why}", item.label));
        } else if let Err(why) = check_identity_splice(&tree, &verbatim, &regions) {
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
fn check_tiling(verbatim: &str, regions: &[OwnedRegion]) -> Result<(), String> {
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

/// The identity splice of every `Safe` region must reproduce the verbatim
/// source; a `Deferred` region must decline.
fn check_identity_splice(
    tree: &aozora::Tree<'_>,
    verbatim: &str,
    regions: &[OwnedRegion],
) -> Result<(), String> {
    for r in regions {
        let same = &verbatim[r.span.start as usize..r.span.end as usize];
        let result = tree.splice_source(*r, same);
        match r.safety {
            SpliceSafety::Safe => match result {
                Ok(out) if out == verbatim => {}
                Ok(_) => {
                    return Err(format!(
                        "identity splice of Safe {:?} changed bytes",
                        r.role
                    ));
                }
                Err(e) => return Err(format!("Safe {:?} declined splice: {e}", r.role)),
            },
            SpliceSafety::Deferred(_) if result.is_ok() => {
                return Err(format!("Deferred {:?} accepted splice", r.role));
            }
            // Deferred + declined (the expected case), or — since
            // `SpliceSafety` is `#[non_exhaustive]` — a future variant.
            _ => {}
        }
    }
    Ok(())
}
