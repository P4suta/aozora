// Package aozora is the Go host SDK for the Aozora Bunko notation
// (青空文庫記法) parser.
//
// It embeds and runs the portable `aozora.wasm` Extism plugin through the
// pure-Go wazero runtime (no cgo, no native libextism), exposing the
// parser's text-in / JSON-out wire API: ruby, bouten, 縦中横, 外字,
// kaeriten, indent containers, page breaks. The same `aozora.wasm` powers
// every language's host SDK, so the output here is byte-identical to the
// Rust / WASM / Python / C-ABI front doors (all funnel through
// `aozora::wire`).
//
// The generated wire types live in wire_gen.go (regenerate with
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
// shape. Mirrors aozora::wire::SCHEMA_VERSION.
const SchemaVersion = 1

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

// Serialize parses source and re-emits it as Aozora notation (round-trip).
func (p *Parser) Serialize(source string) (string, error) {
	return p.call("serialize", source)
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
