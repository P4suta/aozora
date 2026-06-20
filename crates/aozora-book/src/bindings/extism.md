# Extism host SDKs (Java / PHP / Ruby / … the polyglot tail)

The `aozora-extism` crate compiles to **one** portable
`wasm32-unknown-unknown` artifact — `aozora.wasm` — that any language
with an [Extism](https://extism.org) host SDK can load. The bytes are
identical on every platform, so there is **no per-`(OS × arch)` native
build** to produce, sign, and publish: a Java, PHP, or Ruby host loads
the same wasm a Go host does.

This is the *breadth* strategy for new languages
([ADR-0006](#see-also)). The native bindings stay where they already pay
their way — [Python](python.md) (PyO3) and the browser
[WASM](wasm.md) (wasm-bindgen) are in-process and faster — and the
[C ABI](c.md) remains for max-performance embedders willing to ship a
native library per platform. Extism covers everyone else with a single
artifact and mechanically generated types.

The contract is the same "text in → bytes out" waist as the C ABI: each
export takes the Aozora source as input bytes and returns either HTML, a
round-tripped source string, or a versioned JSON envelope. Every JSON
path delegates to [`aozora::json`](../wire/overview.md) — the single
cross-driver authority — so the output is byte-identical to the C ABI,
the browser WASM, and the PyO3 drivers.

## The plugin contract

`aozora.wasm` exports seven `#[plugin_fn]` entry points. Each takes the
source text as input and returns a string:

| Export | Input | Returns | Shape |
|---|---|---|---|
| `to_html` | source | `string` | Semantic HTML5 with `aozora-*` class hooks. |
| `serialize` | source | `string` | Canonical 青空文庫 source (round-trip). |
| `diagnostics_json` | source | `string` | Wire envelope of diagnostics. |
| `nodes_json` | source | `string` | Wire envelope of source-keyed nodes. |
| `pairs_json` | source | `string` | Wire envelope of matched open/close pairs. |
| `container_pairs_json` | source | `string` | Wire envelope of container open/close pairs. |
| `schema_version` | *(ignored)* | `string` | The wire schema version as a decimal string. |

The four `*_json` exports each emit the standard wire envelope

```json
{ "schemaVersion": 1, "data": [ /* … entries … */ ] }
```

The per-endpoint `data` entry shapes — and the committed JSON Schema for
each — are documented in the [Wire format](../wire/overview.md) chapter.
`to_html` and `serialize` return a bare string (no envelope), and
`schema_version` returns just the integer rendered as text (e.g.
`"1"`); it ignores its input, so a host calls it with an empty buffer.

A source larger than the parser's 4 GiB (`u32::MAX`) span limit is
rejected on the Extism error channel rather than aborting the instance —
the same guard the C ABI and browser WASM apply.

## The schema_version wire contract

Every `*_json` export wraps its payload in
`{ "schemaVersion": N, "data": [...] }`, where `N` is
`aozora::json::SCHEMA_VERSION` baked into the wasm at build time.

A host **MUST** call `schema_version` at load time and assert that the
returned integer equals the version its types were generated for:

1. The wasm and the host's generated types are version-locked. Mismatch
   means the `data` array may not decode into the types you compiled
   against.
2. `schema_version` is a cheap, input-free probe — the canonical place
   to fail fast, before the first real parse.

A `SCHEMA_VERSION` bump is a breaking change to the wire shape (a new
`kind` value, a field rename, an envelope restructuring). Per
[ADR-0006's consequences](#see-also), a bump forces:

- regeneration of **every** language's types (`just types-langs`,
  drift-gated), and
- a **coordinated SDK release** — the wasm release asset and the host
  SDKs are released together, version-locked.

So a host that asserts `schema_version == <generated-for>` at load can
treat any other value as "this wasm is from a different release than my
types" and refuse to proceed, rather than silently decoding against the
wrong shape.

## Worked example: the Go SDK

The reference host SDK is [`aozora-go`](go.md) — a pure-Go host built on
the [wazero](https://wazero.io) runtime (no cgo, no native build). It is
the concrete instance of the language-agnostic pattern below: load
`aozora.wasm`, assert `schema_version`, call the exports, and decode the
envelopes with types generated from the committed JSON Schema. Every
other Extism host SDK follows the identical shape — only the host-SDK API
calls and the generated type syntax differ.

See [`aozora-go`](go.md) for the worked, idiomatic version; the section
below is the template every language instantiates.

## Language-agnostic "call a plugin export" template

The steps are the same in every Extism host SDK; only the method names
and type syntax change.

1. **Obtain `aozora.wasm`.** Download it from a GitHub release asset, or
   build it yourself with `just extism-build` (see
   [Building the plugin](#building-the-plugin)).
2. **Create an Extism plugin from the bytes.** Hand the wasm bytes to
   your host SDK's plugin constructor. WASI is not required — the plugin
   needs no filesystem or environment access.
3. **Call `schema_version` and assert.** Invoke `schema_version` with an
   empty input, parse the returned decimal string to an integer, and
   assert it equals the version your types were generated for. Abort on
   mismatch.
4. **Call `to_html(source)`.** Pass the source bytes; receive the HTML5
   string.
5. **Call `nodes_json(source)`** (or any `*_json` export). Receive the
   JSON envelope string and parse it.
6. **Decode `data` with generated types.** Deserialize the envelope's
   `data` array into the types generated from the committed JSON Schema
   for that endpoint.

```text
plugin  = ExtismPlugin(read("aozora.wasm"))      // step 2

ver     = int(plugin.call("schemaVersion", ""))  // step 3
assert ver == EXPECTED_SCHEMA_VERSION

html    = plugin.call("to_html", source)          // step 4

env     = json_parse(plugin.call("nodes_json", source))   // step 5
assert env.schema_version == EXPECTED_SCHEMA_VERSION
nodes   = decode<NodeWire[]>(env.data)            // step 6
```

> **One plugin instance is not concurrency-safe.** A single Extism
> plugin wraps a single wasm instance with its own linear memory; do not
> call into one instance from multiple threads at once. Use one instance
> per thread, or pool them.

## Per-language pointers

Extism publishes host SDKs for roughly **15 languages** — including
Java, PHP, Ruby, .NET, Elixir, Haskell, OCaml, C/C++, and more — plus the
pure-Go [`aozora-go`](go.md) reference. Browse the current set at the
[Extism host-SDK docs](https://extism.org/docs/concepts/host-sdk).

- **Types** for every supported language are generated from the
  committed wire JSON Schema by `just types-langs` (the quicktype
  driver), wired into the same drift-gate that guards the TypeScript
  `.d.ts`. Generate once per `SCHEMA_VERSION`; commit the output.
- **The wasm** ships as a GitHub release asset (one artifact, all
  platforms) and is reproducible locally via `just extism-build`.

## Building the plugin

```sh
just extism-build
```

Builds `aozora-extism` for `wasm32-unknown-unknown` and runs binaryen's
`wasm-opt` (the pinned, bulk-memory-capable build baked into the dev
image), producing:

- `crates/aozora-extism/dist/aozora.wasm` — the portable plugin artifact.

To exercise it end-to-end:

```sh
just smoke-extism
```

Both run inside the dev image — never invoke `cargo` / `wasm-opt` on the
host.

## See also

- [Choosing a binding](choosing.md) — native vs. C ABI vs. Extism, and
  when to reach for each.
- [Go SDK](go.md) — the reference Extism host SDK (pure-Go wazero).
- [Wire format](../wire/overview.md) — the envelope shape, the four
  endpoint payloads, and their JSON Schemas.
- [C ABI](c.md) — the in-process alternative for embedders that ship a
  native library.
- [ADR-0006](https://github.com/P4suta/aozora/blob/main/docs/adr/0006-polyglot-bindings-via-extism.md)
  — why Extism + schema-driven type generation is the breadth strategy.
