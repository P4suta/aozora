# Real-work release gate

`release-ready` verifies the exact release artifacts against every accepted edition in the rights-filtered corpus before any registry or GitHub release is dispatched.

## Immutable inputs

`.github/release-inputs.env` pins these repositories with full commit SHAs:

- `P4suta/aozora-rights-filtered-corpus` for UTF-8 sources and rights evidence;
- `P4suta/aozora-wasm-static-html-example` for the release lab and reusable workflow.

Moving branches and package-registry “latest” versions are not release inputs. The candidate bundle manifest also binds the aozora commit, distribution archive hashes, adapter hashes, version, and wire schema.

## Release graph

The existing artifact jobs build npm/WASM, crate, CLI, C ABI, Extism, Python, and Go outputs. `cargo xtask real-work bundle` combines those exact artifacts into one Linux, macOS, or Windows bundle. The pinned lab then runs four shards on each OS and compares all seven result projections byte-for-byte. Linux additionally builds the no-script static site and compares every work in Chromium, Firefox, and WebKit at both widths.

The final `release-ready` fan-in requires the reusable real-work gate to succeed. A crash, timeout, missing worker, mixed version, schema mismatch, output difference, new diagnostic, unapproved visual difference, or missing baseline therefore prevents the downstream publishing dispatches.

## First baseline

The first run is deliberately fail-closed because `.github/real-work/diagnostics-baseline.json` and `.github/real-work/site/` do not exist yet. Run `release-ready` manually to produce the commit-bound candidate and corpus artifacts, then dispatch `real-work baseline bootstrap` with that run ID and commit.

Bootstrap performs unsharded seven-engine parity across the entire corpus before writing a diagnostics candidate and static site. It uploads `real-work-baseline-review-{commit}` but never promotes it. Review the corpus digest, all edition entries, diagnostics, site, and artifact hashes before adding the two approved inputs in a dedicated PR. Wildcards, failed-job adoption, and automatic baseline updates are not supported.

Normal release runs never call the bootstrap command. Later baseline changes require the exact affected edition list, before/after Merkle roots, a reason, and an issue in the lab's visual approval manifest.
