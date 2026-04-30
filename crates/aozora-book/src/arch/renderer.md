# HTML renderer & canonical serialiser

`aozora-render` ships two walkers over `AozoraTree<'_>`:

- `html::render_to_string` — emits semantic HTML5 with `aozora-*`
  class hooks.
- `serialize::serialize` — emits canonical 青空文庫 source.

Both are pure functions. Both walk the tree once, in source order,
allocating exactly the output buffer (a `String` pre-sized to the
arena footprint).

## HTML renderer

### Class-name scheme

aozora emits stable class names that downstream stylesheets can hook:

| AST node | HTML | Class hook |
|---|---|---|
| `Ruby` | `<ruby>X<rt>Y</rt></ruby>` | (no class — semantic ruby element) |
| `Bouten { kind: Sesame }` | `<em class="aozora-bouten-sesame">…</em>` | `aozora-bouten-<slug>` |
| `Tcy` | `<span class="aozora-tcy">…</span>` | `aozora-tcy` |
| `Gaiji { resolution: Direct }` | `<span data-aozora-gaiji-jis="1-94-37">字</span>` | `data-aozora-gaiji-*` |
| `Gaiji { resolution: Fallback }` | `<span class="aozora-gaiji-fallback" title="…">[…]</span>` | `aozora-gaiji-fallback` |
| `Container { kind: Indent { n: 2 } }` | `<div class="aozora-indent-2">…</div>` | `aozora-indent-<n>` |
| `Container { kind: AlignEnd }` | `<div class="aozora-align-end">…</div>` | `aozora-align-end` |
| `Break::Page` | `<div class="aozora-page-break"/>` | `aozora-page-break` |
| `Kaeriten { mark: Re }` | `<span class="aozora-kaeriten" data-aozora-kaeriten="レ">レ</span>` | `aozora-kaeriten` |

The `aozora-` prefix is reserved for our class names — a downstream
stylesheet can target every aozora-emitted hook with `[class^="aozora-"]`
without conflicting with the consumer's own classes.

### Why a class-hook output instead of inline styles?

Inline styles would force a single typographic decision for every
consumer — print stylesheet, screen stylesheet, e-book renderer, and
LSP/preview pane all want different presentation. The class-hook
output:

- Lets each consumer ship its own stylesheet for its medium.
- Survives content-security-policy regimes that block `style` attrs.
- Stays diff-able (the rendered HTML is stable across runs;
  presentation churn doesn't ripple into snapshot tests).

### HTML escaping

The renderer escapes `<`, `>`, `&`, `"`, `'` in user text exactly
once, at emission. Pre-escaped or doubly-escaped output is a
correctness bug, not a perf decision — every CI run validates
`render_to_string ∘ html_unescape` is the source identity for
plain runs.

## Canonical serialiser

The serialiser is the inverse of the lexer's surface form: walk the
tree, emit the source notation that *would re-parse identically*.
It exists for three reasons:

1. **Round-trip property.** `parse ∘ serialize ∘ parse` must be
   stable on the second iteration. The corpus sweep verifies this
   on every Aozora Bunko work.
2. **`aozora fmt`.** The CLI's `fmt` subcommand canonicalises author
   input (CRLF → LF, accent decomposition, container directive
   spacing).
3. **Diff-quality output.** When the parser drops a malformed
   construct, the serialiser re-emits the surrounding text without
   the offending fragment, so authors can see the *exact* change.

### Why a separate walker, not "render with a different visitor"?

The HTML and canonical-serialise outputs differ on every node type:

- HTML wraps `Ruby { target, reading }` in `<ruby>X<rt>Y</rt></ruby>`;
  serialise emits `｜X《Y》` (or auto-detect form).
- HTML wraps `Container { kind: Indent { n } }` in
  `<div class="aozora-indent-N">…</div>`; serialise emits the
  bracketed directives `［＃ここからN字下げ］…［＃ここで字下げ終わり］`.
- HTML emits `<span data-aozora-gaiji-jis="1-94-37">字</span>` for a
  resolved gaiji; serialise emits the original `※［＃…、第3水準1-94-37］`.

The transformations don't share enough structure to fit a single
"visitor with two methods per node" abstraction. Two purpose-built
walkers stay clearer and slightly faster — the compiler can inline
the per-node match, which a generic visitor with virtual dispatch
prevents.

## Walker shape

Both walkers follow the same shape:

```rust
pub fn render_to_string(tree: &AozoraTree<'_>) -> String {
    let mut buf = String::with_capacity(tree.estimated_html_size());
    walk(tree, &mut buf);
    buf
}

fn walk(tree: &AozoraTree<'_>, out: &mut String) {
    for node in tree.nodes() {
        match node {
            AozoraNode::Plain(s)     => out.push_str(html_escape(s)),
            AozoraNode::Ruby(r)      => emit_ruby(r, out),
            AozoraNode::Bouten(b)    => emit_bouten(b, out),
            AozoraNode::Tcy(t)       => emit_tcy(t, out),
            AozoraNode::Gaiji(g)     => emit_gaiji(g, out),
            AozoraNode::Container(c) => emit_container(c, out),
            AozoraNode::BreakNode(b) => emit_break(b, out),
            // … exhaustive
        }
    }
}
```

Single linear pass; no allocation outside the output buffer; no
recursion that the compiler can't unroll (containers recurse, but
the fan-out is small — typically 1–4 children per container).

## `estimated_html_size` heuristic

The buffer pre-size avoids `String` reallocations during the walk.
Empirical heuristic from the corpus sweep: `2.6 × source_byte_len`
is at the 95th percentile (some HTML wraps a 3-byte ruby kanji in
30 bytes of `<ruby>X<rt>Y</rt></ruby>` markup). Going under leaves
~1 reallocation per render in the worst case; going over wastes
memory on every render. 2.6× is the measured optimum.

## See also

- [Notation overview](../notation/overview.md) — what each AST node
  represents.
- [Borrowed-arena AST](arena.md) — the input shape.
- [Performance → Benchmarks](../perf/bench.md) — the
  `render_hot_path` probe that drives the size estimate.
