import { useEffect, useState } from 'react'
import type { BifrostApi } from '../api/client'
import type { CategoryStatus, CompletenessRow } from '../types'

/** Status -> label + colour. `notInventoried` is the honest "go look in ADO" flag. */
const STATUS: Record<CategoryStatus, { label: string; color: string; bg: string }> = {
  auto: { label: 'Automatic', color: 'var(--color-risk-green)', bg: 'var(--color-risk-green-dim)' },
  review: { label: 'Review', color: 'var(--color-risk-amber)', bg: 'var(--color-risk-amber-dim)' },
  manual: { label: 'Manual setup', color: 'var(--color-accent-aqua)', bg: 'var(--color-accent-aqua-dim)' },
  notInventoried: { label: 'Not yet inventoried', color: 'var(--color-risk-red)', bg: 'var(--color-risk-red-dim)' },
  notApplicable: { label: 'None found', color: 'var(--color-ink-500)', bg: 'transparent' },
}

function StatusChip({ status }: { status: CategoryStatus }) {
  const s = STATUS[status]
  return (
    <span
      className="inline-flex items-center gap-1.5 whitespace-nowrap rounded-full px-2.5 py-1 text-xs font-medium"
      style={{ color: s.color, backgroundColor: s.bg }}
    >
      <span className="h-1.5 w-1.5 rounded-full" style={{ backgroundColor: s.color }} />
      {s.label}
    </span>
  )
}

export function Completeness({ api }: { api: BifrostApi }) {
  const [rows, setRows] = useState<CompletenessRow[] | null>(null)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    api.getCompleteness().then(setRows).catch((e) => setError(String(e)))
  }, [api])

  if (error) {
    return <div className="flex flex-1 items-center justify-center text-[var(--color-risk-red)]">Failed to load coverage: {error}</div>
  }
  if (!rows) {
    return (
      <div className="flex flex-1 items-center justify-center text-ink-300">
        <div className="animate-pulse">Building coverage matrix…</div>
      </div>
    )
  }

  const tally = (s: CategoryStatus) => rows.filter((r) => r.status === s).length
  const summary: { status: CategoryStatus; n: number }[] = [
    { status: 'auto', n: tally('auto') },
    { status: 'review', n: tally('review') },
    { status: 'manual', n: tally('manual') },
    { status: 'notInventoried', n: tally('notInventoried') },
  ]

  return (
    <main className="mx-auto w-full max-w-7xl flex-1 px-6 py-6">
      <div className="mb-5">
        <h1 className="text-xl font-semibold text-ink-100">Coverage</h1>
        <p className="text-sm text-ink-300">
          Every Azure DevOps moving part, mapped to its GitHub equivalent and status — so nothing is
          left behind. Categories Bifrost cannot yet inventory are flagged, never hidden.
        </p>
      </div>

      {/* status tally */}
      <div className="mb-5 grid grid-cols-2 gap-3 sm:grid-cols-4">
        {summary.map(({ status, n }) => (
          <div key={status} className="bf-card rounded-xl p-4">
            <div className="tnum font-display text-2xl font-semibold" style={{ color: STATUS[status].color }}>
              {n}
            </div>
            <div className="mt-1 text-xs text-ink-300">{STATUS[status].label}</div>
          </div>
        ))}
      </div>

      {/* the matrix */}
      <div className="bf-card overflow-hidden rounded-xl">
        <table className="w-full text-sm">
          <thead>
            <tr className="border-b border-ink-800 text-left text-xs uppercase tracking-wide text-ink-300">
              <th className="px-4 py-2.5 font-medium">Moving part</th>
              <th className="px-4 py-2.5 text-right font-medium">Found</th>
              <th className="px-4 py-2.5 font-medium">GitHub equivalent</th>
              <th className="px-4 py-2.5 font-medium">Status</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((r) => (
              <tr key={r.category} className="border-b border-ink-800 align-top last:border-0">
                <td className="px-4 py-3">
                  <div className="font-medium text-ink-100">{r.category}</div>
                  <div className="mt-0.5 text-xs text-ink-400">{r.note}</div>
                </td>
                <td className="tnum px-4 py-3 text-right text-ink-200">
                  {r.inventoried ? r.count : <span className="text-ink-500" title="Not yet inventoried">—</span>}
                </td>
                <td className="px-4 py-3 text-ink-300">{r.githubEquivalent}</td>
                <td className="px-4 py-3">
                  <StatusChip status={r.status} />
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>

      <p className="mt-4 text-xs text-ink-400">
        · &ldquo;Not yet inventoried&rdquo; means Bifrost does not enumerate that category yet — check it in
        Azure DevOps. Secret values are never read; only names and types are recorded.
      </p>
    </main>
  )
}
