# Conformance runner contract

An implementation claims conformance to this specification by passing the
test vectors under [`vectors/`](vectors/). This file defines the contract a
runner implements; it is language-agnostic (the vectors are plain JSON +
UTF-8 text).

## Vector format

Each `vectors/<name>/vector.json` validates against
[`schema/vector.schema.json`](schema/vector.schema.json):

```jsonc
{
  "name": "ruby_explicit",
  "meta": { "feature": "ruby", "level": "must", "spec_section": "6.1" },
  "source": "明治の頃｜青梅《おうめ》街道沿いに。",
  "expected": {
    "html":      "<p>…</p>\n",                       // §8 reference rendering
    "serialize": "明治の頃｜青梅《おうめ》街道沿いに。", // §7 byte-exact round-trip
    "nodes":     [ { "kind": "ruby", "span": { "start": 12, "end": 36 } } ],
    "pairs":     [ { "kind": "ruby", "open": {…}, "close": {…} } ],
    "diagnostics": [ /* §9; [] means none */ ]
  }
}
```

All byte offsets are into the **sanitized** source — the output of the §4
pre-processing pass — measured in **UTF-8 bytes**. For input with no BOM, no
CRLF, and no `〔…〕` accent spans, the sanitized bytes equal the source bytes.

## Procedure

For each vector, the runner:

1. parses `source` per this specification;
2. produces the projections it supports (`nodes`, `pairs`, `serialize`,
   `diagnostics`, and optionally `html`);
3. compares each projection present in `expected`.

A projection the implementation does not produce (e.g. it has no HTML
renderer) is **skipped**, not failed — except that a `must`-level vector's
`nodes`, `pairs`, and `diagnostics` are mandatory comparisons.

## Pass / fail by level (§10)

- **`must`** — every comparable projection MUST match exactly. Any mismatch
  is a conformance failure.
- **`should`** — projections SHOULD match; a mismatch is a reported warning,
  not a failure, and MUST be justified.
- **`may`** — informational; divergence is permitted.

## Comparison rules

- `html` / `serialize`: byte-exact string equality (including a trailing
  newline if present).
- `nodes` / `pairs` / `diagnostics`: order-sensitive deep equality. Each
  `diagnostic` matches on `code`, `severity`, and (when present) `span`.
- Spans match on `{start, end}`.

## Notes for a consuming implementation

An implementation SHOULD pin this specification by revision and run the
vectors in its own CI, failing on any `must` mismatch. This is how the
specification acts as the master: the prose, the examples, and the
implementation cannot drift apart without a red build. The vector corpus is
maintained in this repository; `tools/import_vectors.py` can refresh it from
an external fixture set in the standard layout.
