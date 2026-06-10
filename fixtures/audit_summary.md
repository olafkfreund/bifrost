<!--
  Anonymized `gh actions-importer audit` summary, matching the REAL Importer
  markdown format (bold counts with percentages, plain `Total:` lines, Build-steps
  Known/Unknown/Actions buckets, `${{ secrets.X }}` manual tasks). Captured shape
  from a live audit (org/pipeline names genericized; never any secret values).
-->

# Audit summary

Summary for Azure DevOps

- GitHub Actions Importer version: **1.3.22645**
- Performed at: **1/1/26 at 00:00**

## Pipelines

Total: **3**

- Successful: **1 (33%)**
- Partially successful: **1 (33%)**
- Unsupported: **1 (33%)**
- Failed: **0 (0%)**

### Build steps

Total: **20**

Known: **17 (85%)**

- script: **10**
- checkout: **5**
- AzureCLI@2: **2**

Unknown: **3 (15%)**

- Cache@2: **2**
- TerraformInstaller@1: **1**

Actions: **12**

- run: **8**
- actions/checkout@v4.1.0: **4**

### Manual tasks

Total: **2**

Secrets: **2**

- `${{ secrets.AZURE_CLIENT_SECRET }}`: **1**
- `${{ secrets.REGISTRY_TOKEN }}`: **1**

### Partially successful

#### proj/pipeline-a

- [pipelines/proj/pipeline-a/.github/workflows/pipeline-a.yml](pipelines/proj/pipeline-a/.github/workflows/pipeline-a.yml)
