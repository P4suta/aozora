# 0006. Lint profile policy and scope discipline

- Status: accepted
- Date: 2026-04-23
- Tags: tooling, lint, workspace, ci

## Context

By the end of M1 Phase C the workspace had ~5 crates plus one vendored
fork (`upstream/comrak/`) totalling ~10 kLOC of our own code. Lint
configuration had grown organically: `pedantic` + `nursery` + `cargo`
groups in `[workspace.lints.clippy]`, a minimal `rustfmt.toml`, a
`strict-code` grep gate, `typos`, and `cargo deny`. It worked, but the
scope of each layer was implicit and fragile.

Two concrete failure modes surfaced once we tried to raise the bar in
Phase Q:

1. **Lint CLI flags vs `[workspace.lints]` priority.** `just clippy`
   passed `-W clippy::pedantic -W clippy::nursery` on the command line,
   which re-enabled the whole groups at CLI priority and silently
   overrode per-lint carve-outs (e.g. `redundant_pub_crate = "allow"`).
   The symptom looked like a contradictory ruleset, but the root cause
   was two sources of truth.
2. **Cargo forbids mixing `workspace = true` with per-crate `[lints.*]`
   sections** (rust-lang/cargo#12697). So there is no ergonomic way to
   say "inherit workspace lints, but allow `print_stdout` in this one
   CLI crate". Any lint that needs crate-scoped exceptions has to live
   outside `[workspace.lints]`.

Beyond these, the existing policy had no explicit statement of *what is
protected from lint drift*. `upstream/comrak/` must never inherit our
lint profile — the 200-line diff budget (ADR-0001) depends on
re-applying hooks cleanly, and a lint-induced reformat would blow that
budget. This is currently ensured by `exclude = ["upstream/comrak",
"crates/afm-book"]` in the root `Cargo.toml`, but that invariant was not
documented anywhere a future reader would see it before touching lint
config.

## Decision

Adopt a single, written lint policy for the workspace:

### Source of truth

`[workspace.lints]` in the root `Cargo.toml` is the **only** place lint
levels are declared. No `-W lint::*` / `-D lint::*` CLI flags in
`Justfile`, `lefthook.yml`, or `.github/workflows/`. The CLI surface for
`just clippy` is capped at `-D warnings` — promote warnings to errors,
nothing more.

The three clippy groups (`pedantic`, `nursery`, `cargo`) are enabled at
`warn` with `priority = -1` so individual lint overrides (either promote
to `deny` or demote to `allow`) sit at the default priority and take
precedence.

### Scope

Workspace-level lints apply only to `[workspace] members`. The workspace
`exclude` list — currently `upstream/comrak`, `crates/afm-book` —
structurally isolates:

- **`upstream/comrak/`** — vendored fork, untouched lint-wise. `cargo
  clippy --workspace` does not visit it. ADR-0001 governs modifications
  there.
- **`crates/afm-book`** — mdbook site, not a Rust crate.

Any future crate added to `exclude` must be documented in this ADR.

### Group composition

Enabled lint groups (all at `warn`, priority -1):

- `clippy::pedantic` — high-signal idiom checks.
- `clippy::nursery` — newer lints still being refined upstream.
- `clippy::cargo` — manifest hygiene.
- (Rustc) `rust_2024_compatibility` — edition-drift warnings.

Individual promotions live inline with documented rationale:

- `dbg_macro`, `todo`, `unimplemented`, `clone_on_ref_ptr`,
  `unnecessary_safety_{comment,doc}`, `missing_assert_message`.

Hand-picked entries from `clippy::restriction` (the group is **never**
enabled as a whole — it contains mutually contradictory lints):

- `str_to_string`, `absolute_paths`, `allow_attributes_without_reason`,
  `empty_structs_with_brackets`, `if_then_some_else_none`,
  `let_underscore_must_use`, `mixed_read_write_in_expression`,
  `rest_pat_in_fully_bound_structs`, `same_name_method`,
  `semicolon_outside_block`, `try_err`, `unneeded_field_pattern`,
  `verbose_file_reads`, `format_push_string`, `unnecessary_self_imports`,
  `assertions_on_result_states`, `filetype_is_file`, `rc_buffer`,
  `rc_mutex`, `lossy_float_literal`, `suspicious_xor_used_as_pow`.

Explicit carve-outs with rationale:

- `module_name_repetitions = "allow"` — fighting it hurts readability.
- `multiple_crate_versions = "allow"` — transitive dep graph is not
  ours.
- `missing_const_for_fn = "allow"` — revisits when crate stabilises.
- `redundant_pub_crate = "allow"` — fundamentally conflicts with
  rustc's `unreachable_pub`; we pick `unreachable_pub` as truth.

`[workspace.lints.rust]` adds lifetime / path hygiene lints
(`unreachable_pub`, `single_use_lifetimes`, `elided_lifetimes_in_paths`,
`redundant_lifetimes`, `explicit_outlives_requirements`,
`let_underscore_drop`, `keyword_idents_2024`, `variant_size_differences`,
`ambiguous_negative_literals`) and bans non-ASCII identifiers
(`non_ascii_idents = "deny"`).

`[workspace.lints.rustdoc]` enables `broken_intra_doc_links = "deny"`
(hard fail) and six warn-level lints for doc hygiene.

### Crate-level overrides

When a specific crate needs a lint relaxed (e.g. `print_stdout` in
`afm-cli` and `xtask`), we do **not** reach for `[lints.clippy]` at the
crate level — Cargo disallows combining that with `[lints] workspace =
true`. Instead:

- If the rule is a lint, replicate the enforcement in `strict-code`
  (the `just lint` grep gate), scoping by path. Example: `println!` is
  forbidden in `afm-syntax`, `afm-parser`, `afm-encoding` sources but
  allowed in `afm-cli` and `xtask`.
- If the rule is a lint that *must* fire per-crate, remove
  `workspace = true` from that crate and replicate the full list. This
  is last-resort and should trigger an ADR update.

### Prohibited

- `#[allow(...)]`, `#![allow(...)]`, `#[cfg_attr(..., allow(...))]` in
  **source code** — `strict-code` rejects all three. If a lint fires on
  code that is genuinely correct, fix the lint list in this ADR, don't
  silence the site.
- `continue-on-error`, `--ignore-*` flags in CI — same rationale,
  enforced by `feedback_no_warning_suppression`.
- Nightly-only rustfmt options — the repo pins a stable channel
  (`rust-toolchain.toml`). Formatting is not worth a toolchain split.

### Change procedure

Adding a new lint entry:

1. Propose in a PR that modifies `Cargo.toml`.
2. Run `just lint` (clippy + fmt-check + typos + strict-code) and
   `just test-doc` (rustdoc lints). Fix every warning at its root.
3. Update this ADR if the addition changes policy (new group, new
   carve-out, scope change). Additions that just extend the
   restriction list do not need an ADR update.

## Consequences

**Becomes easier:**

- Onboarding: a single file (`Cargo.toml`) and this ADR describe the
  whole lint posture. No CLI-vs-config drift.
- Toolchain bumps: rustfmt defaults can change, but this policy locks
  in our intent.
- Adding new crates: `lints.workspace = true` in the new `Cargo.toml`
  and they inherit the full profile automatically.

**Becomes harder:**

- Adding per-crate lint exceptions needs a deliberate design choice
  (strict-code grep vs. full-manual replication). This is the right
  trade — exceptions should be rare and considered.
- Future rustfmt nightly features are off-limits until either
  (a) they stabilise, or (b) a future ADR justifies the nightly
  toolchain cost.

**Non-consequences:**

- CI remains identical. `just lint` is unchanged in shape; only
  internal config moved.
- `upstream/comrak/` diff budget (ADR-0001) is unaffected — workspace
  `exclude` already protected it; this ADR just writes it down.

## Alternatives considered

**A) Keep lint groups on CLI flags.** `Justfile` passes
`-W clippy::pedantic -W clippy::nursery` explicitly. Matches the
out-of-the-box clippy story shown in the book. *Rejected:* CLI flags
override per-lint carve-outs in `[workspace.lints]`, producing silent
lint drift. The failure mode is asymptomatic (no error; the lint just
re-fires) and surfaces as unexplained CI errors weeks later.

**B) One `[lints.clippy]` section per crate.** Drop `workspace = true`
entirely; each crate owns its full lint list. Gives maximum per-crate
control. *Rejected:* duplication is fragile (list drift across 5
crates) and defeats the purpose of `[workspace.lints]`. The problem it
solves (per-crate overrides for a handful of lints) is better handled
by `strict-code` path-scoped grep gates.

**C) Enable `clippy::restriction` wholesale.** Maximally strict.
*Rejected:* the group contains contradictory lints (e.g. both
`unwrap_used` and `expect_used` plus `implicit_return` and
`needless_return`). Enabling it forces per-lint `allow`s, inverting the
policy. Hand-picking from restriction keeps the declarative surface
honest.

**D) Defer `string_slice` enforcement to fuzz tests only.** The lint
caught real panic surface but also fires on every parser hot-path
indexing where char-boundary invariants are maintained by construction.
*Accepted as hybrid:* dropped the lint from `[workspace.lints]`,
keeping `Span::slice` / `accent.rs` using `.get()` + `.expect()` /
`.and_then(|s| s.chars().next())` as self-documenting invariant
assertions.

## References

- `Cargo.toml` §`[workspace.lints]`, `[workspace.exclude]` — the
  authoritative config.
- `clippy.toml` — threshold values (`too-many-*`, `pass-by-value-size-
  limit`, `disallowed-{methods,types}`).
- `rustfmt.toml` — formatting policy header.
- `Justfile` §`strict-code`, §`clippy` — gate implementation.
- `deny.toml` — license / advisory / bans enforcement.
- ADR-0001 — fork / vendor-in-tree, 200-line diff budget that this
  scope discipline protects.
- ADR-0002 — Docker-only execution (lint runs through `just lint` too).
- rust-lang/cargo#12697 — the workspace-lints override limitation that
  drove the strict-code enforcement path.
- `feedback_no_warning_suppression` — rule that forbids `#[allow]` and
  drove the strict-code grep gate.
