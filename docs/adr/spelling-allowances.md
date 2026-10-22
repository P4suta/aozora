---
date: 2026-10-23
---

# Spelling allowances

## Status

Proposed.

## Context

The spelling checker scans source, documentation, and dependency metadata.
Some valid names and preserved source text resemble dictionary mistakes.

## Decision

Keep the explicit allowances in [_typos.toml](../../_typos.toml).
Git hashes can contain letter sequences that resemble misspellings.
`afm` names the sibling repository.
`bre` appears in the French accent-decomposition fragment ``fune`bre``.
The listed personal names occur in notices and documentation.
The Hepburn morae `gya`, `myu`, and `ba` in render-slug tests collide with dictionary entries.

Exclude corpus fixtures because their source spellings must remain verbatim.
Exclude vendored character tables because their hexadecimal spans resemble misspellings.
Exclude build, coverage, and mutation outputs because they contain generated text.
Exclude rendered HTML and lockfiles because they contain generated content.

## Consequences

The spelling gate can check maintained prose without rewriting source material or valid names.
Explicit words and paths keep the exceptions reviewable.
Removing these allowances would require changing valid spellings or generated output.
Disabling spelling checks would hide unrelated mistakes.
