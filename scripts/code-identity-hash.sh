#!/usr/bin/env bash
# Content-addressed identity of the worktree at <ref>, with release-plz's
# version footprint — and ONLY that — neutralised, so a Release PR (a version
# bump + CHANGELOG, byte-identical code) hashes the same as the commit it was
# cut from. Everything else stays in the hash: a third-party dependency bump, a
# new line, any real edit changes it, so a mismatch always means the code
# differs and release-ready must run its heavy code gates in full. Reads git
# objects only (content-addressed → unforgeable), never the workflow payload.
#
# Neutralised (exactly release-plz's footprint under `dependencies_update =
# false`, nothing more):
#   * Cargo.toml   — the [workspace.package] `version` and the internal
#                    aozora / tree-sitter-aozora dependency version pins. Only
#                    the root manifest carries a literal version; crate
#                    manifests are `version.workspace = true` and untouched.
#   * Cargo.lock   — the `version` of every workspace-local package (a
#                    [[package]] block with no `source` line), i.e. the members
#                    release-plz bumps. Third-party locked versions stay.
#   * CHANGELOG.md — excluded entirely (release notes; no gate reads it).
set -euo pipefail
ref="${1:?usage: code-identity-hash.sh <git-ref>}"

# Root Cargo.toml: zero the workspace version line and the two internal pins.
# `^version = ` matches only the [workspace.package] line — `rust-version` and
# third-party `name = { version = … }` entries are left intact.
norm_toml() {
  git cat-file blob "${ref}:Cargo.toml" | sed -E '
    s/^version = "[^"]*"/version = "0"/
    s/^((aozora|aozora-[a-z]+|tree-sitter-aozora) = \{ version = )"[^"]*"/\1"0"/
  '
}

# Cargo.lock: zero the version of every [[package]] block that has no `source`
# (a workspace-local member). Deriving "local" from the absence of `source`
# needs no hard-coded member list, so a new member is covered automatically.
norm_lock() {
  git cat-file blob "${ref}:Cargo.lock" | awk '
    function flush() {
      if (block == "") return
      if (!has_source) sub(/\nversion = "[^"]*"/, "\nversion = \"0\"", block)
      printf "%s", block
      block = ""; has_source = 0
    }
    /^\[\[package\]\]/ { flush(); block = $0 "\n"; next }
    block == "" { print; next }
    { if ($0 ~ /^source = /) has_source = 1; block = block $0 "\n" }
    END { flush() }
  '
}

{
  # Every blob except the three version-carrying root files, verbatim:
  # mode + object id + path. `\t<name>$` matches only the root files, not a
  # crate manifest like `crates/aozora/Cargo.toml`.
  git ls-tree -r "$ref" | grep -vP '\t(CHANGELOG\.md|Cargo\.toml|Cargo\.lock)$'
  printf 'Cargo.toml\t%s\n' "$(norm_toml | sha256sum | cut -d' ' -f1)"
  printf 'Cargo.lock\t%s\n' "$(norm_lock | sha256sum | cut -d' ' -f1)"
} | LC_ALL=C sort | sha256sum | cut -d' ' -f1
