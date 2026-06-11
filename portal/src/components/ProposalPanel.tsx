import { useEffect, useState } from 'react'
import { DiffEditor } from '@monaco-editor/react'
import type { Monaco } from '@monaco-editor/react'
import type { editor } from 'monaco-editor'
import '../lib/monaco' // side-effect: bundle Monaco locally + register Gruvbox themes
import type { BifrostApi } from '../api/client'
import type { ConversionResult, Pipeline } from '../types'
import type { Theme } from '../lib/theme'
import { checklistCategoryLabel, statusLabel } from '../lib/format'
import { RiskBadge } from './RiskBadge'

function Section({ title, children, hint }: { title: string; children: React.ReactNode; hint?: string }) {
  return (
    <section className="mt-6 first:mt-0">
      <div className="flex items-baseline justify-between">
        <h3 className="text-sm font-semibold text-ink-100">{title}</h3>
        {hint && <span className="text-xs text-ink-300">{hint}</span>}
      </div>
      <div className="mt-2">{children}</div>
    </section>
  )
}

/**
 * Highlight the model's contribution on the generated (right) side of the diff:
 * everything below the "REVIEW BEFORE USE" banner is tinted, and each
 * `# bifrost-gap-fill:` provenance line gets a gutter marker. Reviewers can see
 * at a glance which lines are the Importer's and which are Bifrost's.
 */
function decorateProvenance(diff: editor.IStandaloneDiffEditor, monaco: Monaco) {
  const modified = diff.getModifiedEditor()
  const model = modified.getModel()
  if (!model) return

  const lines = model.getLinesContent()
  const bannerIdx = lines.findIndex((l) => l.includes('REVIEW BEFORE USE'))
  const decorations: editor.IModelDeltaDecoration[] = []

  lines.forEach((line, i) => {
    const lineNo = i + 1
    if (bannerIdx >= 0 && i >= bannerIdx) {
      decorations.push({
        range: new monaco.Range(lineNo, 1, lineNo, 1),
        options: { isWholeLine: true, className: 'bifrost-gapfill-line' },
      })
    }
    if (line.trimStart().startsWith('# bifrost-gap-fill:')) {
      decorations.push({
        range: new monaco.Range(lineNo, 1, lineNo, 1),
        options: { isWholeLine: true, linesDecorationsClassName: 'bifrost-gapfill-glyph' },
      })
    }
  })

  modified.createDecorationsCollection(decorations)
}

const DIFF_OPTIONS: editor.IStandaloneDiffEditorConstructionOptions = {
  readOnly: true,
  renderSideBySide: true,
  minimap: { enabled: false },
  scrollBeyondLastLine: false,
  fontSize: 12,
  lineNumbers: 'on',
  renderOverviewRuler: false,
  automaticLayout: true,
  guides: { indentation: false },
}

/**
 * The proposal review surface for a single pipeline: a three-pane diff —
 * ADO source (left) vs. the generated, provenance-highlighted Actions workflow
 * (right), with the LLM rationale, deterministic risk-factor breakdown, verify
 * steps, and manual-task runbook in the rail. Approve / request-changes lands
 * next (#52).
 */
export function ProposalPanel({
  pipeline,
  api,
  theme,
  onClose,
}: {
  pipeline: Pipeline | null
  api: BifrostApi
  theme: Theme
  onClose: () => void
}) {
  // Keyed by pipeline id so a stale fetch never paints over the current one,
  // and so we never setState synchronously in the effect body.
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
  const current = state.for === pipeline.id ? state : null
  const loading = !current || current.status === 'loading'
  const error = current?.status === 'error' ? current.error : null
  const result = current?.status === 'ok' ? current.result : null
  const proposal = result?.proposal
  const monacoTheme = theme === 'dark' ? 'bifrost-dark' : 'bifrost-light'

  return (
    <div className="fixed inset-0 z-50 flex">
      <div className="absolute inset-0 bg-black/60" onClick={onClose} />
      <div className="relative z-[60] m-auto flex h-[92vh] w-[96vw] max-w-[1500px] flex-col overflow-hidden rounded-xl border border-ink-800 bg-ink-900 shadow-2xl">
        {/* header */}
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
                  <span className="rounded-full bg-ink-800 px-2 py-0.5 text-xs text-ink-300">prompt {proposal.promptId}</span>
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

        {/* body: diff (left) + review rail (right) */}
        <div className="flex min-h-0 flex-1">
          <div className="flex min-w-0 flex-1 flex-col border-r border-ink-800">
            <div className="flex shrink-0 border-b border-ink-800 text-xs">
              <div className="flex-1 px-4 py-2 font-medium text-ink-300">
                ADO source · <span className="font-mono text-ink-200">azure-pipelines.yml</span>
              </div>
              <div className="flex-1 px-4 py-2 font-medium text-ink-300">
                Generated workflow · <span style={{ color: 'var(--color-accent-aqua)' }}>provenance-highlighted</span>
              </div>
            </div>
            <div className="min-h-0 flex-1">
              {loading && <div className="p-5 text-sm text-ink-300 animate-pulse">Converting pipeline…</div>}
              {error && (
                <div className="m-5 rounded-lg border border-[var(--color-risk-red)]/40 bg-[var(--color-risk-red-dim)] p-3 text-sm text-[var(--color-risk-red)]">
                  Conversion failed: {error}
                </div>
              )}
              {proposal && (
                <DiffEditor
                  height="100%"
                  language="yaml"
                  theme={monacoTheme}
                  original={proposal.sourceYaml}
                  modified={proposal.proposedYaml}
                  options={DIFF_OPTIONS}
                  onMount={(ed, monaco) => decorateProvenance(ed, monaco)}
                />
              )}
            </div>
          </div>

          {/* review rail */}
          <aside className="w-[360px] shrink-0 overflow-y-auto p-5">
            {!proposal && !error && <div className="text-sm text-ink-300">Loading rationale…</div>}
            {proposal && (
              <>
                {pipeline.factors.length > 0 && (
                  <Section title="Risk factors" hint="deterministic">
                    <ul className="space-y-2">
                      {pipeline.factors.map((f) => (
                        <li key={f.key} title={f.detail}>
                          <div className="flex items-baseline justify-between text-xs">
                            <span className="text-ink-200">{f.label}</span>
                            <span className="font-mono text-ink-300">+{f.contribution}</span>
                          </div>
                          <div className="mt-1 h-1.5 overflow-hidden rounded-full bg-ink-800">
                            <div
                              className="h-full rounded-full bg-[var(--color-risk-amber)]"
                              style={{ width: `${Math.min(100, f.contribution)}%` }}
                            />
                          </div>
                        </li>
                      ))}
                    </ul>
                  </Section>
                )}

                <Section title="Rationale" hint="LLM — explanation only">
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
                  }`}
                >
                  {result && result.runbook.items.length > 0 ? (
                    <ul className="space-y-2">
                      {result.runbook.items.map((item, i) => (
                        <li key={i} className="rounded-lg border border-ink-800 bg-ink-850 p-3">
                          <div className="flex items-center justify-between gap-2">
                            <span className="text-sm font-medium text-ink-100">{item.title}</span>
                            <span className="shrink-0 rounded-full bg-ink-800 px-2 py-0.5 text-[11px] uppercase tracking-wide text-ink-300">
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
          </aside>
        </div>

        {/* action bar */}
        <div className="shrink-0 border-t border-ink-800 p-4">
          <button
            disabled
            title="Approve / request changes lands in M3 (#52)"
            className="w-full cursor-not-allowed rounded-lg border border-ink-700 bg-ink-850 px-4 py-2 text-sm font-medium text-ink-300"
          >
            Approve · edit · request changes — coming in M3
          </button>
        </div>
      </div>
    </div>
  )
}
