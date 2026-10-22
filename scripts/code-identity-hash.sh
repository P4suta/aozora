#!/usr/bin/env bash
# Release-only version bumps reuse code-gate evidence when code is unchanged (ADR-0052).
set -euo pipefail
ref="${1:?usage: code-identity-hash.sh <git-ref>}"

git rev-parse --verify --quiet "${ref}^{tree}" >/dev/null \
  || { echo "code-identity-hash: cannot resolve ${ref}" >&2; exit 1; }
for f in Cargo.toml Cargo.lock; do
  git cat-file -e "${ref}:${f}" 2>/dev/null \
    || { echo "code-identity-hash: ${ref}:${f} is missing" >&2; exit 1; }
done

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
  git ls-tree -r "$ref" | grep -vP '\t(CHANGELOG\.md|Cargo\.toml|Cargo\.lock)$'
  printf 'Cargo.toml\t%s\n' "$(norm_toml | sha256sum | cut -d' ' -f1)"
  printf 'Cargo.lock\t%s\n' "$(norm_lock | sha256sum | cut -d' ' -f1)"
} | LC_ALL=C sort | sha256sum | cut -d' ' -f1
