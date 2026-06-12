import { useEffect, useRef, useState } from 'react'
import { DiffEditor } from '@monaco-editor/react'
import type { editor } from 'monaco-editor'
import { monaco } from '../lib/monaco' // configures Monaco locally + Gruvbox themes
import type { BifrostApi } from '../api/client'
import type { ConversionResult, Pipeline, ProposalStatus } from '../types'
import type { Theme } from '../lib/theme'
import { isLight } from '../lib/theme'
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
 * Provenance decorations for the generated (right) side of the diff: everything
 * below the "REVIEW BEFORE USE" banner is tinted, and each `# bifrost-gap-fill:`
 * line gets a gutter marker. Pure — returns the deltas to apply.
 */
function provenanceDecorations(model: editor.ITextModel): editor.IModelDeltaDecoration[] {
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
  return decorations
}

const DIFF_OPTIONS: editor.IStandaloneDiffEditorConstructionOptions = {
  renderSideBySide: true,
  minimap: { enabled: false },
  scrollBeyondLastLine: false,
  fontSize: 12,
  lineNumbers: 'on',
  renderOverviewRuler: false,
  automaticLayout: true,
  guides: { indentation: false },
}

function btn(kind: 'primary' | 'ghost' | 'danger') {
  const base = 'rounded-lg px-4 py-2 text-sm font-medium transition disabled:opacity-50'
  if (kind === 'primary') return `${base} bg-brand-500 text-ink-950 hover:bg-brand-400`
  if (kind === 'danger') return `${base} border border-[var(--color-risk-amber)]/50 text-[var(--color-risk-amber)] hover:bg-ink-850`
  return `${base} border border-ink-700 text-ink-200 hover:bg-ink-850`
}

type PanelState = {
  for: string | null
  status: 'loading' | 'ok' | 'error'
  result?: ConversionResult
  error?: string
  editMode?: boolean
  busy?: boolean
  actionError?: string
}

/**
 * The proposal review surface: a three-pane diff (ADO source vs. the generated,
 * provenance-highlighted workflow) with the rationale, risk-factor breakdown,
 * runbook, and audit trail in the rail — plus the lifecycle actions (submit /
 * approve / request changes / edit), which drive the state machine and audit log.
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
  const [state, setState] = useState<PanelState>({ for: null, status: 'loading' })
  const diffRef = useRef<editor.IStandaloneDiffEditor | null>(null)
  const decoRef = useRef<editor.IEditorDecorationsCollection | null>(null)
  const modelsRef = useRef<{ original: editor.ITextModel | null; modified: editor.ITextModel | null }>({
    original: null,
    modified: null,
  })

  // On unmount the child DiffEditor is torn down first (with its models kept);
  // only then do we dispose the models, so the disposal can't race the widget.
  useEffect(
    () => () => {
      modelsRef.current.original?.dispose()
      modelsRef.current.modified?.dispose()
      modelsRef.current = { original: null, modified: null }
    },
    [],
  )

  // Esc closes the dialog (expected of any modal; keyboard-first reviewers).
  useEffect(() => {
    if (!pipeline) return
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose()
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [pipeline, onClose])

  // (Re)apply provenance highlighting, reusing one collection so edits refresh
  // it rather than stacking stale decorations.
  const applyProvenance = (diff: editor.IStandaloneDiffEditor) => {
    const model = diff.getModifiedEditor().getModel()
    if (!model) return
    const decorations = provenanceDecorations(model)
    if (decoRef.current) decoRef.current.set(decorations)
    else decoRef.current = diff.getModifiedEditor().createDecorationsCollection(decorations)
  }

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

  // Refresh provenance highlighting when the generated workflow changes (edits).
  useEffect(() => {
    if (diffRef.current) applyProvenance(diffRef.current)
  }, [state.result?.proposal.proposedYaml])

  if (!pipeline) return null
  const id = pipeline.id
  const current = state.for === id ? state : null
  const loading = !current || current.status === 'loading'
  const error = current?.status === 'error' ? current.error : null
  const result = current?.status === 'ok' ? current.result : null
  const proposal = result?.proposal
  const editMode = current?.editMode ?? false
  const busy = current?.busy ?? false
  const actionError = current?.actionError
  const monacoTheme = isLight(theme) ? 'bifrost-light' : 'bifrost-dark'

  // Run a lifecycle/edit action, folding the result back into the keyed state.
  async function runAction(fn: () => Promise<ConversionResult>) {
    setState((s) => (s.for === id ? { ...s, busy: true, actionError: undefined } : s))
    try {
      const r = await fn()
      setState((s) => (s.for === id ? { ...s, status: 'ok', result: r, busy: false, editMode: false } : s))
    } catch (e) {
      setState((s) => (s.for === id ? { ...s, busy: false, actionError: String(e) } : s))
    }
  }

  const transition = (to: ProposalStatus) => proposal && runAction(() => api.transitionProposal(proposal.id, to))
  const startEdit = () => setState((s) => (s.for === id ? { ...s, editMode: true, actionError: undefined } : s))
  const cancelEdit = () => {
    if (proposal) diffRef.current?.getModifiedEditor().setValue(proposal.proposedYaml)
    setState((s) => (s.for === id ? { ...s, editMode: false, actionError: undefined } : s))
  }
  const saveEdit = () => {
    const yaml = diffRef.current?.getModifiedEditor().getValue()
    if (yaml != null && proposal) runAction(() => api.editProposal(proposal.id, yaml))
  }

  return (
    <div className="fixed inset-0 z-50 flex">
      <div className="bf-scrim" onClick={onClose} />
      <div className="bf-dialog relative z-[60] m-auto flex h-[92vh] w-[96vw] max-w-[1500px] flex-col overflow-hidden rounded-xl">
        {/* header */}
        <div className="flex items-start justify-between border-b border-ink-800 p-5">
          <div>
            <div className="text-xs text-ink-300">{pipeline.project} · proposal</div>
            <h2 className="text-lg font-semibold text-ink-100">{pipeline.name}</h2>
            <div className="mt-2 flex flex-wrap items-center gap-2">
              <RiskBadge band={proposal?.riskBand ?? pipeline.riskBand} score={proposal?.riskScore ?? pipeline.riskScore} />
              {proposal && (
                <>
                  <span className="rounded-full bg-ink-800 px-2 py-0.5 text-xs text-ink-300">{statusLabel[proposal.status]}</span>
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
                Generated workflow ·{' '}
                {editMode ? (
                  <span style={{ color: 'var(--color-accent-aqua)' }}>editing — your changes save to the proposal</span>
                ) : (
                  <span style={{ color: 'var(--color-accent-aqua)' }}>provenance-highlighted</span>
                )}
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
                  // Keep the models on unmount: @monaco-editor/react otherwise
                  // disposes them while the diff widget is still resetting, which
                  // throws "TextModel got disposed before DiffEditorWidget model
                  // got reset" when the panel closes. We dispose them ourselves
                  // in the cleanup below, after the widget is gone.
                  keepCurrentOriginalModel
                  keepCurrentModifiedModel
                  options={{ ...DIFF_OPTIONS, readOnly: !editMode }}
                  onMount={(ed) => {
                    // Dispose the previous open's kept models (their widget is
                    // long gone, so this can't race) — bounds the leak to one.
                    modelsRef.current.original?.dispose()
                    modelsRef.current.modified?.dispose()
                    diffRef.current = ed
                    modelsRef.current = {
                      original: ed.getOriginalEditor().getModel(),
                      modified: ed.getModifiedEditor().getModel(),
                    }
                    applyProvenance(ed)
                  }}
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

                {[...new Set(proposal.riskFlags)].length > 0 && (
                  <Section title="Risk flags" hint="reviewer must check">
                    <ul className="space-y-1.5">
                      {[...new Set(proposal.riskFlags)].map((f, i) => (
                        <li key={i} className="flex gap-2 text-sm text-ink-200">
                          <span className="text-[var(--color-risk-amber)]">▲</span>
                          {f}
                        </li>
                      ))}
                    </ul>
                  </Section>
                )}

                {[...new Set(proposal.verifySteps)].length > 0 && (
                  <Section title="Verify before approving">
                    <ul className="space-y-1.5">
                      {[...new Set(proposal.verifySteps)].map((s, i) => (
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
                  hint={`${result?.runbook.items.length ?? 0} item${(result?.runbook.items.length ?? 0) === 1 ? '' : 's'}`}
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

                <Section title="Audit trail" hint={`${result?.audit.length ?? 0} event${(result?.audit.length ?? 0) === 1 ? '' : 's'}`}>
                  {result && result.audit.length > 0 ? (
                    <ol className="space-y-2">
                      {result.audit.map((e, i) => (
                        <li key={i} className="flex gap-2 text-xs">
                          <span className="mt-0.5 h-1.5 w-1.5 shrink-0 rounded-full bg-brand-400" />
                          <div className="min-w-0">
                            <div className="text-ink-200">
                              {e.note ? (
                                <span>edited workflow</span>
                              ) : (
                                <span>
                                  {statusLabel[e.from]} <span className="text-ink-500">→</span> {statusLabel[e.to]}
                                </span>
                              )}
                            </div>
                            <div className="text-ink-500">
                              {e.actor} · {new Date(e.at).toLocaleString()}
                            </div>
                          </div>
                        </li>
                      ))}
                    </ol>
                  ) : (
                    <p className="text-sm text-ink-300">No actions yet — submit for review to begin.</p>
                  )}
                </Section>
              </>
            )}
          </aside>
        </div>

        {/* action bar */}
        <div className="shrink-0 border-t border-ink-800 p-4">
          {actionError && (
            <div className="mb-3 rounded-md border border-[var(--color-risk-red)]/40 bg-[var(--color-risk-red-dim)] px-3 py-2 text-xs text-[var(--color-risk-red)]">
              {actionError}
            </div>
          )}
          <div className="flex flex-wrap items-center gap-2">
            {proposal && editMode && (
              <>
                <button className={btn('primary')} disabled={busy} onClick={saveEdit}>
                  {busy ? 'Saving…' : 'Save changes'}
                </button>
                <button className={btn('ghost')} disabled={busy} onClick={cancelEdit}>
                  Cancel
                </button>
                <span className="text-xs text-ink-300">Edit the right pane, then save — recorded in the audit log.</span>
              </>
            )}

            {proposal && !editMode && proposal.status === 'draft' && (
              <>
                <button className={btn('primary')} disabled={busy} onClick={() => transition('in_review')}>
                  Submit for review
                </button>
                <button className={btn('ghost')} disabled={busy} onClick={startEdit}>
                  Edit workflow
                </button>
              </>
            )}

            {proposal && !editMode && proposal.status === 'in_review' && (
              <>
                <button className={btn('primary')} disabled={busy} onClick={() => transition('approved')}>
                  Approve
                </button>
                <button className={btn('danger')} disabled={busy} onClick={() => transition('changes_requested')}>
                  Request changes
                </button>
                <button className={btn('ghost')} disabled={busy} onClick={startEdit}>
                  Edit workflow
                </button>
              </>
            )}

            {proposal && !editMode && proposal.status === 'changes_requested' && (
              <>
                <button className={btn('primary')} disabled={busy} onClick={() => transition('in_review')}>
                  Resubmit for review
                </button>
                <button className={btn('ghost')} disabled={busy} onClick={startEdit}>
                  Edit workflow
                </button>
              </>
            )}

            {proposal && !editMode && proposal.status === 'approved' && (
              <>
                <span className="text-sm font-medium" style={{ color: 'var(--color-accent-aqua)' }}>
                  ✓ Approved — ready to commit
                </span>
                <button className={btn('ghost')} disabled title="Commit & open PR lands in M4 (#56)">
                  Commit &amp; open PR — M4
                </button>
              </>
            )}

            {proposal && !editMode && (proposal.status === 'committed' || proposal.status === 'validated') && (
              <span className="text-sm font-medium text-ink-200">{statusLabel[proposal.status]}</span>
            )}
          </div>
        </div>
      </div>
    </div>
  )
}
