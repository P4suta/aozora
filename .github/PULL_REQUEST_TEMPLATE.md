## Summary

<!-- One or two sentences: what this PR changes and why. -->

## Type of change

- [ ] Bug fix
- [ ] New feature (new Aozora notation, CLI flag, API surface, …)
- [ ] Refactor (no behaviour change)
- [ ] Documentation / book / ADR
- [ ] CI / developer tooling
- [ ] Upstream comrak touch-up (requires an approving ADR — see ADR-0001)

## Checklist

- [ ] `just ci` passes locally (lint + build + test + spec-* + coverage
      + upstream-diff + book-build).
- [ ] Added or updated tests that exercise the change.
- [ ] Updated `CHANGELOG.md` under `[Unreleased]` (or stated why it
      doesn't need a changelog entry).
- [ ] Commit messages follow Conventional Commits (lefthook enforces).
- [ ] If this touches `upstream/comrak/`: linked the approving ADR and
      confirmed the diff is still within the 200-line budget
      (`just upstream-diff`).
- [ ] If this adds a new Aozora notation: followed the 10-step TDD flow
      in `CLAUDE.md` (spec fixture → AST variant → lexer test (red) →
      lexer impl (green) → post_process splice → renderer → serializer
      → CSS themes → cross-layer invariants → verify).
- [ ] If this adds a renderer-emitted class or HTML shape: updated
      `tests/css_class_contract.rs` and both afm-book themes
      (`afm-horizontal.css` / `afm-vertical.css`).

## Related

<!-- Closes #N / part of #M / cross-reference to ADR-NNNN, etc. -->
