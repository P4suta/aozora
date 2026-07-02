# 0024. Canonical reference stylesheet for notation presentation

- Status: accepted
- Date: 2026-07-03
- Deciders: @P4suta
- Tags: render, playground, vscode, css

## Context

The HTML renderer emits only semantic class hooks (`<span
class="aozora-combine-upright">…`) and deliberately ships no CSS — presentation
is the consumer's job (ADR rationale in `arch/renderer.md`,
`notation/tcy.md`). In practice there is more than one consumer — the
playground (`playground/src/aozora.css`), the VS Code preview
(`editors/vscode/src/preview.ts`), and the `aozora` HTML export
(`editors/vscode/src/cliCommands.ts`) — and each hand-rolled its own copy of
the notation CSS.

Three hand-maintained copies drifted, silently and invisibly:

- **縦中横 (TCY) broke in vertical writing mode.** No copy set
  `text-combine-upright: all`, the one property that makes a
  `.aozora-combine-upright` span combine; `text-combine-upright` appeared zero
  times in the whole repository. A reader reported the bug; nothing caught it.
- The playground styled 傍点 under English descriptive names
  (`.aozora-bouten-white-sesame`, `-cross`, …) that the renderer never emits
  (the real slugs are the romaji `-shirogoma`, `-batsu`, …), so most 傍点
  styling was dead.
- The VS Code preview targeted `.aozora_gaiji` / `aozora_tcy` (underscores) —
  classes that do not exist — so gaiji highlighting and TCY were both dead.
- Most of the ~88 emitted classes had no rule in any consumer at all.

The class contract already has a single source of truth,
`crate::classes::AOZORA_CLASSES`, pinned to the emit sites by
`class_list_matches_emitted`. The presentation side had none.

## Decision

Ship a **canonical reference stylesheet**,
`crates/aozora-render/assets/aozora-notation.css`, as the single source of
truth for how every `aozora-*` class is presented. The renderer still emits
only class hooks and still injects no CSS into its HTML — this is a separate,
consumer-adoptable asset, not a change to renderer output.

- Every consumer (playground, VS Code preview, HTML export) adopts this sheet
  instead of hand-rolling `.aozora-*` rules. Consumers keep only their own
  layout/theme shell and may still override individual rules.
- Theming is via `--aozora-*` custom properties with self-contained fallbacks,
  so the sheet renders correctly stand-alone and a consumer bridges its theme
  by setting those variables. Two consumer-applied scope hooks —
  `.aozora-notation` (base) and `.aozora-vertical` (縦書き) — are the only
  non-emitted selectors the sheet carries.
- A test, `classes::canonical_stylesheet_matches_emitted_classes`, pins the
  sheet's `.aozora-*` selectors to `AOZORA_CLASSES` *exactly* (numeric variants
  normalised to their stem). A renamed class, a typo, or a forgotten style is
  a test failure, not a silent visual regression.

## Consequences

- The TCY vertical-mode bug and the whole *class* of name-drift bugs are fixed
  at the root: correct defaults live in one place, and the sync test makes
  future drift impossible to merge.
- Adopting the sheet across surfaces means one edit propagates everywhere;
  there is no longer a "fixed the playground, forgot the preview" gap.
- The renderer's "no CSS in the HTML, presentation deferred to the consumer"
  principle is preserved: the reference sheet is an optional asset, so other
  consumers (e.g. `afm`, which has its own pipeline) are unaffected.
- Cost: consumers import a file that lives in `crates/aozora-render/assets/`.
  The playground (Vite) and VS Code (esbuild) reach it across the workspace at
  build time; this is a build-time dependency edge, not a runtime one.

## Alternatives considered

- **Patch each consumer in place** (add `text-combine-upright` to the
  playground, fix the VS Code class names, leave three copies). Rejected: it
  fixes the symptom on the surfaces we happen to look at while leaving the
  drift mechanism — three uncoordinated copies with no shared authority — fully
  intact, so the next class rename silently breaks a consumer again.
- **Have the renderer inject inline styles / a `<style>` block.** Rejected for
  the reasons in `arch/renderer.md`: it forces one typographic model on every
  medium, trips strict CSP `style` regimes, and churns snapshot output.
- **Add real named fields / a richer taxonomy to the CST** so consumers derive
  presentation structurally. Rejected as disproportionate: the class hooks are
  already the right contract; only a shared stylesheet + a sync test were
  missing.

## References

- Plan: `~/.claude/plans/aozora-defer-defer-todo-repo-ancient-pillow.md` (P1).
- `crates/aozora-render/assets/aozora-notation.css` (the sheet).
- `crates/aozora-render/src/classes.rs` (`AOZORA_CLASSES`,
  `canonical_stylesheet_matches_emitted_classes`).
- [ADR-0022](0022-notation-hygiene-layer-roles.md),
  `crates/aozora-book/src/arch/renderer.md`,
  `crates/aozora-book/src/notation/tcy.md`.
