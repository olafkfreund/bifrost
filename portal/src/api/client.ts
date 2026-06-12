import type {
  AuditEvent,
  CompletenessRow,
  ConnectionView,
  ConversionResult,
  Forecast,
  JobProgress,
  Pipeline,
  Portfolio,
  ProposalStatus,
  ReadinessItem,
  SecretRefView,
  SourceStats,
  WavePlan,
} from '../types'
import { mockPortfolio } from '../data/portfolio'

/** Create-connection payload (tagged by `kind`). Secrets are references; an
 *  `inline` value is encrypted server-side. */
export type ConnectionInput = Record<string, unknown> & { name: string; kind: string }

// The portal depends only on this interface. Today it's backed by mock fixtures;
// once the Rust control plane exists, `HttpBifrostApi` implements the same
// contract against `/api/...` and nothing in the UI changes.
export interface BifrostApi {
  getPortfolio(): Promise<Portfolio>
  convertPipeline(id: string): Promise<ConversionResult>
  /** Move a proposal through the lifecycle state machine (audit-logged). */
  transitionProposal(proposalId: string, to: ProposalStatus, actor?: string): Promise<ConversionResult>
  /** Replace a proposal's workflow with a reviewer edit (audit-logged). */
  editProposal(proposalId: string, proposedYaml: string, actor?: string): Promise<ConversionResult>
  /** Start a conversion job; omit `pipelineIds` to convert every not-started pipeline. */
  startConvertJob(pipelineIds?: string[]): Promise<{ jobId: string; total: number }>
  /** Subscribe to a job's live progress. Returns an unsubscribe function. */
  subscribeJob(jobId: string, onUpdate: (progress: JobProgress) => void): () => void
  /** Connections (#157) — Admin-only; the list is always redacted (no secrets). */
  listConnections(): Promise<ConnectionView[]>
  createConnection(input: ConnectionInput): Promise<ConnectionView>
  deleteConnection(id: string): Promise<void>
  /** Onboarding health checks (#161). */
  health(): Promise<boolean>
  me(): Promise<MeView | null>
  /** LLM routing policy (#158) — Admin-only, per-tenant. */
  getRouting(): Promise<{ policy: RoutingPolicy; airGap: boolean }>
  putRouting(policy: RoutingPolicy): Promise<void>
  /** Control-plane settings (#190). */
  getSettings(): Promise<Settings>
  /** Toggle the runtime air-gap posture (Admin-only). */
  setAirGap(enabled: boolean): Promise<Settings>
  /** LLM providers routable for this tenant + how to enable the rest (#197). */
  getProviders(): Promise<ProvidersView>
  /** The pre-migration status report as Markdown (#204), optionally scoped to a project. */
  getReport(project?: string): Promise<string>
  /** The status report as a PDF blob (#221), optionally scoped to a project. */
  getReportPdf(project?: string): Promise<Blob>
  /** Deterministic GitHub Actions cost + capacity forecast (#237). */
  getForecast(): Promise<Forecast>
  /** Wave/cohort program plan (#242). */
  getProgram(): Promise<WavePlan[]>
  /** The `.github/copilot-instructions.md` for migrated repos (#243). */
  getAgentInstructions(): Promise<string>
  /** Source (Azure DevOps) assessment statistics (#240). */
  getSourceStats(): Promise<SourceStats>
  /** Target GitHub pre-flight readiness checklist (#239). */
  getReadiness(): Promise<ReadinessItem[]>
  /** Migration completeness matrix — every ADO moving part + its status (#238). */
  getCompleteness(): Promise<CompletenessRow[]>
  /** Ask the grounded migration assistant a question (#252). Query-only. */
  chat(message: string): Promise<{ reply: string; provider: string }>
}

/** One LLM provider in the routing catalog (#197). */
export interface ProviderInfo {
  /** The exact name to use in the routing policy. */
  name: string
  label: string
  active: boolean
  viaConnection: boolean
  viaEnv: boolean
  /** Env var(s) that enable it (alternative to a portal connection). */
  enableEnv: string
}

export interface ProvidersView {
  live: boolean
  available: string[]
  catalog: ProviderInfo[]
}

/** Control-plane settings surfaced to the portal (#190). */
export interface Settings {
  airGap: boolean
  /** Locked on by the deployment (BIFROST_AIR_GAP_LOCK) — toggle disabled. */
  airGapLocked: boolean
  live: boolean
}

/** Ordered provider-name preference per task class (mirrors bifrost-llm). */
export interface RoutingPolicy {
  bulk: string[]
  hard: string[]
  docs: string[]
}

/** The authenticated identity (`/api/me`), or `null` when not authenticated. */
export interface MeView {
  subject: string
  tenant: string
  roles: string[]
  name?: string
  email?: string
}

/** Legal lifecycle edges — mirrors `is_legal_transition` in bifrost-core. */
const LEGAL: Record<ProposalStatus, ProposalStatus[]> = {
  not_started: [],
  draft: ['in_review'],
  in_review: ['approved', 'changes_requested'],
  changes_requested: ['in_review'],
  approved: ['committed'],
  committed: ['validated'],
  validated: [],
}
/** States in which the workflow can still be edited (mirrors `record_edit`). */
const EDITABLE: ProposalStatus[] = ['draft', 'in_review', 'changes_requested']

/** Lifecycle predecessor, used to seed a coherent audit event in mock mode. */
const PREDECESSOR: Partial<Record<ProposalStatus, ProposalStatus>> = {
  in_review: 'draft',
  changes_requested: 'in_review',
  approved: 'in_review',
  committed: 'approved',
  validated: 'committed',
}

/** Synthesize a representative proposal so the mock client exercises the UI. */
function mockConversion(id: string): ConversionResult {
  const p = mockPortfolio.pipelines.find((x) => x.id === id || x.name === id)
  const name = p?.name ?? id
  // Seed the proposal from the pipeline's portfolio status so opening it from the
  // review queue is consistent (over HTTP the server overlays this for real).
  const status: ProposalStatus = p && p.status !== 'not_started' ? p.status : 'draft'
  const from = PREDECESSOR[status]
  const audit: AuditEvent[] =
    p?.reviewer && from
      ? [{ proposalId: `prop-${id}`, actor: p.reviewer, from, to: status, at: p.reviewedAt ?? new Date().toISOString() }]
      : []
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
      status,
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
    audit,
  }
}

class MockBifrostApi implements BifrostApi {
  // Keep converted proposals in memory so transitions/edits persist across
  // re-opens within a session — the same behaviour the server's store gives.
  private readonly store = new Map<string, ConversionResult>()
  private readonly jobs = new Map<string, { targets: string[]; progress: JobProgress }>()
  private jobCounter = 0

  /** Overlay a stored proposal's status + last actor onto a pipeline (mirrors
   * the server's /portfolio enrichment) so converting updates the queue. */
  private overlay(p: Pipeline): Pipeline {
    const rec = this.store.get(`prop-${p.id}`)
    if (!rec) return p
    const last = rec.audit[rec.audit.length - 1]
    return {
      ...p,
      status: rec.proposal.status,
      reviewer: last?.actor ?? p.reviewer,
      reviewedAt: last?.at ?? p.reviewedAt,
    }
  }

  async getPortfolio(): Promise<Portfolio> {
    // Simulate a little latency so loading states are exercised.
    await new Promise((r) => setTimeout(r, 350))
    return { ...mockPortfolio, pipelines: mockPortfolio.pipelines.map((p) => this.overlay(p)) }
  }

  async convertPipeline(id: string): Promise<ConversionResult> {
    await new Promise((r) => setTimeout(r, 400))
    const key = `prop-${id}`
    let rec = this.store.get(key)
    if (!rec) {
      rec = mockConversion(id)
      this.store.set(key, rec)
    }
    return structuredClone(rec)
  }

  private record(proposalId: string): ConversionResult {
    const rec = this.store.get(proposalId)
    if (!rec) throw new Error(`no proposal '${proposalId}'`)
    return rec
  }

  private log(rec: ConversionResult, from: ProposalStatus, to: ProposalStatus, actor: string, note?: string) {
    const event: AuditEvent = { proposalId: rec.proposal.id, actor, from, to, at: new Date().toISOString() }
    if (note) event.note = note
    rec.audit.push(event)
  }

  async transitionProposal(proposalId: string, to: ProposalStatus, actor = 'reviewer@portal'): Promise<ConversionResult> {
    await new Promise((r) => setTimeout(r, 150))
    const rec = this.record(proposalId)
    const from = rec.proposal.status
    if (!LEGAL[from].includes(to)) throw new Error(`illegal proposal transition: ${from} → ${to}`)
    rec.proposal.status = to
    this.log(rec, from, to, actor)
    return structuredClone(rec)
  }

  async editProposal(proposalId: string, proposedYaml: string, actor = 'reviewer@portal'): Promise<ConversionResult> {
    await new Promise((r) => setTimeout(r, 150))
    const rec = this.record(proposalId)
    const status = rec.proposal.status
    if (!EDITABLE.includes(status)) throw new Error(`proposal is not editable in state ${status}`)
    rec.proposal.proposedYaml = proposedYaml
    this.log(rec, status, status, actor, 'edited proposed_yaml')
    return structuredClone(rec)
  }

  async startConvertJob(pipelineIds?: string[]): Promise<{ jobId: string; total: number }> {
    const targets =
      pipelineIds ??
      mockPortfolio.pipelines.filter((p) => this.overlay(p).status === 'not_started').map((p) => p.id)
    const jobId = `mock-job-${++this.jobCounter}`
    this.jobs.set(jobId, {
      targets,
      progress: { jobId, total: targets.length, done: 0, finished: false, items: [] },
    })
    return { jobId, total: targets.length }
  }

  subscribeJob(jobId: string, onUpdate: (progress: JobProgress) => void): () => void {
    const job = this.jobs.get(jobId)
    if (!job) return () => {}
    let cancelled = false
    let i = 0

    const emit = () => onUpdate(structuredClone(job.progress))
    const tick = async () => {
      if (cancelled) return
      if (i >= job.targets.length) {
        job.progress.finished = true
        emit()
        return
      }
      const pid = job.targets[i++]
      const already = this.store.has(`prop-${pid}`)
      await this.convertPipeline(pid) // populates the store
      job.progress.done++
      job.progress.items.push({ pipelineId: pid, ok: true, skipped: already })
      emit()
      setTimeout(tick, 220)
    }

    emit() // initial snapshot
    setTimeout(tick, 120)
    return () => {
      cancelled = true
    }
  }

  private connections: ConnectionView[] = []
  async listConnections(): Promise<ConnectionView[]> {
    return [...this.connections]
  }
  async createConnection(input: ConnectionInput): Promise<ConnectionView> {
    // Build a redacted view mirroring the server (no secret values stored here).
    const id = `conn-default-${String(input.name).toLowerCase().replace(/[^a-z0-9]+/g, '-')}`
    const view = {
      id,
      tenant: 'default',
      name: input.name,
      kind: redactInput(input),
      updatedBy: 'local@bifrost',
      updatedAt: new Date().toISOString(),
    } as ConnectionView
    this.connections = [...this.connections.filter((c) => c.id !== id), view]
    return view
  }
  async deleteConnection(id: string): Promise<void> {
    this.connections = this.connections.filter((c) => c.id !== id)
  }
  async health(): Promise<boolean> {
    return true
  }
  async me(): Promise<MeView | null> {
    return { subject: 'local', tenant: 'default', roles: ['admin'], name: 'Local Admin' }
  }
  private routing: RoutingPolicy = {
    bulk: ['ollama', 'mock'],
    hard: ['anthropic', 'ollama'],
    docs: ['anthropic'],
  }
  async getRouting(): Promise<{ policy: RoutingPolicy; airGap: boolean }> {
    return { policy: this.routing, airGap: false }
  }
  async putRouting(policy: RoutingPolicy): Promise<void> {
    this.routing = policy
  }
  private airGap = false
  async getSettings(): Promise<Settings> {
    return { airGap: this.airGap, airGapLocked: false, live: false }
  }
  async setAirGap(enabled: boolean): Promise<Settings> {
    this.airGap = enabled
    return { airGap: enabled, airGapLocked: false, live: false }
  }
  async getProviders(): Promise<ProvidersView> {
    const cat = (
      name: string,
      label: string,
      active: boolean,
      enableEnv: string,
    ): ProviderInfo => ({ name, label, active, viaConnection: false, viaEnv: active, enableEnv })
    const catalog = [
      cat('anthropic', 'Anthropic (Claude)', true, 'ANTHROPIC_API_KEY'),
      cat('gemini', 'Google Gemini (AI Studio)', false, 'GEMINI_API_KEY'),
      cat('copilot', 'GitHub Copilot / Models', false, 'GITHUB_MODELS_TOKEN'),
      cat('azure-openai', 'Azure OpenAI Service', false, 'AZURE_OPENAI_ENDPOINT'),
      cat('vertex', 'GCP Vertex AI', false, 'VERTEX_PROJECT + VERTEX_TOKEN'),
      cat('openai-compatible', 'OpenAI-compatible (incl. Bedrock gateway)', false, 'BIFROST_OPENAI_BASE_URL'),
      cat('ollama', 'Ollama (local)', true, 'OLLAMA_BASE_URL'),
    ]
    return { live: false, available: ['anthropic', 'ollama'], catalog }
  }
  async getReport(project?: string): Promise<string> {
    const scope = project ? `Project: ${project}` : 'Whole estate'
    return `# Migration Status Report\n\n> ${scope} — pre-migration assessment. No changes have been made.\n`
  }
  async getReportPdf(project?: string): Promise<Blob> {
    const scope = project ? ` ${project}` : ''
    return new Blob([`%PDF-1.5 (mock${scope})`], { type: 'application/pdf' })
  }
  async getForecast(): Promise<Forecast> {
    // Mirrors the deterministic core model over the demo portfolio, plus an
    // illustrative capacity block (real capacity comes from the Importer forecast).
    const f = computeForecast(mockPortfolio.pipelines)
    f.capacity = {
      peakConcurrency: 6,
      medianQueueMinutes: 0.8,
      p50JobMinutes: 4.5,
      p90JobMinutes: 12.0,
      maxJobMinutes: 38.0,
    }
    return f
  }
  async getAgentInstructions(): Promise<string> {
    return (
      '# Agent instructions\n\n' +
      'These GitHub Actions workflows were migrated from Azure DevOps by Bifrost. ' +
      'This file is context for an agent working in this repo.\n\n' +
      '## Conventions\n\n' +
      '- Workflows live in `.github/workflows/`. Triggers use `on:`; jobs use `runs-on:`.\n' +
      '- Secrets are GitHub Actions secrets; never commit values. Pin actions to a SHA.\n\n' +
      '## When you change a workflow here\n\n' +
      '- Do not reintroduce Azure DevOps constructs (ADO tasks, `$(var)` macros, service connections).\n' +
      '- Review-first: open a pull request; never push converted CI to the default branch.\n\n' +
      '## Required GitHub setup\n\n' +
      '- Secrets to create: `NUGET_API_KEY`\n' +
      '- Service connections to federate (OIDC): `azure-prod` (azurerm)\n' +
      '- Actions to allow-list: `actions/checkout@v4`\n' +
      '- Gap-filled steps to double-check: `DownloadSecureFile@1` (x2)\n\n' +
      '---\n\nGenerated by Bifrost.\n'
    )
  }
  async getProgram(): Promise<WavePlan[]> {
    // Mirrors `bifrost_core::program` over the demo portfolio.
    const waveOf = (p: Pipeline) =>
      p.classification === 'classic' || p.riskBand === 'red' ? 3 : p.riskBand === 'green' ? 1 : 2
    const meta: Record<number, { name: string; rationale: string }> = {
      1: { name: 'Pilot', rationale: 'Low-risk YAML pipelines — migrate these first to prove the process.' },
      2: { name: 'Early majority', rationale: 'Amber YAML pipelines — standard conversions once the pilot succeeds.' },
      3: { name: 'Late majority', rationale: 'Classic/designer and high-risk pipelines — the hard tail; needs the most review.' },
    }
    const inProg = new Set(['draft', 'in_review', 'changes_requested'])
    const isDone = new Set(['approved', 'committed', 'validated'])
    return [1, 2, 3].map((wave) => {
      const members = mockPortfolio.pipelines.filter((p) => waveOf(p) === wave).map((p) => this.overlay(p))
      const c = (pred: (p: Pipeline) => boolean) => members.filter(pred).length
      return {
        wave,
        name: meta[wave].name,
        rationale: meta[wave].rationale,
        pipelines: members.length,
        green: c((p) => p.riskBand === 'green'),
        amber: c((p) => p.riskBand === 'amber'),
        red: c((p) => p.riskBand === 'red'),
        yaml: c((p) => p.classification === 'yaml'),
        classic: c((p) => p.classification === 'classic'),
        forecastMinutes: members.reduce((s, p) => s + p.forecastMinutes, 0),
        notStarted: c((p) => p.status === 'not_started'),
        inProgress: c((p) => inProg.has(p.status)),
        done: c((p) => isDone.has(p.status)),
        projects: [...new Set(members.map((p) => p.project).filter(Boolean))].sort(),
      }
    })
  }
  async getSourceStats(): Promise<SourceStats> {
    const t = mockPortfolio.summary.totals
    const ps = mockPortfolio.pipelines
    const byMap = new Map<string, { pipelines: number; yaml: number; classic: number; red: number }>()
    for (const p of ps) {
      const c = byMap.get(p.project) ?? { pipelines: 0, yaml: 0, classic: 0, red: 0 }
      c.pipelines += 1
      if (p.classification === 'classic') c.classic += 1
      else c.yaml += 1
      if (p.riskBand === 'red') c.red += 1
      byMap.set(p.project, c)
    }
    const byProject = [...byMap.entries()]
      .map(([project, v]) => ({ project, ...v }))
      .sort((a, b) => b.pipelines - a.pipelines || a.project.localeCompare(b.project))
    return {
      org: mockPortfolio.summary.org,
      pipelines: t.pipelines,
      projects: t.projects,
      yaml: t.yaml,
      classic: t.classic,
      green: t.green,
      amber: t.amber,
      red: t.red,
      forecastMinutes: t.forecastMinutes,
      // Representative inventory density (matches the Coverage demo).
      serviceConnections: 2,
      variableGroups: 2,
      secrets: 3,
      selfHostedRunners: 1,
      customTaskTypes: 7,
      actionsAllowlist: 4,
      byProject,
      uncollected: [
        'Last-run date / dormant vs active pipelines',
        'Historical success-rate and build-duration baseline',
        'Owning team per pipeline',
        'Repository size vs GEI limits (40 GiB / 400 MiB)',
      ],
    }
  }
  async getReadiness(): Promise<ReadinessItem[]> {
    // Representative checklist mirroring `bifrost_core::readiness` over the demo audit.
    const i = (
      category: string,
      status: ReadinessItem['status'],
      detail: string,
      action: string,
    ): ReadinessItem => ({ category, status, detail, action })
    return [
      i('Identity & SSO', 'unverified', 'SAML/OIDC SSO and SCIM provisioning must be configured for the target org.', 'Confirm SSO + SCIM in the GitHub enterprise/org settings.'),
      i('Actions runners', 'action', '1 self-hosted runner referenced; size GitHub runners to the forecast peak concurrency.', 'Provision runners/runner-groups and match the labels the workflows expect.'),
      i('Actions policy', 'action', '4 actions the converted workflows use must be allowed.', 'Add them to the org Actions allow-list (pin to SHAs for supply-chain safety).'),
      i('OIDC federation', 'action', '2 service connections need OIDC/Entra workload-identity federation.', "Configure federated credentials. Note: GitHub's OIDC sub claim format changes for repos created after 2026-07-15."),
      i('Secret management', 'action', '3 secrets to create (names only — values are never read).', 'Create them as Actions secrets at the right scope (repo/env/org).'),
      i('Variables', 'action', '2 variable groups to recreate.', 'Recreate as repo/org/environment variables; ADO stage-scoped variables have no direct equivalent.'),
      i('Branch rulesets', 'unverified', 'Branch protection (required reviews, status checks, no force-push) should be defined before import.', 'Define org/repo rulesets; note rulesets can block a migration if they conflict.'),
      i('Ownership / RACI', 'unverified', 'Assign an owner per project (5 projects) and a change board.', 'Confirm ownership; owning team per pipeline is not yet collected (see Assessment).'),
      i('Rollback plan', 'unverified', 'A documented rollback is required before cutover.', 'Keep Azure DevOps live ~30 days post-cutover; document how to revert.'),
      i('Egress posture', 'ready', 'Air-gap is ON — only in-network providers are used; no pipeline data leaves the box.', 'Confirm the egress posture matches your compliance requirement.'),
    ]
  }
  async getCompleteness(): Promise<CompletenessRow[]> {
    // Representative matrix mirroring `bifrost_core::completeness` over the demo
    // portfolio. Not-yet-inventoried categories are shown honestly, never omitted.
    const r = (
      category: string,
      count: number,
      inventoried: boolean,
      status: CompletenessRow['status'],
      githubEquivalent: string,
      note: string,
    ): CompletenessRow => ({ category, count, inventoried, status, githubEquivalent, note })
    return [
      r('YAML pipelines', 11, true, 'auto', 'GitHub Actions workflows', 'Converted automatically; inspect before production use.'),
      r('Triggers (CI / PR / schedule / path)', 0, false, 'auto', 'on: push / pull_request / schedule', 'Converted automatically by the Importer.'),
      r('Actions allow-list', 4, true, 'manual', 'Org/repo Actions policy', 'Add these actions to the GitHub Actions allow-list.'),
      r('Classic / designer pipelines', 5, true, 'review', 'Workflows (reverse-engineered)', 'The hard tail — UI-defined logic; needs the most review.'),
      r('Unsupported / partial steps', 7, true, 'review', 'Gap-filled workflow steps', 'The model fills each gap from the diff; a human approves.'),
      r('Secrets', 3, true, 'manual', 'Actions secrets', 'Names only — values are never read; re-enter them in GitHub.'),
      r('Service connections', 2, true, 'manual', 'OIDC federation / secrets', 'Azure connections become Entra workload-identity federation.'),
      r('Variable groups', 2, true, 'manual', 'Repo/org/environment variables', 'ADO stage-scoped variables have no direct GitHub equivalent.'),
      r('Self-hosted runners', 1, true, 'manual', 'Self-hosted runners / runner groups', 'Provision runners and match the expected labels.'),
      r('Secure files', 0, false, 'notInventoried', 'Actions secrets / external vault', 'Not yet enumerated; check the ADO library.'),
      r('Task groups', 0, false, 'notInventoried', 'Composite actions / reusable workflows', 'Not yet enumerated.'),
      r('Agent pools', 0, false, 'notInventoried', 'Runner labels / runner groups', 'Not yet enumerated.'),
      r('Deployment groups', 0, false, 'notInventoried', 'Self-hosted runner labels', 'Not yet enumerated.'),
      r('Environments + approvals / gates', 0, false, 'notInventoried', 'GitHub Environments + protection rules', 'Commonly missed; not yet enumerated.'),
      r('Azure Artifacts feeds', 0, false, 'notInventoried', 'GitHub Packages', 'Not yet enumerated.'),
      r('Retention policies', 0, false, 'notInventoried', 'Artifact retention / Releases', 'Not yet enumerated.'),
      r('Pipeline permissions', 0, false, 'notInventoried', 'Workflow permissions / GITHUB_TOKEN', 'Not yet enumerated.'),
      r('Repositories (history, branches, PRs)', 0, false, 'notInventoried', 'GitHub Enterprise Importer (GEI)', 'Out of pipeline scope; migrated via GEI.'),
    ]
  }
  async chat(message: string): Promise<{ reply: string; provider: string }> {
    // Offline assistant: a grounded, keyword-aware reply from the demo portfolio
    // (no LLM). The live API routes through the configured provider instead.
    const t = mockPortfolio.summary.totals
    const f = computeForecast(mockPortfolio.pipelines)
    const m = message.toLowerCase()
    let reply: string
    if (/cost|forecast|minute|spend|budget|price/.test(m)) {
      reply = `Projected GitHub Actions cost is about $${Math.round(f.monthlyCostUsd)}/month (${f.totalMinutes.toLocaleString()} runner-minutes on ${f.runnerClass}). The biggest project is ${f.byProject[0]?.project} at $${f.byProject[0]?.costUsd}/mo.`
    } else if (/risk|red|amber|green|danger/.test(m)) {
      reply = `Across ${t.pipelines} pipelines: ${t.green} green, ${t.amber} amber, ${t.red} red. The red ones are mostly classic (designer) pipelines — the hard tail that needs the most review.`
    } else if (/coverage|moving part|secret|connection|missing|left behind|inventor/.test(m)) {
      reply = `Coverage: YAML pipelines convert automatically; classic pipelines and unsupported steps need review; secrets, service connections, variable groups and self-hosted runners are manual GitHub setup (names only — values are never read). Several categories (secure files, task groups, agent pools, environments/gates) are not yet inventoried — check those in Azure DevOps.`
    } else if (/classic|designer/.test(m)) {
      reply = `There are ${t.classic} classic/designer pipelines. They store logic in the ADO UI rather than YAML, so they default Amber/Red and need the most human review during conversion.`
    } else {
      reply = `This estate has ${t.pipelines} pipelines across ${t.projects} projects (${t.yaml} YAML, ${t.classic} classic), forecast at about $${Math.round(f.monthlyCostUsd)}/month on GitHub Actions. Ask me about cost, risk, coverage, or a specific project. (Offline demo — connect an LLM provider for full answers.)`
    }
    return { reply, provider: 'offline-demo' }
  }
}

/** Deterministic cost forecast — the same arithmetic as `bifrost_core::forecast`,
 *  so the mock and the live API agree. Linux 2-core at $0.008/min by default. */
const RUNNER_CLASS = 'ubuntu-latest (2-core)'
const USD_PER_MINUTE = 0.008
const cents = (x: number) => Math.round(x * 100) / 100
function computeForecast(pipelines: Pipeline[]): Forecast {
  const totalMinutes = pipelines.reduce((s, p) => s + p.forecastMinutes, 0)
  const byProjectMap = new Map<string, { pipelines: number; minutes: number }>()
  for (const p of pipelines) {
    const cur = byProjectMap.get(p.project) ?? { pipelines: 0, minutes: 0 }
    cur.pipelines += 1
    cur.minutes += p.forecastMinutes
    byProjectMap.set(p.project, cur)
  }
  const byProject = [...byProjectMap.entries()]
    .map(([project, v]) => ({ project, pipelines: v.pipelines, minutes: v.minutes, costUsd: cents(v.minutes * USD_PER_MINUTE) }))
    .sort((a, b) => b.minutes - a.minutes || a.project.localeCompare(b.project))
  const monthly = cents(totalMinutes * USD_PER_MINUTE)
  return {
    runnerClass: RUNNER_CLASS,
    usdPerMinute: USD_PER_MINUTE,
    totalMinutes,
    monthlyCostUsd: monthly,
    annualCostUsd: cents(monthly * 12),
    byProject,
    notes: [
      `Assumes all minutes on ${RUNNER_CLASS} at $${USD_PER_MINUTE.toFixed(3)}/min — verify against your GitHub plan.`,
      'Excludes any included free minutes and storage; self-hosted runners incur infrastructure cost not shown here.',
    ],
  }
}

/** Redact a create-input into a list-view kind (drops any inline plaintext). */
function redactInput(input: ConnectionInput): ConnectionView['kind'] {
  const redactAuth = (a: unknown): SecretRefView => {
    const auth = a as { type?: string } & Record<string, unknown>
    if (auth?.type === 'inline') return { type: 'encrypted-inline', ciphertext: '', nonce: '' }
    return auth as unknown as SecretRefView
  }
  const k = input as Record<string, unknown>
  if (input.kind === 'azure-devops')
    return { kind: 'azure-devops', org_url: String(k.org_url ?? ''), auth: redactAuth(k.auth) }
  if (input.kind === 'github')
    return { kind: 'github', org: String(k.org ?? ''), auth: redactAuth(k.auth) }
  return {
    kind: 'llm',
    provider: String(k.provider ?? ''),
    base_url: k.base_url as string | undefined,
    model: String(k.model ?? ''),
    key: k.key ? redactAuth(k.key) : undefined,
    is_local: Boolean(k.is_local),
    residency: k.residency as string | undefined,
  }
}

// When Entra SSO is enabled (backend `BIFROST_AUTH=entra`), the browser login
// flow stores the acquired bearer token here; the client attaches it to every
// API call. With SSO disabled the key is absent and requests go out unauthed.
const TOKEN_KEY = 'bifrost_token'

class HttpBifrostApi implements BifrostApi {
  private readonly base: string
  constructor(base = '/api') {
    this.base = base
  }

  // Merge the Authorization header in when a token is present (Entra SSO).
  private headers(extra: Record<string, string> = {}): Record<string, string> {
    const token = sessionStorage.getItem(TOKEN_KEY)
    return token ? { ...extra, authorization: `Bearer ${token}` } : extra
  }

  async getPortfolio(): Promise<Portfolio> {
    const res = await fetch(`${this.base}/portfolio`, { headers: this.headers() })
    if (!res.ok) throw new Error(`portfolio request failed: ${res.status}`)
    return (await res.json()) as Portfolio
  }

  async convertPipeline(id: string): Promise<ConversionResult> {
    const res = await fetch(`${this.base}/pipelines/${encodeURIComponent(id)}/convert`, {
      method: 'POST',
      headers: this.headers(),
    })
    if (!res.ok) throw new Error(`convert request failed: ${res.status}`)
    return (await res.json()) as ConversionResult
  }

  async transitionProposal(proposalId: string, to: ProposalStatus, actor = 'reviewer@portal'): Promise<ConversionResult> {
    const res = await fetch(`${this.base}/proposals/${encodeURIComponent(proposalId)}/transition`, {
      method: 'POST',
      headers: this.headers({ 'content-type': 'application/json' }),
      body: JSON.stringify({ to, actor }),
    })
    if (!res.ok) throw new Error(`${res.status}: ${(await res.text()) || 'transition failed'}`)
    return (await res.json()) as ConversionResult
  }

  async editProposal(proposalId: string, proposedYaml: string, actor = 'reviewer@portal'): Promise<ConversionResult> {
    const res = await fetch(`${this.base}/proposals/${encodeURIComponent(proposalId)}`, {
      method: 'PATCH',
      headers: this.headers({ 'content-type': 'application/json' }),
      body: JSON.stringify({ proposedYaml, actor }),
    })
    if (!res.ok) throw new Error(`${res.status}: ${(await res.text()) || 'edit failed'}`)
    return (await res.json()) as ConversionResult
  }

  async startConvertJob(pipelineIds?: string[]): Promise<{ jobId: string; total: number }> {
    const res = await fetch(`${this.base}/jobs/convert`, {
      method: 'POST',
      headers: this.headers({ 'content-type': 'application/json' }),
      body: JSON.stringify(pipelineIds ? { pipelineIds } : {}),
    })
    if (!res.ok) throw new Error(`start job failed: ${res.status}`)
    return (await res.json()) as { jobId: string; total: number }
  }

  subscribeJob(jobId: string, onUpdate: (progress: JobProgress) => void): () => void {
    const es = new EventSource(`${this.base}/jobs/${encodeURIComponent(jobId)}/events`)
    // Merge the SSE events (a `snapshot` event, then `item`/`done` messages) into
    // one running JobProgress.
    const progress: JobProgress = { jobId, total: 0, done: 0, finished: false, items: [] }

    es.addEventListener('snapshot', (e) => {
      const s = JSON.parse((e as MessageEvent).data)
      progress.total = s.total ?? progress.total
      progress.done = s.done ?? progress.done
      progress.finished = !!s.finished
      progress.items = s.items ?? progress.items
      onUpdate({ ...progress })
      if (progress.finished) es.close()
    })

    es.onmessage = (e) => {
      const ev = JSON.parse(e.data)
      progress.total = ev.total ?? progress.total
      progress.done = ev.done ?? progress.done
      if (ev.item) progress.items = [...progress.items, ev.item]
      if (ev.kind === 'done') progress.finished = true
      onUpdate({ ...progress })
      if (progress.finished) es.close()
    }

    es.onerror = () => {
      // After a clean finish we've already closed; otherwise let EventSource retry.
      if (progress.finished) es.close()
    }

    return () => es.close()
  }

  async listConnections(): Promise<ConnectionView[]> {
    const res = await fetch(`${this.base}/connections`, { headers: this.headers() })
    if (!res.ok) throw new Error(`connections request failed: ${res.status}`)
    return ((await res.json()) as { connections: ConnectionView[] }).connections
  }

  async createConnection(input: ConnectionInput): Promise<ConnectionView> {
    const res = await fetch(`${this.base}/connections`, {
      method: 'POST',
      headers: this.headers({ 'content-type': 'application/json' }),
      body: JSON.stringify(input),
    })
    if (!res.ok) throw new Error(`${res.status}: ${(await res.text()) || 'create failed'}`)
    return ((await res.json()) as { connection: ConnectionView }).connection
  }

  async deleteConnection(id: string): Promise<void> {
    const res = await fetch(`${this.base}/connections/${encodeURIComponent(id)}`, {
      method: 'DELETE',
      headers: this.headers(),
    })
    if (!res.ok) throw new Error(`${res.status}: ${(await res.text()) || 'delete failed'}`)
  }

  async health(): Promise<boolean> {
    try {
      const res = await fetch(`${this.base}/health`)
      return res.ok
    } catch {
      return false
    }
  }

  async me(): Promise<MeView | null> {
    const res = await fetch(`${this.base}/me`, { headers: this.headers() })
    if (res.status === 401) return null
    if (!res.ok) throw new Error(`me request failed: ${res.status}`)
    return (await res.json()) as MeView
  }
  async getRouting(): Promise<{ policy: RoutingPolicy; airGap: boolean }> {
    const res = await fetch(`${this.base}/routing`, { headers: this.headers() })
    if (!res.ok) throw new Error(`routing request failed: ${res.status}`)
    return (await res.json()) as { policy: RoutingPolicy; airGap: boolean }
  }
  async putRouting(policy: RoutingPolicy): Promise<void> {
    const res = await fetch(`${this.base}/routing`, {
      method: 'PUT',
      headers: this.headers({ 'content-type': 'application/json' }),
      body: JSON.stringify(policy),
    })
    if (!res.ok) throw new Error(`${res.status}: ${(await res.text()) || 'save failed'}`)
  }
  async getSettings(): Promise<Settings> {
    const res = await fetch(`${this.base}/settings`, { headers: this.headers() })
    if (!res.ok) throw new Error(`settings request failed: ${res.status}`)
    return (await res.json()) as Settings
  }
  async setAirGap(enabled: boolean): Promise<Settings> {
    const res = await fetch(`${this.base}/settings/air-gap`, {
      method: 'PUT',
      headers: this.headers({ 'content-type': 'application/json' }),
      body: JSON.stringify({ enabled }),
    })
    if (!res.ok) throw new Error(`${res.status}: ${(await res.text()) || 'toggle failed'}`)
    return (await res.json()) as Settings
  }
  async getProviders(): Promise<ProvidersView> {
    const res = await fetch(`${this.base}/providers`, { headers: this.headers() })
    if (!res.ok) throw new Error(`providers request failed: ${res.status}`)
    return (await res.json()) as ProvidersView
  }
  async getReport(project?: string): Promise<string> {
    const q = project ? `?project=${encodeURIComponent(project)}` : ''
    const res = await fetch(`${this.base}/report${q}`, { headers: this.headers() })
    if (!res.ok) throw new Error(`report request failed: ${res.status}`)
    return await res.text()
  }
  async getReportPdf(project?: string): Promise<Blob> {
    const q = project ? `?project=${encodeURIComponent(project)}` : ''
    const res = await fetch(`${this.base}/report.pdf${q}`, { headers: this.headers() })
    if (!res.ok) throw new Error(`report PDF request failed: ${res.status}`)
    return await res.blob()
  }
  async getForecast(): Promise<Forecast> {
    const res = await fetch(`${this.base}/forecast`, { headers: this.headers() })
    if (!res.ok) throw new Error(`forecast request failed: ${res.status}`)
    return (await res.json()) as Forecast
  }
  async getProgram(): Promise<WavePlan[]> {
    const res = await fetch(`${this.base}/program`, { headers: this.headers() })
    if (!res.ok) throw new Error(`program request failed: ${res.status}`)
    return (await res.json()) as WavePlan[]
  }
  async getAgentInstructions(): Promise<string> {
    const res = await fetch(`${this.base}/copilot-instructions`, { headers: this.headers() })
    if (!res.ok) throw new Error(`copilot-instructions request failed: ${res.status}`)
    return await res.text()
  }
  async getSourceStats(): Promise<SourceStats> {
    const res = await fetch(`${this.base}/source-stats`, { headers: this.headers() })
    if (!res.ok) throw new Error(`source-stats request failed: ${res.status}`)
    return (await res.json()) as SourceStats
  }
  async getReadiness(): Promise<ReadinessItem[]> {
    const res = await fetch(`${this.base}/readiness`, { headers: this.headers() })
    if (!res.ok) throw new Error(`readiness request failed: ${res.status}`)
    return (await res.json()) as ReadinessItem[]
  }
  async getCompleteness(): Promise<CompletenessRow[]> {
    const res = await fetch(`${this.base}/completeness`, { headers: this.headers() })
    if (!res.ok) throw new Error(`completeness request failed: ${res.status}`)
    return (await res.json()) as CompletenessRow[]
  }
  async chat(message: string): Promise<{ reply: string; provider: string }> {
    const res = await fetch(`${this.base}/chat`, {
      method: 'POST',
      headers: { ...this.headers(), 'content-type': 'application/json' },
      body: JSON.stringify({ message }),
    })
    if (!res.ok) throw new Error(`chat request failed: ${res.status}`)
    return (await res.json()) as { reply: string; provider: string }
  }
}

// Flip to the real backend with `VITE_API=http`. Defaults to mock so the portal
// runs standalone with zero backend.
export function createApi(): BifrostApi {
  const mode = import.meta.env.VITE_API ?? 'mock'
  return mode === 'http' ? new HttpBifrostApi() : new MockBifrostApi()
}
