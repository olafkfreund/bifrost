<!--
  Representative `gh actions-importer audit` summary (SYNTHETIC).

  This stands in for a real capture so the parser has a stable fixture to test
  against. Replace it with a genuine audit_summary.md from a sample ADO org when
  one is available (see issue: "Importer-output fixture harness"), and record the
  Importer version it came from. Counts here are internally consistent on purpose.
-->

# Validation Summary

## Pipelines

- Total: 16
- Successful: 9
- Partially successful: 4
- Unsupported: 2
- Failed: 1

## Build steps

- Total: 120
- Successful: 100
- Unsupported: 15

## Unsupported build steps

- DownloadSecureFile@1: 3
- acme-corp.deploy.DeployTask@2: 7
- Kubernetes@1: 5

# Manual tasks

## Secrets

- AZURE_CLIENT_SECRET
- SONAR_TOKEN
- REGISTRY_PASSWORD

## Self hosted runners

- linux-pool: 4
- macos-pool: 1

# Actions

- actions/checkout@v4: 16
- actions/setup-node@v4: 8
- azure/login@v2: 5
