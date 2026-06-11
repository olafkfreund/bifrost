import { useCallback, useEffect, useState } from 'react'
import type { BifrostApi } from '../api/client'

type Status = 'pending' | 'running' | 'ok' | 'warn' | 'fail'

interface StepResult {
  status: Status
  detail: string
  /** Precise fix when the check fails or warns. */
  fix?: string
}

interface Step {
  key: string
  title: string
  /** Run the check; return its result. */
  run: (api: BifrostApi) => Promise<StepResult>
  /** Optional in-portal action (e.g. jump to the Connections page). */
  action?: { label: string; go: () => void }
}

const dot: Record<Status, string> = {
  pending: 'bg-ink-700',
  running: 'bg-[var(--color-risk-amber)] animate-pulse',
  ok: 'bg-[var(--color-risk-green)]',
  warn: 'bg-[var(--color-risk-amber)]',
  fail: 'bg-[var(--color-risk-red)]',
}

export function OnboardingWizard({
  api,
  onClose,
  onGoConnections,
}: {
  api: BifrostApi
  onClose: () => void
  onGoConnections: () => void
}) {
  const steps: Step[] = [
    {
      key: 'backend',
      title: 'Control plane reachable',
      run: async (a) =>
        (await a.health())
          ? { status: 'ok', detail: 'The Bifrost API is up.' }
          : {
              status: 'fail',
              detail: 'Could not reach the API.',
              fix: 'Start it: `cargo run -p bifrost-api` (binds BIFROST_API_ADDR, default 127.0.0.1:8080), and serve the portal with VITE_API=http.',
            },
    },
    {
      key: 'identity',
      title: 'Identity & SSO',
      run: async (a) => {
        const me = await a.me()
        if (!me)
          return {
            status: 'fail',
            detail: 'Authentication is enabled but no valid session.',
            fix: 'Sign in via Entra (MSAL) so the portal sends a bearer token, or unset BIFROST_AUTH to run open.',
          }
        if (me.subject === 'local')
          return {
            status: 'warn',
            detail: 'Running OPEN — every request is the local admin.',
            fix: 'For production set BIFROST_AUTH=entra + BIFROST_ENTRA_TENANT_ID / _AUDIENCE.',
          }
        return { status: 'ok', detail: `Signed in as ${me.email ?? me.subject} (${me.roles.join(', ') || 'viewer'}).` }
      },
    },
    {
      key: 'secrets',
      title: 'Secret backend',
      run: async () => {
        // We can't read server env from the browser; surface the guidance.
        return {
          status: 'warn',
          detail: 'Choose how secrets are stored.',
          fix: 'Preferred: Azure Key Vault references on each connection. Fallback: set BIFROST_SECRET_KEY to enable the encrypted-inline option.',
        }
      },
    },
    {
      key: 'connections',
      title: 'Connections',
      action: { label: 'Add a connection', go: onGoConnections },
      run: async (a) => {
        const conns = await a.listConnections()
        return conns.length
          ? { status: 'ok', detail: `${conns.length} connection(s) configured.` }
          : {
              status: 'warn',
              detail: 'No connections yet.',
              fix: 'Add an Azure DevOps org (+ a GitHub App and an LLM provider) on the Connections page.',
            }
      },
    },
    {
      key: 'audit',
      title: 'First audit',
      run: async (a) => {
        const pf = await a.getPortfolio()
        return pf.pipelines.length
          ? { status: 'ok', detail: `Portfolio loaded — ${pf.pipelines.length} pipelines across ${pf.summary.totals.projects} projects.` }
          : {
              status: 'warn',
              detail: 'No pipelines yet.',
              fix: 'Once a connection is set, refresh to audit it; the heatmap populates from the audit.',
            }
      },
    },
  ]

  const [results, setResults] = useState<Record<string, StepResult>>({})

  const runAll = useCallback(async () => {
    for (const step of steps) {
      setResults((r) => ({ ...r, [step.key]: { status: 'running', detail: 'Checking…' } }))
      try {
        const res = await step.run(api)
        setResults((r) => ({ ...r, [step.key]: res }))
      } catch (e) {
        setResults((r) => ({
          ...r,
          [step.key]: { status: 'fail', detail: String(e) },
        }))
      }
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [api])

  useEffect(() => {
    runAll()
  }, [runAll])

  const allGreen = steps.every((s) => results[s.key]?.status === 'ok')

  function finish() {
    localStorage.setItem('bifrost_onboarded', '1')
    onClose()
  }

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4">
      <div className="max-h-full w-full max-w-2xl overflow-y-auto rounded-2xl border border-ink-800 bg-ink-900 p-6 shadow-xl">
        <div className="mb-4 flex items-start justify-between">
          <div>
            <h2 className="text-lg font-semibold text-ink-100">Welcome to Bifrost</h2>
            <p className="text-sm text-ink-300">Let's check your setup. Fix anything flagged, then re-run.</p>
          </div>
          <button onClick={onClose} className="rounded-md px-2 py-1 text-ink-400 hover:text-ink-100" aria-label="Close">
            ✕
          </button>
        </div>

        <ol className="space-y-3">
          {steps.map((step, i) => {
            const res = results[step.key] ?? { status: 'pending' as Status, detail: '' }
            return (
              <li key={step.key} className="rounded-lg border border-ink-800 bg-ink-900/50 p-3">
                <div className="flex items-center gap-2">
                  <span className={`h-2.5 w-2.5 rounded-full ${dot[res.status]}`} />
                  <span className="text-sm font-medium text-ink-100">
                    {i + 1}. {step.title}
                  </span>
                  {step.action && (
                    <button
                      onClick={step.action.go}
                      className="ml-auto rounded border border-ink-800 px-2 py-0.5 text-xs text-ink-300 hover:text-ink-100"
                    >
                      {step.action.label}
                    </button>
                  )}
                </div>
                {res.detail && <p className="mt-1 pl-4.5 text-xs text-ink-300">{res.detail}</p>}
                {res.fix && (
                  <p className="mt-1 pl-4.5 text-xs text-[var(--color-risk-amber)]">→ {res.fix}</p>
                )}
              </li>
            )
          })}
        </ol>

        <div className="mt-5 flex items-center justify-end gap-2">
          <button
            onClick={runAll}
            className="rounded-lg border border-ink-800 px-3 py-1.5 text-sm text-ink-200 hover:bg-ink-850"
          >
            Re-run checks
          </button>
          <button
            onClick={finish}
            className="rounded-lg bg-brand-500 px-4 py-1.5 text-sm font-medium text-ink-950 hover:bg-brand-400"
          >
            {allGreen ? 'Finish' : 'Continue anyway'}
          </button>
        </div>
      </div>
    </div>
  )
}
