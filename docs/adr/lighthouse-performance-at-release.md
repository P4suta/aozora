---
date: 2026-10-24
---

# Lighthouse performance at release

## Status

proposed

## Context

The shared runner failed the Lighthouse performance floor on #693 while #691 passed with unchanged web code.
The performance score, Largest Contentful Paint (LCP), and Total Blocking Time (TBT) depend on execution timing.
The [fast gate decision](fast-pull-request-gate-and-release-gate.md) places heavy performance checks at the release gate.

## Decision

Pull requests retain Lighthouse assertions for accessibility, best practices, SEO, and Cumulative Layout Shift (CLS).
The release configurations inherit the pull request configurations and add the existing performance score, LCP, and TBT thresholds.
Desktop and mobile audits retain three measurements and median aggregation.
The `release-ready` workflow invokes `lighthouse:release` before producing release artifacts.
The default `lighthouse` command remains the pull request check.

Keeping timing assertions in pull requests would retain shared-runner timing as a merge prerequisite.
Lower thresholds would weaken the existing release contract without separating the gates.

## Consequences

A timing regression can merge and prevents release until fixed.
Accessibility, best practices, SEO, and CLS regressions continue to block pull requests and releases.
Release audits retain their existing thresholds and browser configuration.
