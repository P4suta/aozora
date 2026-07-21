# Release publication is frozen

The qualified v0.5.0 Release PR merge commit is
`7ac70c56e6a392e9807c952367b2493431421869`. Its first release-plz run failed
before the publishing action. At the time of this freeze, that run had created
no `v0.5.0` tag, GitHub Release, or registry publication.

GitHub Actions also has `release-plz.yml` manually disabled. If it is enabled
by mistake, the `publication freeze` job fails while this file exists and both
credential-bearing jobs remain skipped.

Remove this file only in a dedicated, reviewed rearm PR after the recovery
workflow and Actions fan-out fixes have landed, the registry setup has been
rechecked, and a maintainer has explicitly chosen to resume publication. Keep
the workflow disabled while that PR merges; enabling it is a separate final
step.
