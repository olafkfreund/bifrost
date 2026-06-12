import { useEffect, useState } from 'react'
import type { BifrostApi } from '../api/client'
import type { SourceStats } from '../types'

const num = (n: number) => n.toLocaleString()

function Metric({ label, value, sub }: { label: string; value: string | number; sub?: string }) {
  return (
    <div className="bf-card rounded-xl p-4">
      <div className="text-[11px] font-semibold uppercase tracking-[0.08em] text-ink-300">{label}</div>
      <div className="tnum mt-1.5 font-display text-2xl font-semibold tracking-tight text-ink-100">{value}</div>
      {sub && <div className="mt-1 text-xs text-ink-300">{sub}</div>}
    </div>
  )
}

export function Assessment({ api }: { api: BifrostApi }) {
  const [s, setS] = useState<SourceStats | null>(null)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    api.getSourceStats().then(setS).catch((e) => setError(String(e)))
  }, [api])

  if (error) {
    return <div className="flex flex-1 items-center justify-center text-[var(--color-risk-red)]">Failed to load assessment: {error}</div>
  }
  if (!s) {
    return (
      <div className="flex flex-1 items-center justify-center text-ink-300">
        <div className="animate-pulse">Assessing source…</div>
      </div>
    )
  }

  const density: { label: string; value: number }[] = [
    { label: 'Service connections', value: s.serviceConnections },
    { label: 'Variable groups', value: s.variableGroups },
    { label: 'Secrets', value: s.secrets },
    { label: 'Self-hosted runners', value: s.selfHostedRunners },
    { label: 'Custom task types', value: s.customTaskTypes },
    { label: 'Actions allow-list', value: s.actionsAllowlist },
  ]

  return (
    <main className="mx-auto w-full max-w-7xl flex-1 px-6 py-6">
      <div className="mb-5">
        <h1 className="text-xl font-semibold text-ink-100">Assessment</h1>
        <p className="text-sm text-ink-300">
          Status of the source (Azure DevOps org <span className="font-mono text-ink-200">{s.org}</span>) before any
          change — pipeline mix, risk, and the inventory you must account for in GitHub.
        </p>
      </div>

      {/* mix + risk */}
      <div className="grid grid-cols-2 gap-3 lg:grid-cols-4">
        <Metric label="Pipelines" value={num(s.pipelines)} sub={`${s.projects} projects · ${s.yaml} YAML · ${s.classic} classic`} />
        <div className="bf-card rounded-xl p-4">
          <div className="text-[11px] font-semibold uppercase tracking-[0.08em] text-ink-300">Risk</div>
          <div className="tnum mt-1.5 flex items-baseline gap-3 text-sm font-medium">
            <span className="text-[var(--color-risk-green)]">{s.green} green</span>
            <span className="text-[var(--color-risk-amber)]">{s.amber} amber</span>
            <span className="text-[var(--color-risk-red)]">{s.red} red</span>
          </div>
          <div className="mt-3 text-xs text-ink-300">{s.classic} classic pipelines are the hard tail</div>
        </div>
        <Metric label="Forecast minutes" value={`${num(s.forecastMinutes)}/mo`} sub="see Forecast for cost" />
        <Metric label="To recreate in GitHub" value={s.serviceConnections + s.variableGroups + s.secrets + s.selfHostedRunners} sub="connections + groups + secrets + runners" />
      </div>

      {/* inventory density */}
      <div className="mt-6">
        <h2 className="mb-2 text-sm font-semibold text-ink-100">Inventory density</h2>
        <div className="grid grid-cols-2 gap-3 sm:grid-cols-3 lg:grid-cols-6">
          {density.map((d) => (
            <div key={d.label} className="bf-card rounded-lg p-3">
              <div className="tnum font-display text-xl font-semibold text-ink-100">{num(d.value)}</div>
              <div className="mt-0.5 text-[11px] text-ink-300">{d.label}</div>
            </div>
          ))}
        </div>
      </div>

      {/* per-project */}
      <div className="mt-6">
        <h2 className="mb-2 text-sm font-semibold text-ink-100">By project</h2>
        <div className="bf-card overflow-hidden rounded-xl">
          <table className="w-full text-sm">
            <thead>
              <tr className="border-b border-ink-800 text-left text-xs uppercase tracking-wide text-ink-300">
                <th className="px-4 py-2.5 font-medium">Project</th>
                <th className="px-4 py-2.5 text-right font-medium">Pipelines</th>
                <th className="px-4 py-2.5 text-right font-medium">YAML</th>
                <th className="px-4 py-2.5 text-right font-medium">Classic</th>
                <th className="px-4 py-2.5 text-right font-medium">Red</th>
              </tr>
            </thead>
            <tbody>
              {s.byProject.map((p) => (
                <tr key={p.project} className="border-b border-ink-800 last:border-0">
                  <td className="px-4 py-2.5 text-ink-100">{p.project}</td>
                  <td className="tnum px-4 py-2.5 text-right text-ink-200">{num(p.pipelines)}</td>
                  <td className="tnum px-4 py-2.5 text-right text-ink-300">{num(p.yaml)}</td>
                  <td className="tnum px-4 py-2.5 text-right text-ink-300">{num(p.classic)}</td>
                  <td className="tnum px-4 py-2.5 text-right text-[var(--color-risk-red)]">{num(p.red)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </div>

      {/* honest gaps */}
      <div className="mt-6">
        <h2 className="mb-2 text-sm font-semibold text-ink-100">Not yet collected</h2>
        <div className="bf-card rounded-xl p-4">
          <p className="mb-2 text-xs text-ink-300">
            These assessment signals need ADO run-history and ownership data Bifrost does not gather yet — know they
            are still unmeasured before you start.
          </p>
          <ul className="space-y-1.5">
            {s.uncollected.map((u) => (
              <li key={u} className="flex gap-2 text-sm text-ink-200">
                <span className="text-[var(--color-risk-amber)]">○</span>
                {u}
              </li>
            ))}
          </ul>
        </div>
      </div>
    </main>
  )
}
