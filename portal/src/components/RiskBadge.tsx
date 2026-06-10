import type { RiskBand } from '../types'
import { riskMeta } from '../lib/format'

export function RiskBadge({ band, score }: { band: RiskBand; score?: number }) {
  const m = riskMeta[band]
  return (
    <span
      className={`inline-flex items-center gap-1.5 rounded-full px-2.5 py-0.5 text-xs font-medium ${m.bg} ${m.text}`}
    >
      <span className={`h-1.5 w-1.5 rounded-full ${m.dot}`} />
      {m.label}
      {score !== undefined && <span className="font-mono opacity-70">{score}</span>}
    </span>
  )
}
