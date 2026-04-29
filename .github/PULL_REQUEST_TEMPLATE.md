## Summary

<!-- One or two sentences: what this PR changes and why. -->

## Type of change

- [ ] Bug fix
- [ ] New feature (new 青空文庫 notation, CLI flag, API surface, …)
- [ ] Refactor (no behaviour change)
- [ ] Documentation
- [ ] CI / developer tooling

## Checklist

- [ ] `just ci` passes locally (lint + build + test + property tests +
      `cargo deny` + coverage).
- [ ] Added or updated tests that exercise the change.
- [ ] Commit messages follow Conventional Commits (the `commit-msg`
      hook enforces this).
- [ ] If this adds a new 青空文庫 notation: followed the TDD flow in
      [`CONTRIBUTING.md`](../CONTRIBUTING.md) (spec fixture → AST
      variant → lexer test (red) → lexer impl (green) → renderer →
      serializer → cross-layer invariants).

## Related

<!-- Closes #N / part of #M / cross-references, etc. -->
