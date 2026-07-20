# ADR-0048: Generated, layered CLI configuration

- Status: Accepted
- Date: 2026-07-19
- Supersedes: The configuration-scope and schema-authority decisions in ADR-0013

## Context

Project configuration alone cannot express a user's stable defaults across
unrelated Aozora workspaces. The document commands also gained shared colour
and message-language settings, while a separately maintained configuration
reference would be another copy of the runtime value set.

ADR-0047 separately supersedes ADR-0013's config-error exit code.

## Decision

Configuration resolves in the order flag, environment, project, global,
default. The nearest `.aozora.toml` is the project layer. The global layer is
`$XDG_CONFIG_HOME/aozora/config.toml`, falling back to
`$HOME/.config/aozora/config.toml`. An explicit `--config` bypasses both
discovered layers.

The supported keys are `encoding`, `format`, `strict`, `color`, and `lang`.
Unknown keys remain errors. XDG paths are resolved directly without a
platform-directory dependency.

The runtime `ConfigFile` and its flag value enums are the configuration-schema
authority. They derive deserialization and JSON Schema from the same Rust
types. `aozora spec schema config` emits that schema, and native distributions
include the exact command output. No hand-maintained key, enum, or schema copy
is shipped.

## Consequences

Project settings override user defaults field by field, and explicit config
remains deterministic. Adding or removing a runtime setting changes the
generated schema in the same compilation unit, so the command model and
published configuration contract cannot drift independently.
