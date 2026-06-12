import type { PortfolioSummary } from '../types'
import { minutes } from '../lib/format'

function Card({
  label,
  value,
  sub,
  accent,
}: {
  label: string
  value: React.ReactNode
  sub?: React.ReactNode
  accent?: string
}) {
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

/** Stacked Green/Amber/Red bar summarising portfolio risk. */
function RiskBar({ green, amber, red }: { green: number; amber: number; red: number }) {
  const total = Math.max(green + amber + red, 1)
  const seg = (n: number, color: string) =>
    n > 0 && <div className={color} style={{ width: `${(n / total) * 100}%` }} title={`${n}`} />
  return (
    <div className="mt-2 flex h-2 w-full overflow-hidden rounded-full bg-ink-800">
      {seg(green, 'bg-[var(--color-risk-green)]')}
      {seg(amber, 'bg-[var(--color-risk-amber)]')}
      {seg(red, 'bg-[var(--color-risk-red)]')}
    </div>
  )
}

export function StatCards({ summary }: { summary: PortfolioSummary }) {
  const t = summary.totals
  return (
    <div className="grid grid-cols-2 gap-3 lg:grid-cols-4">
      <Card
        label="Pipelines"
        value={t.pipelines}
        sub={`${t.orgs && t.orgs > 1 ? `${t.orgs} orgs · ` : ''}${t.projects} projects · ${t.yaml} YAML · ${t.classic} classic`}
      />
      <div className="bf-card rounded-xl p-4">
        <div className="text-[11px] font-semibold uppercase tracking-[0.08em] text-ink-300">Risk profile</div>
        <div className="tnum mt-1.5 flex items-baseline gap-3 text-sm font-medium">
          <span className="text-[var(--color-risk-green)]">{t.green} green</span>
          <span className="text-[var(--color-risk-amber)]">{t.amber} amber</span>
          <span className="text-[var(--color-risk-red)]">{t.red} red</span>
        </div>
        <RiskBar green={t.green} amber={t.amber} red={t.red} />
      </div>
      <Card
        label="Mechanical (green)"
        value={`${Math.round((t.green / Math.max(t.pipelines, 1)) * 100)}%`}
        sub="low-risk, ready to convert"
        accent="text-[var(--color-risk-green)]"
      />
      <Card
        label="Forecast runner-minutes"
        value={`${minutes(t.forecastMinutes)}/mo`}
        sub="projected GitHub Actions usage"
      />
    </div>
  )
}
