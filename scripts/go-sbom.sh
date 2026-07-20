#!/usr/bin/env bash
set -euo pipefail

version=${1:?version required}

go list -m -json all |
    jq -s --arg version "$version" '
      {
        bomFormat: "CycloneDX",
        specVersion: "1.6",
        version: 1,
        metadata: {
          component: {
            type: "library",
            name: "aozora-go",
            version: $version
          }
        },
        components: [
          .[]
          | select(.Main != true)
          | (.Replace // .) as $dependency
          | {
              type: "library",
              name: $dependency.Path,
              version: ($dependency.Version // "unknown")
            }
        ]
      }
    '
