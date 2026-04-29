//! Data-driven probe for the gaiji-resolution path.
//!
//! Walks every `※[#…]` entry in a sibling-repo gaiji sample file
//! and reports which lookup tier (existing / mencode-to-str /
//! mencode-to-char / U+xxxx / description) resolved it (or that no
//! tier hit). Plus runs a fixed-list smoke probe of entries the user
//! flagged as failing. Output is plain text — feed to `awk` / `sort`
//! / etc. for further slicing.
//!
//! Run with:
//! ```text
//! cargo run -p aozora-encoding --example probe_gaiji
//! ```

use aozora_encoding::gaiji::{Resolved, lookup, table_sizes};

fn main() {
    let (single, combo, desc) = table_sizes();
    eprintln!("=== gaiji table sizes ===");
    eprintln!("  mencode → char (single):  {single}");
    eprintln!("  mencode → str  (combo):   {combo}");
    eprintln!("  description → char:        {desc}");
    eprintln!();

    let failing = [
        // The three plane-2 entries the user flagged as not resolving:
        ("丂", Some("第4水準2-16-1")),
        ("畺", Some("第4水準2-45-30")),
        ("龔", Some("第4水準2-77-60")),
        // Plane-1 sanity (known to resolve):
        ("木＋吶のつくり", Some("第3水準1-85-54")),
        // Plane-2 first-cell sanity:
        ("人の異体", Some("第4水準2-1-1")),
        // Description fallback (no mencode):
        ("木＋吶のつくり", None),
        // Canonical chuki naming: 鄧 = 登 + おおざと radical (阝).
        // Literal "邦＋登" is NOT in the dict — the convention is
        // <left-component> + <right-radical-name>. Both forms
        // probed below to make the difference obvious in the
        // output.
        ("登＋おおざと", None),
        ("邦＋登", None),
    ];

    eprintln!("=== flagged-failing probe ===");
    for (desc, men) in failing {
        let r = lookup(None, men, desc);
        match r {
            Some(Resolved::Char(c)) => eprintln!(
                "  desc={desc:?} men={men:?} → CHAR '{c}' (U+{:04X})",
                c as u32
            ),
            Some(Resolved::Multi(s)) => eprintln!("  desc={desc:?} men={men:?} → MULTI {s:?}"),
            None => eprintln!("  desc={desc:?} men={men:?} → NONE"),
        }
    }

    // Also: try the raw mencode strings WITHOUT the description, so
    // we can isolate whether the description channel or the mencode
    // channel is the failing tier.
    eprintln!();
    eprintln!("=== mencode-only probe (description='dummy') ===");
    for (_, men) in failing {
        let Some(m) = men else { continue };
        let r = lookup(None, Some(m), "dummy");
        eprintln!("  men={m:?} → {r:?}");
    }

    // And: try the raw description WITHOUT mencode, so we know if
    // the description-table path covers these characters.
    eprintln!();
    eprintln!("=== description-only probe (mencode=None) ===");
    for (desc, _) in failing {
        let r = lookup(None, None, desc);
        eprintln!("  desc={desc:?} → {r:?}");
    }
}
