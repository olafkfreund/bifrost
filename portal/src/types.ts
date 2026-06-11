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
