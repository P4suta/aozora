# 0002. Every dev operation runs inside Docker

- Status: accepted
- Date: 2026-04-23
- Tags: infra, dev-env

## Context

Rust/Node toolchain version drift between contributors and CI is a recurring source
of "works on my machine" failures. This project is Japanese-typography-sensitive —
encoding, Unicode version, OS locales all affect results — and the team is small. We
want the cost of environment setup to fall on the Docker image, not on each human.

## Decision

Every development operation runs inside a container defined by `/Dockerfile` (stages:
`dev`, `ci`, `book`, `browser`). The entry point is `/Justfile`; every target
invokes `docker compose run …`. Host-level `cargo`, `mdbook`, `npm`, `playwright`
invocations are forbidden.

CI (`.github/workflows/ci.yml`) uses the same Justfile targets via
`docker compose run --rm ci just <target>` — the CI environment is structurally
identical to every developer's environment.

## Consequences

Easier:
- Toolchain bumps are one-point (Dockerfile) and propagate everywhere.
- Reproducing a CI failure locally is literally `just ci`.
- Onboarding is `docker compose build dev && just test`.

Harder:
- First-time build of the image takes minutes (mitigated by sccache, mold, multi-stage
  caching, and Dependabot keeping image deps fresh).
- Interactive debuggers (rust-gdb) require devcontainer attach — documented in the
  dev-env section of the README.

## Alternatives considered

- **Host toolchain + rust-toolchain.toml**: insufficient because non-Rust tooling
  (mdbook, playwright, sccache) still drifts.
- **Nix flake**: powerful but the team hasn't invested in Nix knowledge, and Nix on
  WSL/Windows has friction.

## References

- [Docker-only execution memory](../../.claude/projects/-home-yasunobu-projects/memory/feedback_docker_only_execution.md)
- [Justfile](../../Justfile)
- [Dockerfile](../../Dockerfile)
