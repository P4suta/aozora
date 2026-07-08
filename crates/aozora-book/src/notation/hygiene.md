# Notation hygiene: canonicalization & degraded rendering

Aozora Bunko notation has accumulated decades of hand-typed variant
spellings: a directive close written `［＃字下げ終わり］` instead of the
canonical `［＃ここで字下げ終わり］`, `ゴチック` for `ゴシック体`, a size
compound the parser has no single construct for. `aozora` handles these with
a deliberate two-part design: **a purified core, and full-acceptance layers on
top of it.**

## The purified core

The parser recognises exactly one *canonical* spelling of each construct. Any
non-canonical variant — a 送り仮名 drift, a synonym, a malformed close, a
size+weight compound — is **not guessed at**. It declines to a lossless
`Unknown` directive that round-trips byte-for-byte: nothing is lost, and the
default output never reinterprets it.

This is a choice. A permissive parser that quietly "fixes" `ゴチック` to bold,
or folds `中文字、ゴシック体` down to one axis, would silently discard what the
author wrote. The core refuses to — it keeps the raw bytes and hands the
judgement to an opt-in layer, so the default `parse` / `fmt` / `render` are
faithful and never surprise you.

## The four layers

Above the core sit the notation-hygiene layers. Each has one job, and only the
opt-in ones ever change what you see:

| Layer | Tool | Changes output? | Tier |
|---|---|---|---|
| Parser | `aozora check`, default `fmt`/`render` | never — lossless | — |
| Linter | `aozora check`, `aozora lint` | never — advisory only | Tier1 |
| Formatter | `aozora fmt --fix` | yes — **rewrites source** | Tier1 |
| Renderer | `aozora render --normalize` | yes — read-only projection | Tier1 |
| Renderer | `aozora render --degraded` | yes — read-only projection | Tier1 + Tier2 |

The default path is byte-identical whether or not these layers exist; every
row that changes output is behind an explicit flag.

## Tier1 vs Tier2

The catalogues split by whether the repair **preserves meaning**:

- **Tier1** — zero-false-positive, meaning-preserving *near-misses*. A fixed
  map of verified variant spellings to their canonical form (送り仮名 drift, a
  synonym, a malformed prefix/close). Loses nothing, so it may safely rewrite
  source (`fmt --fix`) as well as advise (`lint`) and render
  (`render --normalize`).
- **Tier2** — *degraded* reductions Tier1 refuses because they are **lossy** (a
  scope or spelling the parser preserves is erased) or **judgment-laden** (two
  measurement vocabularies folded via a typographic identity). These are
  **render-only** (`render --degraded`): a Tier2 reduction can reach only the
  rendered HTML, and can never rewrite your source. See
  [ADR-0026](https://github.com/P4suta/aozora/blob/main/docs/adr/0026-notation-hygiene-restratification.md).

## Worked examples

### Tier1 — a near-miss close

`［＃字下げ終わり］` is a 送り仮名-dropped spelling of the canonical
`［＃ここで字下げ終わり］`. The parser keeps it `Unknown`; the linter flags it:

```sh
$ printf 'あ［＃字下げ終わり］' | aozora lint
warning[aozora::lint::non_canonical_directive]: non-canonical directive; the canonical form is `ここで字下げ終わり`
```

`fmt --fix` rewrites it to the canonical form in place (idempotently), and a
forward form keeps its target — `［＃「梅」は小書き］` becomes
`［＃「梅」は小文字］`:

```sh
$ printf '「梅」［＃「梅」は小書き］' | aozora fmt --fix
「梅」［＃「梅」は小文字］
```

### Tier2 — a lossy scope

`［＃ここから最後まで３字下げ］` indents until the document (or section) end —
a scope the parser has no construct for, so it stays `Unknown` by default
(rendered as an inert hidden span). Under `--degraded`, the renderer projects
it onto a 3-character indent that the EOF-drain closes at the document end,
which is what 最後まで means:

```sh
$ printf 'あ\n［＃ここから最後まで３字下げ］\n本文\n' | aozora render --degraded
<p>あ</p>
<div class="aozora-container aozora-container-indent aozora-container-indent-3" data-amount="3">
<p>本文</p>
</div>
```

Because it drops the explicit 最後まで scope, this reduction is **render-only**:
`fmt --fix` leaves the bytes verbatim, so your source is never rewritten into a
form that means something subtly different.

### Core purification — a promoted construct

`ゴシック体` is a first-class gothic construct in the core (distinct from
太字/bold; the two are separate typographic weights in the corpus). The
corpus-vanishing variant `ゴチック` declines to `Unknown`, and Tier1 suggests
the canonical:

```sh
$ printf 'あ［＃ここからゴチック］' | aozora lint
warning[aozora::lint::non_canonical_directive]: non-canonical directive; the canonical form is `ここからゴシック体`
```

See
[ADR-0027](https://github.com/P4suta/aozora/blob/main/docs/adr/0027-core-parser-notation-purification.md)
for the full list of the forms the core promotes or declines, and the
[Diagnostics catalogue](diagnostics.md#non-canonical-directive) for the
`non_canonical_directive` lint itself.
