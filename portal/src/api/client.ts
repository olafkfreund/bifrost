import type {
  AuditEvent,
  ConnectionView,
  ConversionResult,
  JobProgress,
  Pipeline,
  Portfolio,
  ProposalStatus,
  SecretRefView,
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
}

// Flip to the real backend with `VITE_API=http`. Defaults to mock so the portal
// runs standalone with zero backend.
export function createApi(): BifrostApi {
  const mode = import.meta.env.VITE_API ?? 'mock'
  return mode === 'http' ? new HttpBifrostApi() : new MockBifrostApi()
}
