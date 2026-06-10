import type { Pipeline } from '../types'
import { riskMeta, statusLabel, pct } from '../lib/format'
import { RiskBadge } from './RiskBadge'

function Stat({ label, value }: { label: string; value: React.ReactNode }) {
  return (
    <div className="rounded-lg border border-ink-800 bg-ink-850 p-3">
      <div className="text-[11px] uppercase tracking-wide text-ink-300">{label}</div>
      <div className="mt-0.5 text-lg font-semibold text-ink-100">{value}</div>
    </div>
  )
}

export function PipelineDrawer({ pipeline, onClose }: { pipeline: Pipeline | null; onClose: () => void }) {
  if (!pipeline) return null
  const p = pipeline
  const maxContribution = Math.max(...p.factors.map((f) => f.contribution), 1)

  return (
    <div className="fixed inset-0 z-40 flex justify-end">
      <div className="absolute inset-0 bg-black/50" onClick={onClose} />
      <aside className="relative z-50 flex h-full w-full max-w-md flex-col border-l border-ink-800 bg-ink-900 shadow-2xl">
        <div className="flex items-start justify-between border-b border-ink-800 p-5">
          <div>
            <div className="text-xs text-ink-300">{p.project}</div>
            <h2 className="text-lg font-semibold text-ink-100">{p.name}</h2>
            <div className="mt-2 flex items-center gap-2">
              <RiskBadge band={p.riskBand} score={p.riskScore} />
              <span className="rounded-full bg-ink-800 px-2 py-0.5 text-xs text-ink-300">
                {statusLabel[p.status]}
              </span>
              <span className="rounded-full bg-ink-800 px-2 py-0.5 text-xs text-ink-300">
                {p.classification}
              </span>
            </div>
          </div>
          <button
            onClick={onClose}
            className="rounded-md p-1 text-ink-300 hover:bg-ink-800 hover:text-ink-100"
            aria-label="Close"
          >
            ✕
          </button>
        </div>

        <div className="flex-1 overflow-y-auto p-5">
          <div className="grid grid-cols-3 gap-2">
            <Stat label="Converted" value={pct(p.convertedRatio)} />
            <Stat label="Unsupported" value={p.unsupportedSteps} />
            <Stat label="Manual tasks" value={p.manualTasks} />
          </div>

          <div className="mt-6">
            <div className="flex items-center justify-between">
              <h3 className="text-sm font-semibold text-ink-100">Risk factors</h3>
              <span className="text-xs text-ink-300">deterministic · score {p.riskScore}</span>
            </div>
            <p className="mt-1 text-xs text-ink-300">
              The score is computed from these weighted factors — never from the LLM.
            </p>
            <ul className="mt-3 space-y-3">
              {p.factors.map((f) => (
                <li key={f.key}>
                  <div className="flex items-center justify-between text-sm">
                    <span className="font-medium text-ink-100">{f.label}</span>
                    <span className="font-mono text-xs text-ink-300">+{f.contribution}</span>
                  </div>
                  <div className="mt-1 h-1.5 w-full overflow-hidden rounded-full bg-ink-800">
                    <div
                      className={`h-full ${riskMeta[p.riskBand].dot}`}
                      style={{ width: `${(f.contribution / maxContribution) * 100}%` }}
                    />
                  </div>
                  <p className="mt-1 text-xs text-ink-300">{f.detail}</p>
                </li>
              ))}
            </ul>
          </div>
        </div>

        <div className="border-t border-ink-800 p-4">
          <button
            disabled
            title="Conversion + review lands in M2/M3"
            className="w-full cursor-not-allowed rounded-lg border border-ink-700 bg-ink-850 px-4 py-2 text-sm font-medium text-ink-300"
          >
            Open proposal · coming in M3
          </button>
        </div>
      </aside>
    </div>
  )
}
