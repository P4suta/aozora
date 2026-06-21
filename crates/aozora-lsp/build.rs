//! No-op build script.
//!
//! In the monorepo the crate version *is* the `aozora` parser version (a shared
//! workspace version), so `--version` reports `CARGO_PKG_VERSION` directly and
//! nothing needs to be baked in at build time. Kept as an empty `main` only
//! because the file is part of the package layout.

fn main() {}
