// Package aozora runs the Aozora Bunko notation parser through an embedded Extism plugin.
package aozora

import (
	"context"
	_ "embed"
	"encoding/json"
	"fmt"
	"strconv"

	extism "github.com/extism/go-sdk"
)

//go:embed aozora.wasm
var wasmBytes []byte

// Parser holds a loaded plugin instance.
// It is not safe for concurrent use; create one per goroutine or guard it with a mutex.
type Parser struct {
	plugin *extism.Plugin
}

// Open instantiates the embedded plugin and verifies its wire schema version.
// Call Close when done.
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

// ContainerPairs parses source and returns container pairs in source coordinates.
func (p *Parser) ContainerPairs(source string) (AozoraContainerPairsEnvelope, error) {
	var env AozoraContainerPairsEnvelope
	err := p.callJSON("container_pairs_json", source, &env)
	return env, err
}

// Gaiji parses source and returns resolved `※［＃…］` gaiji references.
func (p *Parser) Gaiji(source string) (AozoraGaijiEnvelope, error) {
	var env AozoraGaijiEnvelope
	err := p.callJSON("gaiji_json", source, &env)
	return env, err
}

// Slugs returns the static spec slug catalogue used by completion.
func (p *Parser) Slugs() (AozoraSlugsEnvelope, error) {
	var env AozoraSlugsEnvelope
	err := p.callJSON("slugs_json", "", &env)
	return env, err
}

// Version returns the parser's build version, distinct from the wire SchemaVersion.
func (p *Parser) Version() (string, error) {
	return p.call("version", "")
}

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
