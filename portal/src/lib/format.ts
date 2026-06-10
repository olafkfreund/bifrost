import type { ChecklistCategory, ProposalStatus, RiskBand } from '../types'

export const riskMeta: Record<RiskBand, { label: string; text: string; bg: string; dot: string; ring: string }> = {
  green: {
    label: 'Green',
    text: 'text-[var(--color-risk-green)]',
    bg: 'bg-[var(--color-risk-green-dim)]',
    dot: 'bg-[var(--color-risk-green)]',
    ring: 'ring-[var(--color-risk-green)]/40',
  },
  amber: {
    label: 'Amber',
    text: 'text-[var(--color-risk-amber)]',
    bg: 'bg-[var(--color-risk-amber-dim)]',
    dot: 'bg-[var(--color-risk-amber)]',
    ring: 'ring-[var(--color-risk-amber)]/40',
  },
  red: {
    label: 'Red',
    text: 'text-[var(--color-risk-red)]',
    bg: 'bg-[var(--color-risk-red-dim)]',
    dot: 'bg-[var(--color-risk-red)]',
    ring: 'ring-[var(--color-risk-red)]/40',
  },
}

export const statusLabel: Record<ProposalStatus, string> = {
  not_started: 'Not started',
  draft: 'Draft',
  in_review: 'In review',
  changes_requested: 'Changes requested',
  approved: 'Approved',
  committed: 'Committed',
  validated: 'Validated',
}

/** Short, human label for a runbook checklist category. */
export const checklistCategoryLabel: Record<ChecklistCategory, string> = {
  secret: 'Secret',
  service_connection: 'Service connection',
  variable_group: 'Variable group',
  self_hosted_runner: 'Runner',
  environment: 'Environment',
  replacement_action: 'Replacement action',
  other: 'Manual task',
}

export function pct(ratio: number): string {
  return `${Math.round(ratio * 100)}%`
}

export function minutes(n: number): string {
  if (n >= 1000) return `${(n / 1000).toFixed(1)}k`
  return String(n)
}
