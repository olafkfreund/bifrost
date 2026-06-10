import { useEffect, useState } from 'react'
import type { BifrostApi } from '../api/client'
import type { ConversionResult, Pipeline } from '../types'
import { checklistCategoryLabel, statusLabel } from '../lib/format'
import { RiskBadge } from './RiskBadge'

function Section({ title, children, hint }: { title: string; children: React.ReactNode; hint?: string }) {
  return (
    <section className="mt-6">
      <div className="flex items-baseline justify-between">
        <h3 className="text-sm font-semibold text-ink-100">{title}</h3>
        {hint && <span className="text-xs text-ink-300">{hint}</span>}
      </div>
      <div className="mt-2">{children}</div>
    </section>
  )
}

/**
 * The proposal review surface for a single pipeline. Fetches the conversion
 * (`/convert`) and renders the assembled workflow, the model's rationale + flags,
 * the deterministic risk, and the manual-task runbook. The Monaco three-pane
 * diff (+ ADO source pane) is a later slice; this is the read-only foundation.
 */
export function ProposalPanel({
  pipeline,
  api,
  onClose,
}: {
  pipeline: Pipeline | null
  api: BifrostApi
  onClose: () => void
}) {
  // Keyed by pipeline id: a fresh fetch per pipeline, with status discriminated
  // so we never call setState synchronously in the effect body (which would
  // cascade renders). `for` ties the state to the request that produced it.
  const [state, setState] = useState<{
    for: string | null
    status: 'loading' | 'ok' | 'error'
    result?: ConversionResult
    error?: string
  }>({ for: null, status: 'loading' })

  useEffect(() => {
    if (!pipeline) return
    const id = pipeline.id
    let live = true
    api
      .convertPipeline(id)
      .then((r) => live && setState({ for: id, status: 'ok', result: r }))
      .catch((e) => live && setState({ for: id, status: 'error', error: String(e) }))
    return () => {
      live = false
    }
  }, [pipeline, api])

  if (!pipeline) return null
  // Only trust state that belongs to the pipeline currently shown.
  const current = state.for === pipeline.id ? state : null
  const loading = !current || current.status === 'loading'
  const error = current?.status === 'error' ? current.error : null
  const result = current?.status === 'ok' ? current.result : null
  const proposal = result?.proposal

  return (
    <div className="fixed inset-0 z-50 flex justify-end">
      <div className="absolute inset-0 bg-black/60" onClick={onClose} />
      <aside className="relative z-[60] flex h-full w-full max-w-3xl flex-col border-l border-ink-800 bg-ink-900 shadow-2xl">
        <div className="flex items-start justify-between border-b border-ink-800 p-5">
          <div>
            <div className="text-xs text-ink-300">{pipeline.project} · proposal</div>
            <h2 className="text-lg font-semibold text-ink-100">{pipeline.name}</h2>
            <div className="mt-2 flex flex-wrap items-center gap-2">
              <RiskBadge band={proposal?.riskBand ?? pipeline.riskBand} score={proposal?.riskScore ?? pipeline.riskScore} />
              {proposal && (
                <>
                  <span className="rounded-full bg-ink-800 px-2 py-0.5 text-xs text-ink-300">
                    {statusLabel[proposal.status]}
                  </span>
                  <span className="rounded-full bg-ink-800 px-2 py-0.5 text-xs text-ink-300">
                    prompt {proposal.promptId}
                  </span>
                  <span className="rounded-full bg-ink-800 px-2 py-0.5 text-xs text-ink-300">
                    confidence {Math.round(proposal.confidence * 100)}%
                  </span>
                </>
              )}
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
          {loading && <div className="animate-pulse text-sm text-ink-300">Converting pipeline…</div>}
          {error && (
            <div className="rounded-lg border border-[var(--color-risk-red)]/40 bg-[var(--color-risk-red-dim)] p-3 text-sm text-[var(--color-risk-red)]">
              Conversion failed: {error}
            </div>
          )}

          {proposal && (
            <>
              <Section title="Proposed workflow" hint="Importer baseline + LLM gap-fills">
                <pre className="max-h-80 overflow-auto rounded-lg border border-ink-800 bg-ink-950 p-4 text-xs leading-relaxed text-ink-100">
                  <code>{proposal.proposedYaml}</code>
                </pre>
                <p className="mt-2 text-xs text-ink-300">
                  Blocks above the <span className="font-mono">REVIEW BEFORE USE</span> banner are the official
                  Importer's output; blocks below are model-proposed and tagged with their source construct.
                </p>
              </Section>

              <Section title="Rationale" hint="LLM — explanation only, not scoring">
                <p className="whitespace-pre-wrap text-sm text-ink-200">{proposal.rationale || '—'}</p>
              </Section>

              {proposal.riskFlags.length > 0 && (
                <Section title="Risk flags" hint="reviewer must check">
                  <ul className="space-y-1.5">
                    {proposal.riskFlags.map((f, i) => (
                      <li key={i} className="flex gap-2 text-sm text-ink-200">
                        <span className="text-[var(--color-risk-amber)]">▲</span>
                        {f}
                      </li>
                    ))}
                  </ul>
                </Section>
              )}

              {proposal.verifySteps.length > 0 && (
                <Section title="Verify before approving">
                  <ul className="space-y-1.5">
                    {proposal.verifySteps.map((s, i) => (
                      <li key={i} className="flex gap-2 text-sm text-ink-200">
                        <span className="text-ink-500">○</span>
                        {s}
                      </li>
                    ))}
                  </ul>
                </Section>
              )}

              <Section
                title="Manual-task runbook"
                hint={`${result?.runbook.items.length ?? 0} item${
                  (result?.runbook.items.length ?? 0) === 1 ? '' : 's'
                } the Importer can't do`}
              >
                {result && result.runbook.items.length > 0 ? (
                  <ul className="space-y-2">
                    {result.runbook.items.map((item, i) => (
                      <li key={i} className="rounded-lg border border-ink-800 bg-ink-850 p-3">
                        <div className="flex items-center justify-between">
                          <span className="text-sm font-medium text-ink-100">{item.title}</span>
                          <span className="rounded-full bg-ink-800 px-2 py-0.5 text-[11px] uppercase tracking-wide text-ink-300">
                            {checklistCategoryLabel[item.category]}
                          </span>
                        </div>
                        <p className="mt-1 text-xs text-ink-300">
                          <span className="font-mono text-ink-200">{item.construct}</span> — {item.detail}
                        </p>
                      </li>
                    ))}
                  </ul>
                ) : (
                  <p className="text-sm text-ink-300">No manual tasks — the Importer covered everything.</p>
                )}
              </Section>
            </>
          )}
        </div>

        <div className="border-t border-ink-800 p-4">
          <button
            disabled
            title="Approve / request changes lands in M3 (#52)"
            className="w-full cursor-not-allowed rounded-lg border border-ink-700 bg-ink-850 px-4 py-2 text-sm font-medium text-ink-300"
          >
            Approve · edit · request changes — coming in M3
          </button>
        </div>
      </aside>
    </div>
  )
}
