import type { Pipeline } from '../types'
import { riskMeta } from '../lib/format'

function groupByProject(pipelines: Pipeline[]): [string, Pipeline[]][] {
  const map = new Map<string, Pipeline[]>()
  for (const p of pipelines) {
    const arr = map.get(p.project) ?? []
    arr.push(p)
    map.set(p.project, arr)
  }
  return [...map.entries()].sort((a, b) => a[0].localeCompare(b[0]))
}

function Tile({ p, onSelect }: { p: Pipeline; onSelect: (p: Pipeline) => void }) {
  const m = riskMeta[p.riskBand]
  return (
    <button
      onClick={() => onSelect(p)}
      title={`${p.name} — ${m.label} (${p.riskScore})`}
      className={`group relative flex h-20 flex-col justify-between rounded-lg border border-ink-800 p-2.5 text-left transition
        hover:-translate-y-0.5 hover:border-ink-600 hover:ring-2 ${m.ring} ${m.bg}`}
    >
      <div className="flex items-center justify-between">
        <span className={`h-2 w-2 rounded-full ${m.dot}`} />
        {p.classification === 'classic' && (
          <span className="rounded bg-ink-900/70 px-1 text-[10px] font-medium uppercase text-ink-300">classic</span>
        )}
      </div>
      <div className="truncate text-xs font-medium text-ink-100">{p.name.split(' · ')[1] ?? p.name}</div>
      <div className="font-mono text-[10px] text-ink-300">{p.riskScore} · {Math.round(p.convertedRatio * 100)}%</div>
    </button>
  )
}

export function Heatmap({
  pipelines,
  onSelect,
}: {
  pipelines: Pipeline[]
  onSelect: (p: Pipeline) => void
}) {
  const groups = groupByProject(pipelines)
  return (
    <div className="space-y-5">
      {groups.map(([project, items]) => (
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
