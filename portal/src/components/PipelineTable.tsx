import type { Pipeline } from '../types'
import { RiskBadge } from './RiskBadge'
import { statusLabel, pct, minutes } from '../lib/format'

export function PipelineTable({
  pipelines,
  onSelect,
}: {
  pipelines: Pipeline[]
  onSelect: (p: Pipeline) => void
}) {
  const rows = [...pipelines].sort((a, b) => b.riskScore - a.riskScore)
  return (
    <div className="overflow-hidden rounded-xl border border-ink-800">
      <table className="w-full text-left text-sm">
        <thead className="bg-ink-900 text-xs uppercase tracking-wide text-ink-300">
          <tr>
            <th className="px-4 py-2.5 font-medium">Pipeline</th>
            <th className="px-4 py-2.5 font-medium">Type</th>
            <th className="px-4 py-2.5 font-medium">Risk</th>
            <th className="hidden px-4 py-2.5 font-medium md:table-cell">Converted</th>
            <th className="hidden px-4 py-2.5 font-medium lg:table-cell">Gaps</th>
            <th className="hidden px-4 py-2.5 font-medium sm:table-cell">Status</th>
            <th className="hidden px-4 py-2.5 text-right font-medium lg:table-cell">min/mo</th>
          </tr>
        </thead>
        <tbody className="divide-y divide-ink-800">
          {rows.map((p) => (
            <tr
              key={p.id}
              onClick={() => onSelect(p)}
              className="cursor-pointer bg-ink-900/30 hover:bg-ink-850"
            >
              <td className="px-4 py-2.5">
                <div className="font-medium text-ink-100">{p.name}</div>
                <div className="text-xs text-ink-300">{p.project}</div>
              </td>
              <td className="px-4 py-2.5">
                <span
                  className={`rounded px-1.5 py-0.5 text-xs ${
                    p.classification === 'classic'
                      ? 'bg-[var(--color-risk-amber-dim)] text-[var(--color-risk-amber)]'
                      : 'bg-ink-800 text-ink-300'
                  }`}
                >
                  {p.classification}
                </span>
              </td>
              <td className="px-4 py-2.5">
                <RiskBadge band={p.riskBand} score={p.riskScore} />
              </td>
              <td className="hidden px-4 py-2.5 font-mono text-ink-200 md:table-cell">{pct(p.convertedRatio)}</td>
              <td className="hidden px-4 py-2.5 text-ink-300 lg:table-cell">
                {p.unsupportedSteps} unsupported · {p.manualTasks} manual
              </td>
              <td className="hidden px-4 py-2.5 text-ink-300 sm:table-cell">{statusLabel[p.status]}</td>
              <td className="hidden px-4 py-2.5 text-right font-mono text-ink-300 lg:table-cell">
                {minutes(p.forecastMinutes)}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  )
}
