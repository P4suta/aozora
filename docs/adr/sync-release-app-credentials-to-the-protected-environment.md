---
date: 2026-11-08
status: proposed
---
# Sync release App credentials to the protected environment

## Context

The release GitHub App requires a private key.
Doppler's [identity limit](https://docs.doppler.com/docs/platform-limits) constrains separate workload identities for every repository and purpose.
The existing release-plz environment restricts jobs to main and requires owner review.

## Decision

Use [native GitHub synchronization](https://docs.doppler.com/docs/github-actions) from a purpose-specific Doppler config into the existing environment.
Keep its branch restriction, owner review, and release gates.
Pass named secrets only to the App token action.
Mint tokens for the current repository and consumer permissions.
Limit the default token to source and qualification reads.
Keep registry OIDC.

## Alternatives

Doppler OIDC avoids GitHub copies and preserves workflow and event claims, but consumes identity capacity.
A service token adds another stored credential.

## Consequences

GitHub stores App credential copies.
Environment protection and reviewed main code replace Doppler claims.
The shared App key retains authority across its installations.
Reconsider when GitHub supports workload identity for the App.

## Confirmation

Verify synchronization and manual authentication from reviewed main before retiring the existing identity.
Migration permits no release or tag.
