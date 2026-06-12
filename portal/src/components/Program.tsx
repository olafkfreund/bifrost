import { useEffect, useState } from 'react'
import type { BifrostApi } from '../api/client'
import type { WavePlan } from '../types'

const num = (n: number) => n.toLocaleString()

/** Done / in-progress / not-started progress bar for a wave. */
function Progress({ done, inProgress, notStarted }: { done: number; inProgress: number; notStarted: number }) {
  const total = Math.max(done + inProgress + notStarted, 1)
  const seg = (n: number, color: string, title: string) =>
    n > 0 && <div className={color} style={{ width: `${(n / total) * 100}%` }} title={`${title}: ${n}`} />
  return (
    <div className="mt-3 flex h-2 w-full overflow-hidden rounded-full bg-ink-800">
      {seg(done, 'bg-[var(--color-risk-green)]', 'done')}
      {seg(inProgress, 'bg-[var(--color-brand-400)]', 'in progress')}
      {seg(notStarted, 'bg-ink-700', 'not started')}
    </div>
  )
}

export function Program({ api }: { api: BifrostApi }) {
  const [waves, setWaves] = useState<WavePlan[] | null>(null)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    api.getProgram().then(setWaves).catch((e) => setError(String(e)))
  }, [api])

  if (error) {
    return <div className="flex flex-1 items-center justify-center text-[var(--color-risk-red)]">Failed to load program: {error}</div>
  }
  if (!waves) {
    return (
      <div className="flex flex-1 items-center justify-center text-ink-300">
        <div className="animate-pulse">Planning waves…</div>
      </div>
    )
  }

  const totalPipelines = waves.reduce((s, w) => s + w.pipelines, 0)
  const totalDone = waves.reduce((s, w) => s + w.done, 0)

  return (
    <main className="mx-auto w-full max-w-7xl flex-1 px-6 py-6">
      <div className="mb-5 flex flex-wrap items-end justify-between gap-3">
        <div>
          <h1 className="text-xl font-semibold text-ink-100">Program</h1>
          <p className="text-sm text-ink-300">
            A phased migration: pilot the easy pipelines, prove the process, then roll out in waves — the hard tail last.
          </p>
        </div>
        <div className="tnum text-sm text-ink-300">
          {num(totalDone)} / {num(totalPipelines)} pipelines done
        </div>
      </div>

      <div className="grid gap-4 lg:grid-cols-3">
        {waves.map((w) => (
          <div key={w.wave} className="bf-card flex flex-col rounded-xl p-5">
            <div className="flex items-baseline justify-between">
              <h2 className="font-display text-lg font-semibold text-ink-100">
                Wave {w.wave} · {w.name}
              </h2>
              <span className="tnum font-display text-2xl font-semibold text-ink-100">{num(w.pipelines)}</span>
            </div>
            <p className="mt-1 text-xs text-ink-300">{w.rationale}</p>

            {/* risk mix */}
            <div className="tnum mt-4 flex items-baseline gap-3 text-sm font-medium">
              <span className="text-[var(--color-risk-green)]">{w.green} green</span>
              <span className="text-[var(--color-risk-amber)]">{w.amber} amber</span>
              <span className="text-[var(--color-risk-red)]">{w.red} red</span>
            </div>
            <div className="mt-1 text-xs text-ink-400">
              {w.yaml} YAML · {w.classic} classic · {num(w.forecastMinutes)} min/mo
            </div>

            {/* progress */}
            <Progress done={w.done} inProgress={w.inProgress} notStarted={w.notStarted} />
            <div className="tnum mt-1.5 flex justify-between text-[11px] text-ink-400">
              <span className="text-[var(--color-risk-green)]">{w.done} done</span>
              <span className="text-[var(--color-brand-400)]">{w.inProgress} in progress</span>
              <span>{w.notStarted} not started</span>
            </div>

            {/* projects */}
            {w.projects.length > 0 && (
              <div className="mt-4 border-t border-ink-800 pt-3">
                <div className="text-[10px] font-semibold uppercase tracking-wide text-ink-400">Projects</div>
                <div className="mt-1 flex flex-wrap gap-1.5">
                  {w.projects.map((p) => (
                    <span key={p} className="rounded bg-ink-850 px-2 py-0.5 text-xs text-ink-300">
                      {p}
                    </span>
                  ))}
                </div>
              </div>
            )}
          </div>
        ))}
      </div>

      <p className="mt-4 text-xs text-ink-400">
        · Waves are assigned deterministically by difficulty (classification + risk). Pilot = green YAML; the hard
        tail = classic or red. Sequence the program pilot-first.
      </p>
    </main>
  )
}
