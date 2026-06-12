// Domain types for the Bifrost portal.
//
// These mirror the shapes the Rust control plane (`bifrost-core`) will expose
// over the JSON API. Keeping them in one place lets the mock client and the
// future HTTP client implement the same contract — see api/client.ts.

/** Deterministic risk band. The score is computed by the engine, never the LLM. */
export type RiskBand = 'green' | 'amber' | 'red'

/** ADO pipeline shape. Classic/designer pipelines are the hard tail. */
export type Classification = 'yaml' | 'classic'

/** Where a converted pipeline sits in the review lifecycle. */
export type ProposalStatus =
  | 'not_started'
  | 'draft'
  | 'in_review'
  | 'changes_requested'
  | 'approved'
  | 'committed'
  | 'validated'

/** A single migration-risk factor with its contribution to the score. */
export interface RiskFactor {
  key: string
  label: string
  /** Weighted contribution to the deterministic score (0–100 scale). */
  contribution: number
  detail: string
}

export interface Pipeline {
  id: string
  name: string
  project: string
  /** Owning source org (multi-org). Empty for single-org audits. */
  org?: string
  classification: Classification
  /** Importer outcome: share of steps converted automatically (0–1). */
  convertedRatio: number
  unsupportedSteps: number
  manualTasks: number
  riskBand: RiskBand
  riskScore: number
  status: ProposalStatus
  /** Forecast Actions runner-minutes/month for this pipeline. */
  forecastMinutes: number
  factors: RiskFactor[]
  /** Who last acted on the proposal (latest audit event), if any. */
  reviewer?: string
  /** When the proposal was last acted on (ISO-8601), if any. */
  reviewedAt?: string
}

export interface PortfolioSummary {
  org: string
  /** Tooling provenance, pinned per audit run for attestation. */
  importerVersion: string
  ado2ghVersion: string
  airGap: boolean
  generatedAt: string
  totals: {
    pipelines: number
    /** Distinct source orgs across the portfolio (multi-org). */
    orgs?: number
    projects: number
    yaml: number
    classic: number
    green: number
    amber: number
    red: number
    /** Total forecast runner-minutes/month across the portfolio. */
    forecastMinutes: number
  }
}

export interface Portfolio {
  summary: PortfolioSummary
  pipelines: Pipeline[]
}

/** A redacted secret reference (no values — server strips encrypted material). */
export type SecretRefView =
  | { type: 'env-var'; name: string }
  | { type: 'key-vault'; uri: string }
  | { type: 'git-hub-app'; installation_id: string }
  | { type: 'entra-wif'; tenant_id: string; client_id: string }
  | { type: 'encrypted-inline'; ciphertext: string; nonce: string }

/** A redacted connection as returned by the API (never carries secret values). */
export interface ConnectionView {
  id: string
  tenant: string
  name: string
  kind:
    | { kind: 'azure-devops'; org_url: string; auth: SecretRefView }
    | { kind: 'github'; org: string; auth: SecretRefView }
    | {
        kind: 'llm'
        provider: string
        base_url?: string
        model: string
        key?: SecretRefView
        is_local?: boolean
        residency?: string
      }
    | {
        kind: 'source'
        platform: string
        base_url?: string
        username?: string
        auth: SecretRefView
      }
  updatedBy: string
  updatedAt: string
}

/** A category of manual follow-up in the runbook (mirrors bifrost-core). */
export type ChecklistCategory =
  | 'secret'
  | 'service_connection'
  | 'variable_group'
  | 'self_hosted_runner'
  | 'environment'
  | 'replacement_action'
  | 'other'

/** One actionable item the Importer cannot do for you. */
export interface ChecklistItem {
  category: ChecklistCategory
  title: string
  construct: string
  detail: string
  /** Must be resolved before the migration can be validated. */
  required?: boolean
  /** Whether a human has marked this task done (#57). */
  done?: boolean
}

/** The per-pipeline manual-task checklist. */
export interface Runbook {
  items: ChecklistItem[]
}

/**
 * An augmented workflow awaiting review. `riskBand`/`riskScore` are computed
 * deterministically; `rationale`/`riskFlags`/`confidence` are the model's.
 */
export interface Proposal {
  id: string
  pipelineId: string
  /** Source ADO pipeline definition — the left-hand side of the review diff. */
  sourceYaml: string
  proposedYaml: string
  rationale: string
  riskFlags: string[]
  verifySteps: string[]
  riskBand: RiskBand
  riskScore: number
  promptId: string
  confidence: number
  status: ProposalStatus
  /** URL of the PR opened when the workflow was committed (set on commit). */
  prUrl?: string
}

/** One immutable entry in a proposal's attestation trail. */
export interface AuditEvent {
  proposalId: string
  actor: string
  from: ProposalStatus
  to: ProposalStatus
  at: string
  /** Set for content actions (e.g. an edit) that don't move state. */
  note?: string
}

/** The conversion-loop output the `/convert` endpoint returns. */
export interface ConversionResult {
  proposal: Proposal
  runbook: Runbook
  /** The proposal's audit trail, oldest first. */
  audit: AuditEvent[]
}

/** One pipeline's outcome within a conversion job. */
export interface JobItem {
  pipelineId: string
  ok: boolean
  /** Already converted in a prior run — skipped (resumability). */
  skipped: boolean
  error?: string
}

/** Progress of a conversion job (fan-out across pipelines). */
export interface JobProgress {
  jobId: string
  total: number
  done: number
  finished: boolean
  items: JobItem[]
}

/** One project's slice of the cost forecast. */
export interface ProjectForecast {
  project: string
  pipelines: number
  minutes: number
  costUsd: number
}

/** Capacity figures from the Importer forecast (run-history based). */
export interface CapacityForecast {
  peakConcurrency: number
  medianQueueMinutes: number
  p50JobMinutes: number
  p90JobMinutes: number
  maxJobMinutes: number
}

/** One project's source-side counts (#240). */
export interface ProjectStat {
  project: string
  pipelines: number
  yaml: number
  classic: number
  red: number
}

/** Source (Azure DevOps) assessment statistics (#240). */
export interface SourceStats {
  org: string
  pipelines: number
  projects: number
  yaml: number
  classic: number
  green: number
  amber: number
  red: number
  forecastMinutes: number
  serviceConnections: number
  variableGroups: number
  secrets: number
  selfHostedRunners: number
  customTaskTypes: number
  actionsAllowlist: number
  byProject: ProjectStat[]
  uncollected: string[]
}

/** Per-project repo + pipeline coordination (#245). */
export interface ProjectCoordination {
  project: string
  pipelines: number
  pipelinesDone: number
  /** Repo migration (GEI) status — `pendingInventory` until an inventory runs. */
  repoStatus: 'pendingInventory'
}

/** A custom field Bifrost would create on the GitHub Project (#265). */
export interface BoardField {
  name: string
  /** GraphQL field data type: `single-select`, `number`, or `date`. */
  dataType: string
  /** Options for a single-select field (empty otherwise). */
  options: string[]
}

/** One issue the program board would carry — one per pipeline (#265). */
export interface PlannedIssue {
  title: string
  wave: number
  risk: string
  status: string
  forecastMinutes: number
  /** The migration checklist, as sub-issues. */
  subIssues: string[]
}

/** Bifrost-computed program KPIs (Projects Insights is UI-only) (#265). */
export interface BoardKpis {
  total: number
  /** Committed or validated. */
  migrated: number
  validated: number
  /** Draft / in-review / changes-requested. */
  inProgress: number
  notStarted: number
  percentDone: number
  forecastMinutes: number
}

/**
 * The deterministic, dry-run plan of the GitHub Projects board Bifrost would
 * stand up for the migration program (#265). Nothing is created on GitHub until
 * provisioning is approved (a separate, gated step).
 */
export interface ProgramBoardPlan {
  repo: string
  projectTitle: string
  fields: BoardField[]
  issues: PlannedIssue[]
  kpis: BoardKpis
  notes: string[]
}

/** One migration wave with its cohort facts + progress (#242). */
export interface WavePlan {
  wave: number
  name: string
  rationale: string
  pipelines: number
  green: number
  amber: number
  red: number
  yaml: number
  classic: number
  forecastMinutes: number
  notStarted: number
  inProgress: number
  done: number
  projects: string[]
}

/** Status of a target-readiness checklist item (#239). */
export type ReadinessStatus = 'ready' | 'action' | 'unverified' | 'blocked'

/** One target GitHub pre-flight checklist item (#239). */
export interface ReadinessItem {
  category: string
  status: ReadinessStatus
  detail: string
  action: string
}

/** Status of one migration category in the completeness matrix (#238). */
export type CategoryStatus = 'auto' | 'review' | 'manual' | 'notInventoried' | 'notApplicable'

/** One ADO moving-part category mapped to its GitHub equivalent + status (#238). */
export interface CompletenessRow {
  category: string
  count: number
  inventoried: boolean
  status: CategoryStatus
  githubEquivalent: string
  note: string
}

/** Deterministic GitHub Actions cost + capacity projection (#237). */
export interface Forecast {
  runnerClass: string
  usdPerMinute: number
  totalMinutes: number
  monthlyCostUsd: number
  annualCostUsd: number
  byProject: ProjectForecast[]
  capacity?: CapacityForecast
  notes: string[]
}
