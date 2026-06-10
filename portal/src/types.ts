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
  proposedYaml: string
  rationale: string
  riskFlags: string[]
  verifySteps: string[]
  riskBand: RiskBand
  riskScore: number
  promptId: string
  confidence: number
  status: ProposalStatus
}

/** The conversion-loop output the `/convert` endpoint returns. */
export interface ConversionResult {
  proposal: Proposal
  runbook: Runbook
}
