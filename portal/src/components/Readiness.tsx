import { useEffect, useState } from 'react'
import type { BifrostApi } from '../api/client'
import type { ReadinessItem, ReadinessStatus } from '../types'

const STATUS: Record<ReadinessStatus, { label: string; color: string; bg: string }> = {
  ready: { label: 'Ready', color: 'var(--color-risk-green)', bg: 'var(--color-risk-green-dim)' },
  action: { label: 'Action needed', color: 'var(--color-risk-amber)', bg: 'var(--color-risk-amber-dim)' },
  unverified: { label: 'Unverified', color: 'var(--color-ink-300)', bg: 'transparent' },
  blocked: { label: 'Blocked', color: 'var(--color-risk-red)', bg: 'var(--color-risk-red-dim)' },
}

function StatusChip({ status }: { status: ReadinessStatus }) {
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

export function Readiness({ api }: { api: BifrostApi }) {
  const [rows, setRows] = useState<ReadinessItem[] | null>(null)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    api.getReadiness().then(setRows).catch((e) => setError(String(e)))
  }, [api])

  if (error) {
    return <div className="flex flex-1 items-center justify-center text-[var(--color-risk-red)]">Failed to load readiness: {error}</div>
  }
  if (!rows) {
    return (
      <div className="flex flex-1 items-center justify-center text-ink-300">
        <div className="animate-pulse">Checking readiness…</div>
      </div>
    )
  }

  const tally = (s: ReadinessStatus) => rows.filter((r) => r.status === s).length
  const summary: ReadinessStatus[] = ['ready', 'action', 'unverified', 'blocked']

  return (
    <main className="mx-auto w-full max-w-7xl flex-1 px-6 py-6">
      <div className="mb-5">
        <h1 className="text-xl font-semibold text-ink-100">Readiness</h1>
        <p className="text-sm text-ink-300">
          Is the target GitHub org ready to receive the migration? Quantified action items come from the audit;
          operational gates Bifrost can&rsquo;t verify are flagged for you to confirm.
        </p>
      </div>

      <div className="mb-5 grid grid-cols-2 gap-3 sm:grid-cols-4">
        {summary.map((s) => (
          <div key={s} className="bf-card rounded-xl p-4">
            <div className="tnum font-display text-2xl font-semibold" style={{ color: STATUS[s].color }}>
              {tally(s)}
            </div>
            <div className="mt-1 text-xs text-ink-300">{STATUS[s].label}</div>
          </div>
        ))}
      </div>

      <div className="space-y-2">
        {rows.map((r) => (
          <div key={r.category} className="bf-card rounded-xl p-4">
            <div className="flex items-start justify-between gap-3">
              <div className="min-w-0">
                <div className="font-medium text-ink-100">{r.category}</div>
                <div className="mt-0.5 text-sm text-ink-300">{r.detail}</div>
                <div className="mt-1.5 text-xs text-ink-400">→ {r.action}</div>
              </div>
              <StatusChip status={r.status} />
            </div>
          </div>
        ))}
      </div>

      <p className="mt-4 text-xs text-ink-400">
        · &ldquo;Unverified&rdquo; means Bifrost cannot check this — confirm it in your GitHub org. Nothing here changes
        anything; it is a pre-flight checklist.
      </p>
    </main>
  )
}
