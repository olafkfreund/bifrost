import { useEffect, useState } from 'react'
import type { BifrostApi, ProviderInfo, RoutingPolicy } from '../api/client'

type Klass = keyof RoutingPolicy

const KLASS: { key: Klass; title: string; hint: string }[] = [
  { key: 'bulk', title: 'Bulk / cheap gap-fills', hint: 'High-volume, simple fills — prefer a cheap or local model.' },
  { key: 'hard', title: 'Hard reasoning', hint: 'Tricky conversions — a stronger model is worth it.' },
  { key: 'docs', title: 'Documentation', hint: 'Runbook / rationale prose.' },
]

export function Routing({ api }: { api: BifrostApi }) {
  const [policy, setPolicy] = useState<RoutingPolicy | null>(null)
  const [airGap, setAirGap] = useState(false)
  const [airGapLocked, setAirGapLocked] = useState(false)
  const [togglingAirGap, setTogglingAirGap] = useState(false)
  const [providers, setProviders] = useState<ProviderInfo[]>([])
  const [focused, setFocused] = useState<Klass>('bulk')
  const [saved, setSaved] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)

  useEffect(() => {
    Promise.all([api.getRouting(), api.getSettings(), api.getProviders()])
      .then(([{ policy }, settings, prov]) => {
        setPolicy(policy)
        setAirGap(settings.airGap)
        setAirGapLocked(settings.airGapLocked)
        setProviders(prov.catalog)
      })
      .catch((e) => setError(String(e)))
  }, [api])

  // Append a provider name to the last-focused class list (if not already there).
  function addProvider(name: string) {
    setSaved(false)
    setPolicy((prev) =>
      prev && !prev[focused].includes(name)
        ? { ...prev, [focused]: [...prev[focused], name] }
        : prev,
    )
  }

  async function toggleAirGap() {
    setError(null)
    setTogglingAirGap(true)
    try {
      const s = await api.setAirGap(!airGap)
      setAirGap(s.airGap)
      setAirGapLocked(s.airGapLocked)
    } catch (e) {
      setError(String(e))
    } finally {
      setTogglingAirGap(false)
    }
  }

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

      <div className="mt-4 rounded-xl border border-ink-800 bg-ink-900/40 p-4">
        <div className="flex items-center justify-between gap-4">
          <div>
            <div className="text-sm font-medium text-ink-100">Air-gap mode</div>
            <p className="mt-0.5 text-xs text-ink-400">
              No public egress: only providers marked <span className="font-medium">in-network</span>{' '}
              (<code className="text-ink-300">is_local</code>) are used — a private cloud-LLM endpoint
              (e.g. a private Azure OpenAI / Bedrock / Vertex endpoint) is allowed, public APIs are blocked.
            </p>
          </div>
          <button
            onClick={toggleAirGap}
            disabled={togglingAirGap || airGapLocked}
            aria-pressed={airGap}
            title={airGapLocked ? 'Locked on by the deployment (BIFROST_AIR_GAP_LOCK)' : undefined}
            className={`relative inline-flex h-6 w-11 shrink-0 items-center rounded-full transition disabled:opacity-60 ${
              airGap ? 'bg-[var(--color-risk-amber)]' : 'bg-ink-700'
            }`}
          >
            <span
              className={`inline-block h-5 w-5 transform rounded-full bg-ink-100 transition ${
                airGap ? 'translate-x-5' : 'translate-x-0.5'
              }`}
            />
          </button>
        </div>
        <div className="mt-2 text-xs">
          {airGapLocked ? (
            <span className="text-[var(--color-risk-amber)]">
              Locked on by the deployment — cannot be disabled here.
            </span>
          ) : airGap ? (
            <span className="text-[var(--color-risk-amber)]">
              On — frontier (public-API) providers are blocked regardless of preference order.
            </span>
          ) : (
            <span className="text-ink-400">Off — all configured providers are eligible.</span>
          )}
        </div>
      </div>

      {providers.length > 0 && (
        <div className="mt-4 rounded-xl border border-ink-800 bg-ink-900/40 p-4">
          <div className="text-sm font-medium text-ink-100">Providers</div>
          <p className="mt-0.5 text-xs text-ink-400">
            Click a configured provider to add it to the focused list below. To enable one, add an
            LLM connection on the <span className="font-medium text-ink-200">Connections</span> page
            (recommended), or set its environment variable.
          </p>
          <div className="mt-3 flex flex-wrap gap-2">
            {providers.map((p) => (
              <button
                key={p.name}
                onClick={() => p.active && addProvider(p.name)}
                disabled={!p.active}
                title={
                  p.active
                    ? `${p.label} — click to add to "${focused}"`
                    : `${p.label} — not configured. Add a connection or set ${p.enableEnv}.`
                }
                className={`inline-flex items-center gap-1.5 whitespace-nowrap rounded-full border px-2.5 py-1 text-xs font-medium transition ${
                  p.active
                    ? 'border-[var(--color-risk-green)]/40 bg-[var(--color-risk-green)]/10 text-[var(--color-risk-green)] hover:bg-[var(--color-risk-green)]/20'
                    : 'cursor-not-allowed border-ink-800 bg-ink-900 text-ink-500'
                }`}
              >
                <span
                  className="h-1.5 w-1.5 shrink-0 rounded-full"
                  style={{ backgroundColor: p.active ? 'var(--color-risk-green)' : 'var(--color-ink-600)' }}
                />
                {p.name}
              </button>
            ))}
          </div>
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
              <input
                className={input}
                value={policy[key].join(', ')}
                onFocus={() => setFocused(key)}
                onChange={(e) => update(key, e.target.value)}
              />
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
