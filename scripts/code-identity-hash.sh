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

# Fail — emitting NOTHING — if the ref or the files it must read are absent, so
# an unresolvable ref can never print a plausible, comparable hash that a caller
# might mistake for a real one.
git rev-parse --verify --quiet "${ref}^{tree}" >/dev/null \
  || { echo "code-identity-hash: cannot resolve ${ref}" >&2; exit 1; }
for f in Cargo.toml Cargo.lock; do
  git cat-file -e "${ref}:${f}" 2>/dev/null \
    || { echo "code-identity-hash: ${ref}:${f} is missing" >&2; exit 1; }
done

# Root Cargo.toml, section-aware so ONLY release-plz's footprint is zeroed: the
# [workspace.package] `version`, and internal dependency pins in
# [workspace.dependencies] — an inline entry carrying both a `version` and a
# `path = "crates/…"`, i.e. a workspace member identified by path, not by name.
# Every other `version = ` line (a `[workspace.dependencies.foo]` table-form
# third-party pin, `rust-version`, a crate-manifest version) stays in the hash.
norm_toml() {
  git cat-file blob "${ref}:Cargo.toml" | awk '
    /^\[/ { section = $0 }
    section == "[workspace.package]" && /^version = "/ {
      sub(/"[^"]*"/, "\"0\""); print; next
    }
    section == "[workspace.dependencies]" && /version = "/ && /path = "crates\// {
      sub(/version = "[^"]*"/, "version = \"0\""); print; next
    }
    { print }
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
