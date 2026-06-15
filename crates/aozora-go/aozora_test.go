package aozora

import (
	"context"
	"strings"
	"testing"
)

// TestEndToEnd loads the embedded aozora.wasm through the Extism Go host
// SDK and exercises every export across the real plugin ABI — the Go
// analogue of the Rust `host_smoke` test. It proves the Go transport
// (wazero → wasm → wire JSON → generated structs) works end to end; the
// byte-identical-to-aozora::wire guarantee itself is pinned by the Rust
// smoke test, so here we assert decoded structure for known inputs.
func TestEndToEnd(t *testing.T) {
	ctx := context.Background()
	p, err := Open(ctx)
	if err != nil {
		t.Fatalf("Open: %v", err)
	}
	t.Cleanup(func() { _ = p.Close(ctx) })

	const ruby = "｜青梅《おうめ》"

	html, err := p.ToHTML(ruby)
	if err != nil {
		t.Fatalf("ToHTML: %v", err)
	}
	if !strings.Contains(html, "おうめ") {
		t.Errorf("ToHTML: ruby reading missing from HTML: %q", html)
	}

	nodes, err := p.Nodes(ruby)
	if err != nil {
		t.Fatalf("Nodes: %v", err)
	}
	if nodes.SchemaVersion != SchemaVersion {
		t.Errorf("Nodes: schema_version = %d, want %d", nodes.SchemaVersion, SchemaVersion)
	}
	if !hasNodeKind(nodes.Data, "ruby") {
		t.Errorf("Nodes: expected a ruby node, got %+v", nodes.Data)
	}

	pairs, err := p.Pairs(ruby)
	if err != nil {
		t.Fatalf("Pairs: %v", err)
	}
	if len(pairs.Data) == 0 {
		t.Errorf("Pairs: expected at least one pair for %q", ruby)
	}

	// U+E001 is a PUA sentinel the parser reserves; source carrying one
	// must raise a source_contains_pua diagnostic.
	diag, err := p.Diagnostics("abc\uE001def")
	if err != nil {
		t.Fatalf("Diagnostics: %v", err)
	}
	if !hasDiagKind(diag.Data, "source_contains_pua") {
		t.Errorf("Diagnostics: expected source_contains_pua, got %+v", diag.Data)
	}

	containers, err := p.ContainerPairs("［＃ここから2字下げ］あ［＃ここで字下げ終わり］")
	if err != nil {
		t.Fatalf("ContainerPairs: %v", err)
	}
	if len(containers.Data) == 0 || containers.Data[0].Kind != "indent" {
		t.Errorf("ContainerPairs: expected an indent container, got %+v", containers.Data)
	}
}

func hasNodeKind(nodes []NodeWire, kind string) bool {
	for _, n := range nodes {
		if n.Kind == kind {
			return true
		}
	}
	return false
}

func hasDiagKind(diags []DiagnosticWire, kind string) bool {
	for _, d := range diags {
		if d.Kind == kind {
			return true
		}
	}
	return false
}
