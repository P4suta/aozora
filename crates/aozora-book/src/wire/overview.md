# Wire format

aozora ships a stable JSON wire format used by every binding —
`aozora-ffi` (C ABI), `aozora-wasm` (npm), `aozora-py` (PyO3) —
to project the parser's output across language boundaries.
[`aozora::wire`](https://docs.rs/aozora/latest/aozora/wire/index.html)
is the single authority for that projection; downstream drivers
call into it and receive bit-identical output.

## Envelope shape

Every wire JSON has the form

```json
{ "schema_version": 1, "data": [ /* … entries … */ ] }
```

where `schema_version` is the major version of the wire contract and
`data` is the per-endpoint payload array.

The four endpoint envelopes are:

| Endpoint                    | Entry shape                                        | JSON Schema                                                |
| --------------------------- | -------------------------------------------------- | ---------------------------------------------------------- |
| `serialize_diagnostics`     | `{ kind, severity, source, span, codepoint? }`     | [`schema-diagnostics.json`](schema-diagnostics.json)       |
| `serialize_nodes`           | `{ kind, span: { start, end } }`                   | [`schema-nodes.json`](schema-nodes.json)                   |
| `serialize_pairs`           | `{ kind, open: { start, end }, close: { … } }`     | [`schema-pairs.json`](schema-pairs.json)                   |
| `serialize_container_pairs` | `{ kind, open: { offset }, close: { offset } }`    | [`schema-container-pairs.json`](schema-container-pairs.json) |

## SCHEMA_VERSION

The `schema_version` integer (`aozora::wire::SCHEMA_VERSION`)
bumps on any breaking change to the serialised shape — variant
additions exposing as a new `kind` value, field renames, envelope
restructuring. Clients should branch on the version and handle
unknown values defensively; schema 1 makes no forward-compatibility
guarantees with later schemas.

## Stability vs. `non_exhaustive`

[`Diagnostic`](https://docs.rs/aozora/latest/aozora/enum.Diagnostic.html)
and [`AozoraNode`](https://docs.rs/aozora/latest/aozora/syntax/borrowed/enum.AozoraNode.html)
are `#[non_exhaustive]` — minor releases can add variants. The wire
format protects callers in two ways:

1. Unrecognised variants emit `kind: "unknown"` rather than failing to
   serialise, so an old client never sees parse-time data loss.
2. `SCHEMA_VERSION` bumps when new variants ship in the wire surface,
   giving version-branching clients a chance to react before
   `"unknown"` shows up in production traffic.

## See also

- [Diagnostics catalogue](../notation/diagnostics.md) — the source-code
  identifiers each `DiagnosticWire` entry's `kind` field carries.
- [Architecture → Error recovery](../arch/error-recovery.md) — what
  the parser actually *does* after each diagnostic fires.
- [Node reference](../nodes/index.md) — per-`NodeKind` documentation
  for every wire `kind` tag emitted by `serialize_nodes`.
- [`aozora::wire` rustdoc](https://docs.rs/aozora/latest/aozora/wire/index.html)
  — Rust API surface (envelope structs, the `schema_*` introspection
  helpers behind the `schema` Cargo feature).
