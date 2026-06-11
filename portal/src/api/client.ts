import type { ConversionResult, Portfolio } from '../types'
import { mockPortfolio } from '../data/portfolio'

// The portal depends only on this interface. Today it's backed by mock fixtures;
// once the Rust control plane exists, `HttpBifrostApi` implements the same
// contract against `/api/...` and nothing in the UI changes.
export interface BifrostApi {
  getPortfolio(): Promise<Portfolio>
  convertPipeline(id: string): Promise<ConversionResult>
}

/** Synthesize a representative proposal so the mock client exercises the UI. */
function mockConversion(id: string): ConversionResult {
  const p = mockPortfolio.pipelines.find((x) => x.id === id || x.name === id)
  const name = p?.name ?? id
  return {
    proposal: {
      id: `prop-${id}`,
      pipelineId: id,
      sourceYaml: `trigger:\n  branches:\n    include:\n      - main\n\npool:\n  vmImage: ubuntu-latest\n\nstrategy:\n  matrix:\n    linux:\n      imageName: ubuntu-latest\n    windows:\n      imageName: windows-latest\n\nsteps:\n  - checkout: self\n\n  - task: DownloadSecureFile@1\n    name: signingCert\n    inputs:\n      secureFile: code-signing.pfx\n\n  - script: dotnet build --configuration Release\n    displayName: Build\n\n  - task: acme-corp.deploy.DeployTask@2\n    displayName: Deploy to prod\n    inputs:\n      connectedServiceName: azure-prod\n      environment: pre-deploy\n      clientSecret: $(AZURE_CLIENT_SECRET)`,
      proposedYaml: `name: ${name}\non:\n  push:\n    branches: [ main ]\njobs:\n  build:\n    runs-on: ubuntu-latest\n    steps:\n      - uses: actions/checkout@v4\n      - name: Build\n        run: echo "Importer-converted baseline step"\n\n# ──────────────────────────────────────────────────────────────────────\n# bifrost: gap-fills below — REVIEW BEFORE USE\n# ──────────────────────────────────────────────────────────────────────\n\n# bifrost-gap-fill: DownloadSecureFile@1 (prompt: gap-fill.v1)\n# gap fill for DownloadSecureFile@1`,
      rationale:
        'DownloadSecureFile@1: replaced the secure-file download with an OIDC-authenticated fetch from the platform secret store.',
      riskFlags: ['mock — human review required'],
      verifySteps: ['run the converted workflow in a sandbox'],
      riskBand: p?.riskBand ?? 'amber',
      riskScore: p?.riskScore ?? 50,
      promptId: 'gap-fill.v1',
      confidence: 0.5,
      status: 'draft',
    },
    runbook: {
      items: [
        {
          category: 'secret',
          title: 'Provision repository secret',
          construct: 'secret',
          detail: 'AZURE_CLIENT_SECRET must be configured as a repository secret',
        },
        {
          category: 'service_connection',
          title: 'Federate service connection to GitHub via OIDC',
          construct: 'service-connection',
          detail: 'azure-prod must be federated to GitHub via OIDC',
        },
        {
          category: 'environment',
          title: 'Recreate approval gate as a GitHub Environment',
          construct: 'environment',
          detail: 'pre-deploy approval gate must be recreated as an Environment',
        },
      ],
    },
  }
}

class MockBifrostApi implements BifrostApi {
  async getPortfolio(): Promise<Portfolio> {
    // Simulate a little latency so loading states are exercised.
    await new Promise((r) => setTimeout(r, 350))
    return mockPortfolio
  }

  async convertPipeline(id: string): Promise<ConversionResult> {
    await new Promise((r) => setTimeout(r, 400))
    return mockConversion(id)
  }
}

class HttpBifrostApi implements BifrostApi {
  private readonly base: string
  constructor(base = '/api') {
    this.base = base
  }

  async getPortfolio(): Promise<Portfolio> {
    const res = await fetch(`${this.base}/portfolio`)
    if (!res.ok) throw new Error(`portfolio request failed: ${res.status}`)
    return (await res.json()) as Portfolio
  }

  async convertPipeline(id: string): Promise<ConversionResult> {
    const res = await fetch(`${this.base}/pipelines/${encodeURIComponent(id)}/convert`, {
      method: 'POST',
    })
    if (!res.ok) throw new Error(`convert request failed: ${res.status}`)
    return (await res.json()) as ConversionResult
  }
}

// Flip to the real backend with `VITE_API=http`. Defaults to mock so the portal
// runs standalone with zero backend.
export function createApi(): BifrostApi {
  const mode = import.meta.env.VITE_API ?? 'mock'
  return mode === 'http' ? new HttpBifrostApi() : new MockBifrostApi()
}
