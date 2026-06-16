# aozora-go

Go host SDK for the **aozora** Aozora Bunko notation (青空文庫記法) parser.

It runs the portable `aozora.wasm` Extism plugin through the pure-Go
[wazero](https://wazero.io) runtime — **no cgo, no native `libextism`** —
so a `go get` is all you need. Output is byte-identical to aozora's Rust /
WASM / Python / C-ABI front doors: every binding funnels through the same
`aozora::wire` authority and the same `aozora.wasm` bytes.

This is one spoke of aozora's polyglot binding strategy
([ADR-0006](../../docs/adr/0006-polyglot-bindings-via-extism.md)): one
portable wasm artifact + types generated from the wire JSON Schema, rather
than a hand-written native binding per language.

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
	fmt.Println(html) // <ruby>青梅<rt>おうめ</rt></ruby>

	nodes, _ := p.Nodes("｜青梅《おうめ》")
	for _, n := range nodes.Data {
		fmt.Printf("%s @ [%d,%d)\n", n.Kind, n.Span.Start, n.Span.End)
	}
}
```

`Parser` also exposes `Serialize`, `Diagnostics`, `Pairs`, and
`ContainerPairs`, each returning the matching wire envelope. A `Parser` is
not safe for concurrent use — open one per goroutine.

## Layout

| File | Source |
|---|---|
| `aozora.go` | Hand-written transport wrapper (Extism / wazero). |
| `wire_gen.go` | **Generated** wire types — `just types-langs` (quicktype from the wire JSON Schema). Do not edit. |
| `aozora.wasm` | The plugin artifact — `just extism-build` / the release workflow drops it in (git-ignored locally). |

## Development

From the aozora workspace root: `just smoke-go` builds the plugin, embeds
it here, and runs `gofmt` + `go vet` + `go test`.
