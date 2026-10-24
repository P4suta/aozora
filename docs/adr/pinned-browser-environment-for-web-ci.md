---
date: 2026-10-24
---

# Pinned browser environment for web CI

## Status

proposed

## Context

The web gate depends on browser system libraries and Japanese screenshot fonts.
Resolving those prerequisites through Ubuntu package indexes adds mirror-dependent delays to every run.
The browser package and its system image must also agree about browser revisions.

## Decision

Run the web job in the official Playwright image whose version matches the locked playground dependency.
Pin the image by digest and keep the repository's locked mise tools inside that environment.
Install the existing Noto and Takao font packages and their font dependencies from fixed Ubuntu archives.
Verify every archive against the committed checksum manifest on both cache hits and downloads before installation.
Key the archive cache by that manifest.
Use the existing browser suite and screenshot expectations to check rendering compatibility.

This partly supersedes the web CI prerequisite provisioning in [ADR-0053](0053-locked-native-mise-replaces-docker.md).
Native development continues to use its documented host prerequisites.

## Consequences

The web job needs no package index update or Playwright system dependency installation.
Updates to the locked Playwright version require a matching image version and digest.
Font updates require new archive checksums and a rendering check against the existing screenshots.
Cold caches still depend on the Ubuntu archive download service.
A checksum mismatch stops the job before package installation.

Keeping package index installation would retain the observed mirror delay.
A custom browser image would add a second image build and dependency inventory to maintain.
