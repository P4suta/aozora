# aozora-go

Go host SDK for the **aozora** Aozora Bunko notation (青空文庫記法) parser.

It runs the portable `aozora.wasm` Extism plugin through the pure-Go
[wazero](https://wazero.io) runtime — **no cgo, no native `libextism`**.
Output is byte-identical to aozora's Rust / WASM / Python / C-ABI front
doors because every binding funnels through the same `aozora::json`
authority.

This is one spoke of aozora's polyglot binding strategy
([ADR-0006](../../docs/adr/0006-polyglot-bindings-via-extism.md)): one
portable wasm artifact + types generated from the JSON Schema, rather
than a hand-written native binding per language.

## Install

For v0.5.0 the SDK is a **tarball channel**: the release attaches
`aozora-go.tar.gz` — a complete, buildable module with the `aozora.wasm`
plugin, generated types, and licences bundled — but there is no
`github.com/P4suta/aozora-go` repository to `go get` (see
[Distribution](#distribution)). The tarball carries the embedded wasm, so
there is no separate plugin download — but the module still requires
`github.com/extism/go-sdk` and its transitive dependencies, so
`go build` / `go mod download` fetches those Go modules from the proxy as
usual.

Download `aozora-go.tar.gz`, verify it against its `.sha256`, and extract
it into your project (for example under `third_party/`):

```sh
curl -LO https://github.com/P4suta/aozora/releases/latest/download/aozora-go.tar.gz
curl -LO https://github.com/P4suta/aozora/releases/latest/download/aozora-go.tar.gz.sha256
sha256sum --check aozora-go.tar.gz.sha256
mkdir -p third_party && tar -xzf aozora-go.tar.gz -C third_party
```

Point your module at the extracted copy with a `replace` directive, then
`go mod tidy`:

```
require github.com/P4suta/aozora-go v0.5.0

replace github.com/P4suta/aozora-go => ./third_party/aozora-go
```

The `github.com/P4suta/aozora-go` import path is served by the local
replacement — no network `go get` is involved.

## Usage

```go
package main

import (
	"context"
	"fmt"

	aozora "github.com/P4suta/aozora-go"
)

func main() {
	ctx := context.Background()
	p, err := aozora.Open(ctx)
	if err != nil {
		panic(err)
	}
	defer p.Close(ctx)

	html, _ := p.ToHTML("｜青梅《おうめ》")
	fmt.Println(html) // <p><ruby>青梅<rp>(</rp><rt>おうめ</rt><rp>)</rp></ruby></p>

	nodes, _ := p.Nodes("｜青梅《おうめ》")
	for _, n := range nodes.Data {
		fmt.Printf("%s @ [%d,%d)\n", n.Kind, n.Span.Start, n.Span.End)
	}
}
```

`Parser` also exposes `ToSource`, `Diagnostics`, `Pairs`,
`ContainerPairs`, and `Gaiji` (each returning the matching JSON envelope),
plus `Slugs` (the static annotation-slug catalogue) and `Version` (the
engine build stamp, distinct from `SchemaVersion`). A `Parser` is not safe
for concurrent use — open one per goroutine.

## Distribution

The SDK is tarball-only because it has no published module mirror: `go get`
resolves a module from a VCS host, and there is no
`github.com/P4suta/aozora-go` repository. A `go get`-able mirror would also
have to commit the ~800 KB `aozora.wasm` the package embeds — it is a build
artifact, git-ignored here, so a naive mirror of this tree fails to build
(`pattern aozora.wasm: no matching files found`). Publishing a tagged
mirror with the wasm committed is deferred; until then the release tarball
is the supported install.

## Layout

| File | Source |
|---|---|
| `aozora.go` | Hand-written transport wrapper (Extism / wazero). |
| `json_gen.go` | **Generated** JSON types — `just types-langs` (quicktype from the JSON Schema). Do not edit. |
| `aozora.wasm` | The embedded plugin artifact — `just extism-build` / the release workflow drops it in (git-ignored in this repo; bundled in the release tarball). |

## Development

From the aozora workspace root: `just smoke-go` builds the plugin, embeds
it here, and runs `gofmt` + `go vet` + `go test`. A bare `go build` in a
fresh checkout fails until that wasm is present — it is git-ignored.
