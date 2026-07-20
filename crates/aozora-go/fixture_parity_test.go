package aozora

import (
	"bytes"
	"context"
	"os"
	"path/filepath"
	"testing"
)

// TestFixtureParity is the Go/Extism arm of the cross-surface parity gate:
// one golden authority (crates/aozora-conformance/fixtures/render), N thin
// walkers. It walks every render fixture through the real plugin boundary
// (wazero -> wasm -> wire) and asserts each export is byte-identical to the
// golden the in-process Rust render_gate pins.
//
// It compares the RAW plugin output (via the unexported call), not the
// decoded structs, so a reframed / re-ordered / dropped byte is caught here
// too — the decoded-struct smoke lives in aozora_test.go. All six surfaces
// are byte-exact: every plugin export returns the shared aozora::json bytes
// (and to_html / to_source output) with no framing.
func TestFixtureParity(t *testing.T) {
	root := filepath.Join("testdata", "render")
	if _, err := os.Stat(root); err != nil {
		root = filepath.Join("..", "aozora-conformance", "fixtures", "render")
	}
	entries, err := os.ReadDir(root)
	if err != nil {
		t.Fatalf("read fixtures root %s: %v", root, err)
	}

	ctx := context.Background()
	p, err := Open(ctx)
	if err != nil {
		t.Fatalf("Open: %v", err)
	}
	t.Cleanup(func() { _ = p.Close(ctx) })

	// surface plugin export -> golden file with byte-identical output.
	surfaces := []struct{ export, file string }{
		{"to_html", "expected.html"},
		{"to_source", "expected.serialize.txt"},
		{"diagnostics_json", "expected.diagnostics.json"},
		{"nodes_json", "expected.nodes.json"},
		{"pairs_json", "expected.pairs.json"},
		{"container_pairs_json", "expected.container_pairs.json"},
	}

	count := 0
	for _, e := range entries {
		if !e.IsDir() {
			continue
		}
		count++
		dir := filepath.Join(root, e.Name())
		src, err := os.ReadFile(filepath.Join(dir, "source.txt"))
		if err != nil {
			t.Fatalf("%s: read source: %v", e.Name(), err)
		}
		for _, s := range surfaces {
			golden, err := os.ReadFile(filepath.Join(dir, s.file))
			if err != nil {
				t.Fatalf("%s: read golden %s: %v", e.Name(), s.file, err)
			}
			if filepath.Ext(s.file) == ".json" {
				golden = bytes.TrimSuffix(golden, []byte{'\n'})
			}
			got, err := p.call(s.export, string(src))
			if err != nil {
				t.Fatalf("%s: call %s: %v", e.Name(), s.export, err)
			}
			if got != string(golden) {
				t.Errorf("%s/%s drift:\n golden=%q\n actual=%q", e.Name(), s.file, string(golden), got)
			}
		}
	}
	if count == 0 {
		t.Fatalf("no fixtures under %s", root)
	}
}
