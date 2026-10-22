package aozora

import (
	"context"
	"os"
	"strings"
	"testing"
)

func TestInstallDocsHonest(t *testing.T) {
	readme, err := os.ReadFile("README.md")
	if err != nil {
		t.Fatalf("read README.md: %v", err)
	}
	doc := string(readme)
	if strings.Contains(doc, "go get github.com/P4suta/aozora-go") {
		t.Error("README advertises `go get github.com/P4suta/aozora-go`, but no module mirror exists")
	}
	if !strings.Contains(doc, "aozora-go.tar.gz") {
		t.Error("README no longer documents the aozora-go.tar.gz release channel")
	}
}

func TestUsageExampleHTMLMatchesPlugin(t *testing.T) {
	readme, err := os.ReadFile("README.md")
	if err != nil {
		t.Fatalf("read README.md: %v", err)
	}

	ctx := context.Background()
	p, err := Open(ctx)
	if err != nil {
		t.Fatalf("Open: %v", err)
	}
	t.Cleanup(func() { _ = p.Close(ctx) })

	html, err := p.ToHTML("｜青梅《おうめ》")
	if err != nil {
		t.Fatalf("ToHTML: %v", err)
	}
	if !strings.Contains(string(readme), html) {
		t.Errorf("README usage example does not document the real ToHTML output %q", html)
	}
}

func TestEmbeddedPluginIsRealWasm(t *testing.T) {
	if len(wasmBytes) < 4096 {
		t.Fatalf("embedded aozora.wasm is %d bytes; expected a real plugin", len(wasmBytes))
	}
	if string(wasmBytes[:4]) != "\x00asm" {
		t.Fatalf("embedded aozora.wasm lacks the wasm magic header: %q", wasmBytes[:4])
	}
}
