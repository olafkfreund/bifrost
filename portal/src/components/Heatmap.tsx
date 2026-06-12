import type { Pipeline } from '../types'
import { riskMeta } from '../lib/format'

function groupBy(pipelines: Pipeline[], key: (p: Pipeline) => string): [string, Pipeline[]][] {
  const map = new Map<string, Pipeline[]>()
  for (const p of pipelines) {
    const k = key(p)
    const arr = map.get(k) ?? []
    arr.push(p)
    map.set(k, arr)
  }
  return [...map.entries()].sort((a, b) => a[0].localeCompare(b[0]))
}

const groupByProject = (pipelines: Pipeline[]) => groupBy(pipelines, (p) => p.project)

function Tile({ p, onSelect }: { p: Pipeline; onSelect: (p: Pipeline) => void }) {
  const m = riskMeta[p.riskBand]
  return (
    <button
      onClick={() => onSelect(p)}
      title={`${p.name} — ${m.label}: risk ${p.riskScore} · ${Math.round(p.convertedRatio * 100)}% converted by the Importer`}
      className={`group relative flex h-20 flex-col justify-between rounded-lg border border-ink-800 p-2.5 text-left shadow-[var(--elevation-card)] transition
        hover:-translate-y-0.5 hover:border-ink-600 hover:ring-2 ${m.ring} ${m.bg}`}
    >
      <div className="flex items-center justify-between">
        <span className={`h-2 w-2 rounded-full ${m.dot}`} />
        {p.classification === 'classic' && (
          <span className="rounded bg-ink-900/70 px-1 text-[10px] font-medium uppercase text-ink-300">classic</span>
        )}
      </div>
      <div className="truncate text-xs font-medium text-ink-100">{p.name.split(' · ')[1] ?? p.name}</div>
      <div className="tnum font-mono text-[10px] text-ink-300">
        <span title="Deterministic risk score (0–100)">risk {p.riskScore}</span>
        <span className="text-ink-500"> · </span>
        <span title="Share converted automatically by the Importer">{Math.round(p.convertedRatio * 100)}%</span>
      </div>
    </button>
  )
}

function ProjectGroups({
  pipelines,
  onSelect,
}: {
  pipelines: Pipeline[]
  onSelect: (p: Pipeline) => void
}) {
  return (
    <div className="space-y-5">
      {groupByProject(pipelines).map(([project, items]) => (
        <div key={project}>
          <div className="mb-2 flex items-center gap-2">
            <h3 className="text-sm font-semibold text-ink-100">{project}</h3>
            <span className="text-xs text-ink-300">{items.length}</span>
          </div>
          <div className="grid grid-cols-2 gap-2 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-6">
            {items.map((p) => (
              <Tile key={p.id} p={p} onSelect={onSelect} />
            ))}
          </div>
        </div>
      ))}
    </div>
  )
}

export function Heatmap({
  pipelines,
  onSelect,
}: {
  pipelines: Pipeline[]
  onSelect: (p: Pipeline) => void
}) {
  // Multi-org (#157): when the view spans more than one source org, group by
  // org → project; otherwise keep the flat by-project view.
  const orgs = groupBy(pipelines, (p) => p.org ?? '')
  const multiOrg = orgs.filter(([o]) => o !== '').length > 1
  if (!multiOrg) {
    return <ProjectGroups pipelines={pipelines} onSelect={onSelect} />
  }
  return (
    <div className="space-y-8">
      {orgs.map(([org, items]) => (
        <section key={org || '—'}>
          <div className="mb-3 flex items-center gap-2 border-b border-ink-800 pb-1.5">
            <h2 className="text-sm font-semibold uppercase tracking-wide text-ink-200">
              {org || 'Unassigned org'}
            </h2>
            <span className="rounded bg-ink-850 px-1.5 text-[10px] font-medium text-ink-300">
              {items.length} pipelines
            </span>
          </div>
          <ProjectGroups pipelines={items} onSelect={onSelect} />
        </section>
      ))}
    </div>
  )
}
