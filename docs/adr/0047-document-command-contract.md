# ADR-0047: One document-command contract

- Status: Accepted
- Date: 2026-07-19
- Supersedes: the coordinate decision in ADR-0008 and the config-error exit
  decision in ADR-0013

## Context

Document subcommands had acquired separate flag sets, diagnostic handling,
coordinate assumptions, and error mappings. Pandoc output selection also used
the same flag name as diagnostic rendering. A command could therefore accept
the same document as another command but decode it differently, render a
diagnostic against a different text axis, or return a different process code.

## Decision

Every document subcommand shares source encoding, diagnostic format, strict
mode, configuration, timing, and watch settings. Single-input commands also
share stdin and file loading. Formatter batch discovery remains specific to
the formatter, but each discovered file uses the same decoder, parser,
diagnostic policy, and original-source coordinate contract.

`--format` selects diagnostic rendering. Pandoc output selection uses
`--to`/`-t`, matching Pandoc's own interface.

Source diagnostics attach the original decoded UTF-8 text because public
spans use original-source byte offsets under ADR-0041. Sanitized and normalized
coordinates do not cross the core boundary.

Process outcomes are shared:

- tolerated diagnostics and completed output succeed;
- strict diagnostics and formatter check mismatches return code 1;
- invalid arguments, unreadable input, decode and configuration errors, and
  failed output-tool invocations return code 2;
- internal diagnostics return code 3;
- a closed output pipe succeeds under ADR-0029.

## Consequences

Scripts can apply one diagnostic policy to parsing, rendering, inspection,
Pandoc projection, and formatting. Code 1 is reserved for a negative document
quality result, while operational and input failures remain distinguishable.
Human diagnostic carets use the same source axis as JSON, bindings, editors,
and edit requests.
