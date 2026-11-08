---
doppler_automation_app: release_plz
---

# Credential delivery

The release App credentials use Doppler's [native GitHub synchronization](https://docs.doppler.com/docs/github-actions).
The [delivery decision](adr/sync-release-app-credentials-to-the-protected-environment.md) governs the cutover.
Canonical values remain in Doppler, and a purpose-specific config references the automation fields.
The [sync manifest](../automation/release-app-sync.toml) defines the repository identity, protected environment, and field mapping.

The release-plz environment admits main jobs and requires owner review without administrator bypass.
Its RELEASE_APP_CLIENT_ID and RELEASE_APP_PRIVATE_KEY secrets reach only the App token action.
Consumers reject empty inputs and unresolved references before minting repository-scoped tokens with explicit permissions.
The shared App key retains authority across its installations.
Protected main and environment policies replace Doppler workflow and event claims.

The manual **Verify credential access** workflow checks required fields and creates a repository-scoped token with contents read access.
It requires the exact repository identity and main ref and retains the environment's owner review.
Its operation ends after authentication, and the action revokes its token during cleanup.
Existing release qualification, owner authorization, and registry OIDC remain independent gates.

Verify the exact synchronization target and successful initial synchronization before merging the workflow cutover.
Then run the manual authentication check for the reviewed main revision.
Keep the existing Doppler identity until authentication succeeds and all consumers use the replacement.
Replace credentials in their canonical Doppler config, verify synchronization, and repeat the authentication check before retiring the previous key.

Store only field names, identity metadata, source revisions, and verification outcomes in issues and logs.
Keep credential values and native authentication state out of the repository.
