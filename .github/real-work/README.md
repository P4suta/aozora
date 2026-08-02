# Real-work release baselines

The release gate expects two reviewed inputs in this directory:

- `diagnostics-baseline.json`, generated from every edition in the pinned rights-filtered corpus;
- `site/`, the last approved static site, including `index.html`.

They are intentionally absent until the bootstrap report has been reviewed. A release run fails closed while either input is missing. Baselines are never copied from a failed job or updated automatically.
