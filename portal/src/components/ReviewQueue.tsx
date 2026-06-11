import { useEffect, useMemo, useRef, useState } from 'react'
import type { JobProgress, Pipeline, ProposalStatus, RiskBand } from '../types'
import type { BifrostApi } from '../api/client'
import { statusLabel, riskMeta } from '../lib/format'
import { RiskBadge } from './RiskBadge'

// Lifecycle order, left → right, matching the state machine.
const STATUS_ORDER: ProposalStatus[] = [
  'not_started',
  'draft',
  'in_review',
  'changes_requested',
  'approved',
  'committed',
  'validated',
]

const STATUS_COLOR: Record<ProposalStatus, string> = {
  not_started: 'var(--color-ink-700)',
  draft: 'var(--color-ink-500)',
  in_review: 'var(--color-brand-400)',
  changes_requested: 'var(--color-risk-amber)',
  approved: 'var(--color-accent-aqua)',
  committed: 'var(--color-brand-500)',
  validated: 'var(--color-risk-green)',
}

function when(iso?: string): string {
  if (!iso) return '—'
  const d = new Date(iso)
  return Number.isNaN(d.getTime()) ? '—' : d.toLocaleDateString(undefined, { month: 'short', day: 'numeric' })
}

/**
 * The review queue: a portfolio-wide list of proposals by lifecycle state, with a
 * progress bar across the state machine and filters by status + risk. Each row
 * shows who last acted and when; clicking opens the proposal for review.
 */
export function ReviewQueue({
  pipelines,
  api,
  onSelect,
  onRefresh,
}: {
  pipelines: Pipeline[]
  api: BifrostApi
  onSelect: (p: Pipeline) => void
  onRefresh: () => void
}) {
  const [statusFilter, setStatusFilter] = useState<ProposalStatus | 'all'>('all')
  const [riskFilter, setRiskFilter] = useState<RiskBand | 'all'>('all')
  const [job, setJob] = useState<JobProgress | null>(null)
  const unsub = useRef<(() => void) | null>(null)

  useEffect(() => () => unsub.current?.(), [])

  const notStarted = pipelines.filter((p) => p.status === 'not_started').length
  const running = job != null && !job.finished

  async function convertAll() {
    if (notStarted === 0 || running) return
    const ids = pipelines.filter((p) => p.status === 'not_started').map((p) => p.id)
    const { jobId } = await api.startConvertJob(ids)
    unsub.current?.()
    unsub.current = api.subscribeJob(jobId, (p) => {
      setJob(p)
      if (p.finished) {
        unsub.current?.()
        unsub.current = null
        onRefresh()
        // Clear the bar once the refreshed queue paints the new statuses.
        setTimeout(() => setJob(null), 800)
      }
    })
  }

  const total = pipelines.length
  const counts = useMemo(() => {
    const c = Object.fromEntries(STATUS_ORDER.map((s) => [s, 0])) as Record<ProposalStatus, number>
    for (const p of pipelines) c[p.status] = (c[p.status] ?? 0) + 1
    return c
  }, [pipelines])

  const awaiting = counts.in_review + counts.changes_requested
  const segments = STATUS_ORDER.filter((s) => counts[s] > 0)

  const rows = useMemo(
    () =>
      pipelines
        .filter((p) => statusFilter === 'all' || p.status === statusFilter)
        .filter((p) => riskFilter === 'all' || p.riskBand === riskFilter)
        .sort(
          (a, b) =>
            STATUS_ORDER.indexOf(a.status) - STATUS_ORDER.indexOf(b.status) || b.riskScore - a.riskScore,
        ),
    [pipelines, statusFilter, riskFilter],
  )

  return (
    <main className="mx-auto w-full max-w-7xl flex-1 px-6 py-6">
      <div className="mb-5 flex flex-wrap items-end justify-between gap-3">
        <div>
          <h1 className="text-xl font-semibold text-ink-100">Review queue</h1>
          <p className="text-sm text-ink-300">
            {awaiting} awaiting review · {counts.approved} approved · {total} pipelines total
          </p>
        </div>
        <div className="flex items-center gap-2">
          <button
            onClick={convertAll}
            disabled={notStarted === 0 || running}
            title={notStarted === 0 ? 'Nothing left to convert' : `Convert ${notStarted} not-started pipelines`}
            className="rounded-lg bg-brand-500 px-3 py-1.5 text-xs font-medium text-ink-950 transition hover:bg-brand-400 disabled:cursor-not-allowed disabled:opacity-50"
          >
            {running ? `Converting ${job!.done}/${job!.total}…` : `Convert all (${notStarted})`}
          </button>
          <div className="flex overflow-hidden rounded-lg border border-ink-800 text-xs">
            {(['all', 'green', 'amber', 'red'] as const).map((b) => (
              <button
                key={b}
                onClick={() => setRiskFilter(b)}
                className={`px-3 py-1.5 capitalize transition ${
                  riskFilter === b ? 'bg-ink-800 text-ink-100' : 'text-ink-300 hover:bg-ink-850'
                } ${b !== 'all' ? riskMeta[b as RiskBand].text : ''}`}
              >
                {b}
              </button>
            ))}
          </div>
        </div>
      </div>

      {/* job progress bar (visible while a conversion job runs) */}
      {job != null && (
        <div className="mb-4 rounded-xl border border-ink-800 bg-ink-900/40 p-4">
          <div className="mb-2 flex items-center justify-between text-xs">
            <span className="font-medium text-ink-200">
              {job.finished ? 'Conversion complete' : 'Converting pipelines…'}
            </span>
            <span className="font-mono text-ink-300">
              {job.done}/{job.total}
            </span>
          </div>
          <div className="h-2 overflow-hidden rounded-full bg-ink-800">
            <div
              className="h-full rounded-full bg-brand-400 transition-all duration-300"
              style={{ width: `${job.total ? (job.done / job.total) * 100 : 0}%` }}
            />
          </div>
        </div>
      )}

      {/* lifecycle progress bar */}
      <div className="rounded-xl border border-ink-800 bg-ink-900/40 p-4">
        <div className="mb-3 flex items-center justify-between">
          <span className="text-xs font-medium uppercase tracking-wide text-ink-300">Migration progress</span>
          {statusFilter !== 'all' && (
            <button onClick={() => setStatusFilter('all')} className="text-xs text-ink-300 hover:text-ink-100">
              clear filter ✕
            </button>
          )}
        </div>
        <div className="flex h-2.5 overflow-hidden rounded-full bg-ink-800">
          {segments.map((s) => (
            <div
              key={s}
              title={`${statusLabel[s]}: ${counts[s]}`}
              style={{ width: `${(counts[s] / total) * 100}%`, backgroundColor: STATUS_COLOR[s] }}
            />
          ))}
        </div>
        <div className="mt-3 flex flex-wrap gap-x-4 gap-y-1.5 text-xs">
          {STATUS_ORDER.map((s) => (
            <button
              key={s}
              onClick={() => setStatusFilter(statusFilter === s ? 'all' : s)}
              className={`flex items-center gap-1.5 rounded px-1.5 py-0.5 transition ${
                statusFilter === s ? 'bg-ink-800 text-ink-100' : 'text-ink-300 hover:text-ink-100'
              } ${counts[s] === 0 ? 'opacity-40' : ''}`}
            >
              <span className="h-2 w-2 rounded-full" style={{ backgroundColor: STATUS_COLOR[s] }} />
              {statusLabel[s]} <span className="font-mono text-ink-500">{counts[s]}</span>
            </button>
          ))}
        </div>
      </div>

      {/* queue table */}
      <div className="mt-6 overflow-hidden rounded-xl border border-ink-800">
        <table className="w-full text-sm">
          <thead className="bg-ink-900/60 text-left text-xs uppercase tracking-wide text-ink-300">
            <tr>
              <th className="px-4 py-2.5 font-medium">Pipeline</th>
              <th className="px-4 py-2.5 font-medium">Status</th>
              <th className="px-4 py-2.5 font-medium">Risk</th>
              <th className="px-4 py-2.5 font-medium">Converted</th>
              <th className="px-4 py-2.5 font-medium">Last action</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((p) => (
              <tr
                key={p.id}
                onClick={() => onSelect(p)}
                className="cursor-pointer border-t border-ink-800 transition hover:bg-ink-850"
              >
                <td className="px-4 py-3">
                  <div className="font-medium text-ink-100">{p.name}</div>
                  <div className="text-xs text-ink-300">
                    {p.project}
                    {p.classification === 'classic' && (
                      <span className="ml-2 rounded bg-ink-800 px-1 text-[10px] uppercase text-ink-300">classic</span>
                    )}
                  </div>
                </td>
                <td className="px-4 py-3">
                  <span className="inline-flex items-center gap-1.5 text-ink-200">
                    <span className="h-2 w-2 rounded-full" style={{ backgroundColor: STATUS_COLOR[p.status] }} />
                    {statusLabel[p.status]}
                  </span>
                </td>
                <td className="px-4 py-3">
                  <RiskBadge band={p.riskBand} score={p.riskScore} />
                </td>
                <td className="px-4 py-3 font-mono text-ink-300">{Math.round(p.convertedRatio * 100)}%</td>
                <td className="px-4 py-3 text-xs text-ink-300">
                  {p.reviewer ? (
                    <>
                      <span className="text-ink-200">{p.reviewer}</span> · {when(p.reviewedAt)}
                    </>
                  ) : (
                    <span className="text-ink-500">no action yet</span>
                  )}
                </td>
              </tr>
            ))}
            {rows.length === 0 && (
              <tr>
                <td colSpan={5} className="px-4 py-8 text-center text-sm text-ink-300">
                  No pipelines match this filter.
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </div>
    </main>
  )
}
