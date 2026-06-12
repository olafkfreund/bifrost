import { useEffect, useState } from 'react'
import type { BifrostApi } from '../api/client'
import type { Forecast as ForecastData } from '../types'

const usd = (n: number) =>
  n.toLocaleString(undefined, { style: 'currency', currency: 'USD', maximumFractionDigits: n < 100 ? 2 : 0 })
const num = (n: number) => n.toLocaleString()

function Metric({ label, value, sub, accent }: { label: string; value: string; sub?: string; accent?: string }) {
  return (
    <div className="bf-card rounded-xl p-4">
      <div className="text-[11px] font-semibold uppercase tracking-[0.08em] text-ink-300">{label}</div>
      <div className={`tnum mt-1.5 font-display text-2xl font-semibold tracking-tight ${accent ?? 'text-ink-100'}`}>
        {value}
      </div>
      {sub && <div className="mt-1 text-xs text-ink-300">{sub}</div>}
    </div>
  )
}

export function Forecast({ api }: { api: BifrostApi }) {
  const [data, setData] = useState<ForecastData | null>(null)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    api.getForecast().then(setData).catch((e) => setError(String(e)))
  }, [api])

  if (error) {
    return <div className="flex flex-1 items-center justify-center text-[var(--color-risk-red)]">Failed to load forecast: {error}</div>
  }
  if (!data) {
    return (
      <div className="flex flex-1 items-center justify-center text-ink-300">
        <div className="animate-pulse">Computing forecast…</div>
      </div>
    )
  }

  const cap = data.capacity

  return (
    <main className="mx-auto w-full max-w-7xl flex-1 px-6 py-6">
      <div className="mb-5">
        <h1 className="text-xl font-semibold text-ink-100">Forecast</h1>
        <p className="text-sm text-ink-300">
          Projected GitHub Actions cost and capacity for the target org · {data.runnerClass} · computed
          deterministically from the audit
        </p>
      </div>

      {/* headline cost + scale */}
      <div className="grid grid-cols-2 gap-3 lg:grid-cols-4">
        <Metric label="Projected monthly cost" value={usd(data.monthlyCostUsd)} sub={`${usd(data.annualCostUsd)} / year`} accent="text-accent-aqua" />
        <Metric label="Runner-minutes / month" value={num(data.totalMinutes)} sub="across all pipelines" />
        <Metric label="Rate" value={`$${data.usdPerMinute.toFixed(3)}/min`} sub={data.runnerClass} />
        <Metric
          label="Peak concurrency"
          value={cap ? num(cap.peakConcurrency) : '—'}
          sub={cap ? 'simultaneous jobs (sizes the runner pool)' : 'run a live forecast to populate'}
        />
      </div>

      {/* capacity from the Importer forecast (run-history based) */}
      <div className="mt-6">
        <h2 className="mb-2 text-sm font-semibold text-ink-100">Capacity</h2>
        {cap ? (
          <div className="grid grid-cols-2 gap-3 sm:grid-cols-4">
            <Metric label="Median queue" value={`${cap.medianQueueMinutes.toFixed(1)} min`} sub="job wait for a runner" />
            <Metric label="p50 job" value={`${cap.p50JobMinutes.toFixed(1)} min`} />
            <Metric label="p90 job" value={`${cap.p90JobMinutes.toFixed(1)} min`} />
            <Metric label="Longest job" value={`${cap.maxJobMinutes.toFixed(1)} min`} />
          </div>
        ) : (
          <div className="bf-card rounded-xl p-4 text-sm text-ink-300">
            Concurrency, queue time, and job-duration percentiles come from real run history via{' '}
            <span className="font-mono text-ink-200">gh actions-importer forecast</span>. Run a live forecast to
            populate them — cost above is already computed from the audit.
          </div>
        )}
      </div>

      {/* per-project breakdown */}
      <div className="mt-6">
        <h2 className="mb-2 text-sm font-semibold text-ink-100">Cost by project</h2>
        <div className="bf-card overflow-hidden rounded-xl">
          <table className="w-full text-sm">
            <thead>
              <tr className="border-b border-ink-800 text-left text-xs uppercase tracking-wide text-ink-300">
                <th className="px-4 py-2.5 font-medium">Project</th>
                <th className="px-4 py-2.5 text-right font-medium">Pipelines</th>
                <th className="px-4 py-2.5 text-right font-medium">Minutes / mo</th>
                <th className="px-4 py-2.5 text-right font-medium">Cost / mo</th>
              </tr>
            </thead>
            <tbody>
              {data.byProject.map((p) => (
                <tr key={p.project} className="border-b border-ink-800 last:border-0">
                  <td className="px-4 py-2.5 text-ink-100">{p.project}</td>
                  <td className="tnum px-4 py-2.5 text-right text-ink-300">{num(p.pipelines)}</td>
                  <td className="tnum px-4 py-2.5 text-right text-ink-300">{num(p.minutes)}</td>
                  <td className="tnum px-4 py-2.5 text-right font-medium text-ink-100">{usd(p.costUsd)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </div>

      {/* assumptions */}
      <div className="mt-4 space-y-1 text-xs text-ink-400">
        {data.notes.map((n, i) => (
          <p key={i}>· {n}</p>
        ))}
      </div>
    </main>
  )
}
