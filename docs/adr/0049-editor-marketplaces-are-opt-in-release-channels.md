# ADR-0049: Editor marketplaces are opt-in release channels

- Status: Accepted
- Date: 2026-07-21
- Supersedes: The automatic editor-marketplace publication decision in ADR-0045

## Context

ADR-0045 gave every distribution channel one version and release cadence so an
editor artifact could not drift from its embedded CLI and wire schema. The VS
Code extension is not yet maintained to the same publication readiness as the
package registries. Automatically attempting Marketplace and Open VSX uploads
would make their credentials and availability prerequisites for otherwise
qualified package releases.

## Decision

The workspace version, `vX.Y.Z` tag, and `release-ready` artifact identity stay
shared. VSIX artifacts remain part of `release-ready`, including every target
package and smoke check.

VS Code Marketplace and Open VSX publication is opt-in. `release-vscode.yml`
runs only by explicit dispatch against an already-qualified commit and matching
tag. A package release does not require either marketplace credential and does
not trigger an editor upload.

The hosted playground remains a continuous deployment of `main`, not a
versioned registry channel.

## Consequences

Package releases cannot accidentally publish an extension or fail for missing
editor credentials. A later editor publication still uses the exact qualified
VSIX artifacts, workspace version, and source tag rather than creating an
independent build or version.
