// Package aozora is the Go host SDK for the Aozora Bunko notation
// (青空文庫記法) parser.
//
// It embeds and runs the portable `aozora.wasm` Extism plugin through the
// pure-Go wazero runtime (no cgo, no native libextism), exposing the
// parser's text-in / JSON-out wire API: ruby, bouten, 縦中横, 外字,
// kaeriten, indent containers, page breaks. The same `aozora.wasm` powers
// every language's host SDK, so the output here is byte-identical to the
// Rust / WASM / Python / C-ABI front doors (all funnel through
// `aozora::json`).
//
// The generated JSON types live in json_gen.go (regenerate with
// `just types-langs`). This file is the hand-written transport wrapper.
package aozora

import (
	"context"
	_ "embed"
	"encoding/json"
	"fmt"
	"strconv"

	extism "github.com/extism/go-sdk"
)

// The single portable plugin artifact, produced by `just extism-build`
// and copied here by `just smoke-go` / the release workflow.
//
//go:embed aozora.wasm
var wasmBytes []byte

// SchemaVersion is the wire-format schema version this SDK is built
// against. Open verifies the loaded plugin reports the same version, so a
// plugin/SDK skew fails loudly instead of decoding against the wrong
// shape. Mirrors aozora::json::SCHEMA_VERSION.
const SchemaVersion = 2

// Parser is a loaded aozora plugin instance. It is NOT safe for
// concurrent use; create one per goroutine or guard it with a mutex.
type Parser struct {
	plugin *extism.Plugin
}

// Open instantiates the embedded aozora.wasm plugin and verifies its wire
// schema version matches this SDK. Call Close when done.
func Open(ctx context.Context) (*Parser, error) {
	manifest := extism.Manifest{
		Wasm: []extism.Wasm{extism.WasmData{Data: wasmBytes}},
	}
	plugin, err := extism.NewPlugin(ctx, manifest, extism.PluginConfig{EnableWasi: true}, nil)
	if err != nil {
		return nil, fmt.Errorf("aozora: instantiate plugin: %w", err)
	}
	p := &Parser{plugin: plugin}
	version, err := p.call("schema_version", "")
	if err != nil {
		_ = plugin.Close(ctx)
		return nil, err
	}
	if version != strconv.Itoa(SchemaVersion) {
		_ = plugin.Close(ctx)
		return nil, fmt.Errorf(
			"aozora: plugin wire schema v%s != SDK v%d; upgrade the SDK or the wasm in lock-step",
			version, SchemaVersion,
		)
	}
	return p, nil
}

// Close releases the underlying plugin and its wazero resources.
func (p *Parser) Close(ctx context.Context) error {
	return p.plugin.Close(ctx)
}

// ToHTML parses source and renders it to semantic HTML5.
func (p *Parser) ToHTML(source string) (string, error) {
	return p.call("to_html", source)
}

// ToSource parses source and re-emits it as Aozora notation (round-trip).
// The cross-binding name for source re-emission (Rust to_source, WASM
// toSource, Python to_source).
func (p *Parser) ToSource(source string) (string, error) {
	return p.call("to_source", source)
}

// Diagnostics parses source and returns the diagnostics wire envelope.
func (p *Parser) Diagnostics(source string) (AozoraDiagnosticsEnvelope, error) {
	var env AozoraDiagnosticsEnvelope
	err := p.callJSON("diagnostics_json", source, &env)
	return env, err
}

// Nodes parses source and returns the classified-node wire envelope.
func (p *Parser) Nodes(source string) (AozoraNodesEnvelope, error) {
	var env AozoraNodesEnvelope
	err := p.callJSON("nodes_json", source, &env)
	return env, err
}

// Pairs parses source and returns the matched open/close pair envelope.
func (p *Parser) Pairs(source string) (AozoraPairsEnvelope, error) {
	var env AozoraPairsEnvelope
	err := p.callJSON("pairs_json", source, &env)
	return env, err
}

// ContainerPairs parses source and returns the container open/close pair
// envelope (indent / warichu / keigakomi / alignEnd, in normalized coords).
func (p *Parser) ContainerPairs(source string) (AozoraContainerPairsEnvelope, error) {
	var env AozoraContainerPairsEnvelope
	err := p.callJSON("container_pairs_json", source, &env)
	return env, err
}

// Gaiji parses source and returns the resolved ※［＃…］ gaiji-reference
// wire envelope as raw JSON, byte-identical to aozora::json::gaiji.
func (p *Parser) Gaiji(source string) (string, error) {
	return p.call("gaiji_json", source)
}

// Slugs returns the static spec slug catalogue as a raw-JSON wire
// envelope (byte-identical to aozora::json::slugs). It is input-independent
// — the same envelope every call — so the result can be cached. Powers
// ［＃…］ annotation completion.
func (p *Parser) Slugs() (string, error) {
	return p.call("slugs_json", "")
}

// Version returns the parser's channel-aware build version string (e.g.
// "0.5.0" or "0.5.0-dev+g3672e3f"). This is the engine build stamp,
// distinct from SchemaVersion (the wire-contract version). Mirrors the
// version() export of the WASM / Python drivers.
func (p *Parser) Version() (string, error) {
	return p.call("version", "")
}

// call invokes one plugin export with source bytes and returns its raw
// string output.
func (p *Parser) call(fn, source string) (string, error) {
	exit, out, err := p.plugin.Call(fn, []byte(source))
	if err != nil {
		return "", fmt.Errorf("aozora: %s: %w", fn, err)
	}
	if exit != 0 {
		return "", fmt.Errorf("aozora: %s: plugin exited with code %d", fn, exit)
	}
	return string(out), nil
}

// callJSON invokes a JSON-returning export and decodes the wire envelope.
func (p *Parser) callJSON(fn, source string, v any) error {
	out, err := p.call(fn, source)
	if err != nil {
		return err
	}
	if err := json.Unmarshal([]byte(out), v); err != nil {
		return fmt.Errorf("aozora: decode %s output: %w", fn, err)
	}
	return nil
}
