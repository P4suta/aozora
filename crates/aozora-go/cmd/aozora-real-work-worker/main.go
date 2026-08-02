package main

import (
	"bufio"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"os"

	aozora "github.com/P4suta/aozora-go"
)

type request struct {
	ProtocolVersion int    `json:"protocolVersion"`
	RequestID       string `json:"requestId"`
	Operation       string `json:"operation"`
	Source          string `json:"source"`
}

type result struct {
	Version        string `json:"version"`
	SchemaVersion  int    `json:"schemaVersion"`
	HTML           string `json:"html"`
	Diagnostics    any    `json:"diagnostics"`
	Gaiji          any    `json:"gaiji"`
	Nodes          any    `json:"nodes"`
	Pairs          any    `json:"pairs"`
	ContainerPairs any    `json:"containerPairs"`
	Source         string `json:"source"`
}

type response struct {
	ProtocolVersion int     `json:"protocolVersion"`
	RequestID       string  `json:"requestId"`
	OK              bool    `json:"ok"`
	Result          *result `json:"result,omitempty"`
	Error           string  `json:"error,omitempty"`
}

func render(parser *aozora.Parser, input request) (*result, error) {
	if input.ProtocolVersion != 1 || input.Operation != "render" || input.RequestID == "" {
		return nil, fmt.Errorf("invalid request fields")
	}
	version, err := parser.Version()
	if err != nil {
		return nil, err
	}
	html, err := parser.ToHTML(input.Source)
	if err != nil {
		return nil, err
	}
	source, err := parser.ToSource(input.Source)
	if err != nil {
		return nil, err
	}
	diagnostics, err := parser.Diagnostics(input.Source)
	if err != nil {
		return nil, err
	}
	gaiji, err := parser.Gaiji(input.Source)
	if err != nil {
		return nil, err
	}
	nodes, err := parser.Nodes(input.Source)
	if err != nil {
		return nil, err
	}
	pairs, err := parser.Pairs(input.Source)
	if err != nil {
		return nil, err
	}
	containers, err := parser.ContainerPairs(input.Source)
	if err != nil {
		return nil, err
	}
	return &result{
		Version:        version,
		SchemaVersion:  aozora.SchemaVersion,
		HTML:           html,
		Diagnostics:    diagnostics.Data,
		Gaiji:          gaiji.Data,
		Nodes:          nodes.Data,
		Pairs:          pairs.Data,
		ContainerPairs: containers.Data,
		Source:         source,
	}, nil
}

func run() error {
	ctx := context.Background()
	parser, err := aozora.Open(ctx)
	if err != nil {
		return err
	}
	defer parser.Close(ctx) //nolint:errcheck
	decoder := json.NewDecoder(bufio.NewReader(os.Stdin))
	buffer := bufio.NewWriter(os.Stdout)
	encoder := json.NewEncoder(buffer)
	for {
		var input request
		if err := decoder.Decode(&input); err != nil {
			if errors.Is(err, io.EOF) {
				return buffer.Flush()
			}
			return err
		}
		output, renderErr := render(parser, input)
		message := response{ProtocolVersion: 1, RequestID: input.RequestID, OK: renderErr == nil, Result: output}
		if renderErr != nil {
			message.Error = renderErr.Error()
		}
		if err := encoder.Encode(message); err != nil {
			return err
		}
		if err := buffer.Flush(); err != nil {
			return err
		}
	}
}

func main() {
	if err := run(); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}
