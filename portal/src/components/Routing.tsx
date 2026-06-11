import { useEffect, useState } from 'react'
import type { BifrostApi, RoutingPolicy } from '../api/client'

type Klass = keyof RoutingPolicy

const KLASS: { key: Klass; title: string; hint: string }[] = [
  { key: 'bulk', title: 'Bulk / cheap gap-fills', hint: 'High-volume, simple fills — prefer a cheap or local model.' },
  { key: 'hard', title: 'Hard reasoning', hint: 'Tricky conversions — a stronger model is worth it.' },
  { key: 'docs', title: 'Documentation', hint: 'Runbook / rationale prose.' },
]

export function Routing({ api }: { api: BifrostApi }) {
  const [policy, setPolicy] = useState<RoutingPolicy | null>(null)
  const [airGap, setAirGap] = useState(false)
  const [saved, setSaved] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)

  useEffect(() => {
    api
      .getRouting()
      .then(({ policy, airGap }) => {
        setPolicy(policy)
        setAirGap(airGap)
      })
      .catch((e) => setError(String(e)))
  }, [api])

  function update(klass: Klass, csv: string) {
    if (!policy) return
    setSaved(false)
    setPolicy({
      ...policy,
      [klass]: csv
        .split(',')
        .map((s) => s.trim())
        .filter(Boolean),
    })
  }

  async function save() {
    if (!policy) return
    setBusy(true)
    setError(null)
    try {
      await api.putRouting(policy)
      setSaved(true)
    } catch (e) {
      setError(String(e))
    } finally {
      setBusy(false)
    }
  }

  const input =
    'w-full rounded-md border border-ink-800 bg-ink-900 px-3 py-2 text-sm text-ink-100 placeholder:text-ink-500'

  return (
    <main className="mx-auto w-full max-w-3xl flex-1 px-6 py-6">
      <h1 className="text-xl font-semibold text-ink-100">LLM routing</h1>
      <p className="mt-1 text-sm text-ink-300">
        Choose which providers fill which kind of gap, in preference order. The first usable
        provider wins. Comma-separated provider names (e.g. <code className="text-ink-100">ollama, anthropic</code>).
      </p>

      {airGap && (
        <div className="mt-4 rounded-lg border border-[var(--color-risk-amber)]/40 bg-[var(--color-risk-amber)]/10 px-4 py-2 text-sm text-[var(--color-risk-amber)]">
          Air-gap mode is on — only <span className="font-medium">local</span> providers are used,
          regardless of preference order. Frontier providers are blocked.
        </div>
      )}
      {error && (
        <div className="mt-4 rounded-lg border border-[var(--color-risk-red)]/40 bg-[var(--color-risk-red)]/10 px-4 py-2 text-sm text-[var(--color-risk-red)]">
          {error}
        </div>
      )}

      {!policy ? (
        <div className="mt-6 text-sm text-ink-300">Loading…</div>
      ) : (
        <div className="mt-6 space-y-4">
          {KLASS.map(({ key, title, hint }) => (
            <div key={key} className="rounded-xl border border-ink-800 bg-ink-900/40 p-4">
              <label className="mb-1 block text-sm font-medium text-ink-100">{title}</label>
              <p className="mb-2 text-xs text-ink-400">{hint}</p>
              <input className={input} value={policy[key].join(', ')} onChange={(e) => update(key, e.target.value)} />
            </div>
          ))}
          <div className="flex items-center gap-3">
            <button
              onClick={save}
              disabled={busy}
              className="rounded-lg bg-brand-500 px-4 py-2 text-sm font-medium text-ink-950 hover:bg-brand-400 disabled:opacity-60"
            >
              {busy ? 'Saving…' : 'Save routing policy'}
            </button>
            {saved && <span className="text-xs text-[var(--color-risk-green)]">Saved</span>}
          </div>
        </div>
      )}
    </main>
  )
}
