# Real-work release gate

`release-ready` compares the exact release artifacts with every accepted edition in the rights-filtered corpus before dispatching registry or GitHub publication.

## Immutable inputs

`.github/release-inputs.env` pins these repositories with full commit SHAs:

- `P4suta/aozora-rights-filtered-corpus` for UTF-8 sources and rights evidence.
- `P4suta/aozora-wasm-static-html-example` for the release lab and reusable workflow.

Release inputs exclude moving branches and package-registry `latest` versions.
The candidate bundle manifest binds the aozora commit, distribution archive hashes, adapter hashes, version, and wire schema.

## Release graph

The existing artifact jobs build npm/WebAssembly, crate, CLI, C binary interface, Extism, Python, and Go outputs.
`cargo run -p aozora-xtask -q -- real-work bundle` combines those exact artifacts into one Linux, macOS, or Windows bundle.
The pinned lab then runs four shards on each OS and compares all seven result projections byte-for-byte.
Linux additionally builds the no-script static site and compares every work in Chromium, Firefox, and WebKit at both widths.

The final `release-ready` fan-in requires the reusable real-work gate to succeed.
A crash, timeout, missing worker, mixed version, schema mismatch, output difference, new diagnostic, unapproved visual difference, or missing baseline prevents downstream publishing dispatches.

## First baseline

The first run refuses publication while `.github/real-work/diagnostics-baseline.json` and `.github/real-work/site/` remain absent.
Run `release-ready` manually to produce the commit-bound candidate and corpus artifacts.
Then dispatch `real-work baseline bootstrap` with the run ID and commit.

Bootstrap checks unsharded seven-engine parity across the entire corpus before writing a diagnostics candidate and static site.
It uploads `real-work-baseline-review-{commit}` without promoting the candidate.
Review the corpus digest, all edition entries, diagnostics, site, and artifact hashes before adding the two approved inputs in a dedicated PR.
The gate rejects wildcards, failed-job adoption, and automatic baseline updates.

Normal release runs never call the bootstrap command.
Later baseline changes require the exact affected edition list, before/after Merkle roots, a reason, and an issue in the lab's visual approval manifest.
