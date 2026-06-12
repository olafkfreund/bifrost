import { useEffect, useState } from 'react'
import type { BifrostApi } from '../api/client'
import type { ProgramBoardPlan } from '../types'

const num = (n: number) => n.toLocaleString()

const WAVE_NAME: Record<number, string> = { 1: 'Pilot', 2: 'Early', 3: 'Late' }

/** A KPI stat tile. */
function Kpi({ label, value, accent }: { label: string; value: string; accent?: string }) {
  return (
    <div className="bf-card rounded-xl p-4">
      <div className="text-[10px] font-semibold uppercase tracking-[0.12em] text-ink-400">{label}</div>
      <div className={`tnum mt-1 font-display text-2xl font-semibold ${accent ?? 'text-ink-100'}`}>{value}</div>
    </div>
  )
}

const riskColor = (risk: string) =>
  risk === 'Green'
    ? 'var(--color-risk-green)'
    : risk === 'Amber'
      ? 'var(--color-risk-amber)'
      : risk === 'Red'
        ? 'var(--color-risk-red)'
        : 'var(--color-ink-300)'

/**
 * The program board (#265) — a deterministic, **dry-run** view of the GitHub
 * Projects board Bifrost would stand up for the migration. Nothing here is
 * created on GitHub: provisioning the org Project + dedicated repo is a separate,
 * approval-gated step (Phase 2). This is the plan a program manager reviews first.
 */
export function Board({ api }: { api: BifrostApi }) {
  const [plan, setPlan] = useState<ProgramBoardPlan | null>(null)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    api.getProgramBoardPlan().then(setPlan).catch((e) => setError(String(e)))
  }, [api])

  if (error) {
    return (
      <div className="flex flex-1 items-center justify-center text-[var(--color-risk-red)]">
        Failed to load board plan: {error}
      </div>
    )
  }
  if (!plan) {
    return (
      <div className="flex flex-1 items-center justify-center text-ink-300">
        <div className="animate-pulse">Planning the board…</div>
      </div>
    )
  }

  const k = plan.kpis

  return (
    <main className="mx-auto w-full max-w-7xl flex-1 px-6 py-6">
      <div className="mb-5 flex flex-wrap items-end justify-between gap-3">
        <div>
          <h1 className="text-xl font-semibold text-ink-100">Program board</h1>
          <p className="text-sm text-ink-300">
            The GitHub Projects board for this migration — one issue per pipeline, the migration checklist as
            sub-issues, KPIs for management. This is a dry-run plan; nothing is created until you approve provisioning.
          </p>
        </div>
        <div
          title="Provisioning the org Project + dedicated repo is a separate, approval-gated step (Phase 2)."
          className="inline-flex items-center gap-2 rounded-lg border border-ink-800 bg-ink-900/40 px-3 py-1.5 text-xs text-ink-300"
        >
          <span className="h-1.5 w-1.5 rounded-full bg-[var(--color-risk-amber)]" />
          Dry-run · not yet provisioned
        </div>
      </div>

      {/* What would be created */}
      <div className="bf-card mb-5 flex flex-wrap items-center gap-x-8 gap-y-2 rounded-xl p-4 text-sm">
        <div>
          <span className="text-ink-400">Project</span>{' '}
          <span className="font-medium text-ink-100">{plan.projectTitle}</span>
        </div>
        <div>
          <span className="text-ink-400">Issues repo</span>{' '}
          <span className="font-mono text-ink-100">{plan.repo}</span>
        </div>
        <div>
          <span className="text-ink-400">Custom fields</span>{' '}
          <span className="tnum font-medium text-ink-100">{plan.fields.length}</span>
        </div>
      </div>

      {/* KPIs — what management sees */}
      <div className="mb-6 grid grid-cols-2 gap-3 sm:grid-cols-3 lg:grid-cols-6">
        <Kpi label="Pipelines" value={num(k.total)} />
        <Kpi label="Migrated" value={num(k.migrated)} accent="text-[var(--color-risk-green)]" />
        <Kpi label="Validated" value={num(k.validated)} accent="text-[var(--color-risk-green)]" />
        <Kpi label="In progress" value={num(k.inProgress)} accent="text-[var(--color-brand-400)]" />
        <Kpi label="Not started" value={num(k.notStarted)} />
        <Kpi label="Percent done" value={`${k.percentDone}%`} accent="text-[var(--color-risk-green)]" />
      </div>

      <div className="grid gap-6 lg:grid-cols-[1fr_18rem]">
        {/* Planned issues */}
        <div>
          <h2 className="text-sm font-semibold text-ink-100">Planned issues</h2>
          <p className="mb-3 mt-0.5 text-xs text-ink-300">
            One issue per pipeline, tagged with its wave, risk and lifecycle status. Each carries the migration
            checklist as sub-issues.
          </p>
          <div className="bf-card overflow-hidden rounded-xl">
            <table className="w-full text-sm">
              <thead>
                <tr className="border-b border-ink-800 text-left text-xs uppercase tracking-wide text-ink-300">
                  <th className="px-4 py-2.5 font-medium">Issue</th>
                  <th className="px-4 py-2.5 font-medium">Wave</th>
                  <th className="px-4 py-2.5 font-medium">Risk</th>
                  <th className="px-4 py-2.5 font-medium">Status</th>
                  <th className="px-4 py-2.5 text-right font-medium">Min/mo</th>
                  <th className="px-4 py-2.5 text-right font-medium">Sub-issues</th>
                </tr>
              </thead>
              <tbody>
                {plan.issues.map((it, i) => (
                  <tr key={`${it.title}-${i}`} className="border-b border-ink-800 last:border-0">
                    <td className="px-4 py-2.5 text-ink-100">{it.title}</td>
                    <td className="px-4 py-2.5 text-ink-300">
                      {it.wave} · {WAVE_NAME[it.wave] ?? ''}
                    </td>
                    <td className="px-4 py-2.5">
                      <span className="inline-flex items-center gap-1.5" style={{ color: riskColor(it.risk) }}>
                        <span className="h-1.5 w-1.5 rounded-full" style={{ background: riskColor(it.risk) }} />
                        {it.risk}
                      </span>
                    </td>
                    <td className="px-4 py-2.5 text-ink-300">{it.status}</td>
                    <td className="tnum px-4 py-2.5 text-right text-ink-300">{num(it.forecastMinutes)}</td>
                    <td className="tnum px-4 py-2.5 text-right text-ink-400">{it.subIssues.length}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </div>

        {/* Custom fields the Project would carry */}
        <div>
          <h2 className="text-sm font-semibold text-ink-100">Project fields</h2>
          <p className="mb-3 mt-0.5 text-xs text-ink-300">
            The custom fields Bifrost sets — they drive the board, roadmap and Insights views.
          </p>
          <div className="space-y-2">
            {plan.fields.map((f) => (
              <div key={f.name} className="bf-card rounded-lg p-3">
                <div className="flex items-baseline justify-between">
                  <span className="text-sm font-medium text-ink-100">{f.name}</span>
                  <span className="font-mono text-[10px] uppercase tracking-wide text-ink-400">{f.dataType}</span>
                </div>
                {f.options.length > 0 && (
                  <div className="mt-2 flex flex-wrap gap-1.5">
                    {f.options.map((o) => (
                      <span key={o} className="rounded bg-ink-850 px-2 py-0.5 text-[11px] text-ink-300">
                        {o}
                      </span>
                    ))}
                  </div>
                )}
              </div>
            ))}
          </div>
        </div>
      </div>

      {/* Dry-run notes — what provisioning would do, and the guardrails */}
      <div className="mt-6 rounded-xl border border-ink-800 bg-ink-900/40 p-4">
        <div className="text-[10px] font-semibold uppercase tracking-[0.12em] text-ink-400">Before you provision</div>
        <ul className="mt-2 space-y-1.5 text-xs text-ink-300">
          {plan.notes.map((n) => (
            <li key={n} className="flex gap-2">
              <span className="mt-1.5 h-1 w-1 shrink-0 rounded-full bg-ink-500" />
              {n}
            </li>
          ))}
        </ul>
      </div>
    </main>
  )
}
