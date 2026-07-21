package aozora

import (
	"os"
	"strings"
	"testing"
)

// TestInstallDocsHonest pins the N1 fix: the SDK has no public
// `go get github.com/P4suta/aozora-go` mirror, so the README must not
// re-advertise that broken install path, and must keep documenting the
// real `aozora-go.tar.gz` release channel. The README travels inside the
// tarball, so this also runs against the shipped module.
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

// TestEmbeddedPluginIsRealWasm pins the other half of N1: the module must
// embed a genuine wasm plugin, not an empty or placeholder file that would
// let a broken build ship green. The embed pattern already fails the
// compile when the file is absent; this rejects a truncated / non-wasm
// stand-in (checks the `\0asm` magic and a sane minimum size).
func TestEmbeddedPluginIsRealWasm(t *testing.T) {
	if len(wasmBytes) < 4096 {
		t.Fatalf("embedded aozora.wasm is %d bytes; expected a real plugin", len(wasmBytes))
	}
	if string(wasmBytes[:4]) != "\x00asm" {
		t.Fatalf("embedded aozora.wasm lacks the wasm magic header: %q", wasmBytes[:4])
	}
}
