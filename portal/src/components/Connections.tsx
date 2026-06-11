import { useEffect, useState } from 'react'
import type { BifrostApi, ConnectionInput } from '../api/client'
import type { ConnectionView, SecretRefView } from '../types'

type Kind = 'azure-devops' | 'github' | 'llm'
type AuthMethod = 'key-vault' | 'github-app' | 'entra-wif' | 'env-var' | 'inline'

const KIND_LABEL: Record<Kind, string> = {
  'azure-devops': 'Azure DevOps',
  github: 'GitHub',
  llm: 'LLM provider',
}

// Vault/identity references first; the inline (encrypted) secret is the labelled
// fallback and is deliberately last + warned.
const AUTH_LABEL: Record<AuthMethod, string> = {
  'key-vault': 'Azure Key Vault reference (recommended)',
  'github-app': 'GitHub App installation',
  'entra-wif': 'Entra workload identity',
  'env-var': 'Environment variable',
  inline: 'Inline secret (encrypted) — fallback',
}

function authSummary(ref?: SecretRefView): string {
  if (!ref) return '—'
  switch (ref.type) {
    case 'key-vault':
      return `Key Vault · ${ref.uri}`
    case 'git-hub-app':
      return `GitHub App · ${ref.installation_id}`
    case 'entra-wif':
      return 'Entra workload identity'
    case 'env-var':
      return `env · ${ref.name}`
    case 'encrypted-inline':
      return 'inline (encrypted)'
  }
}

export function Connections({ api }: { api: BifrostApi }) {
  const [conns, setConns] = useState<ConnectionView[] | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)

  // Form state
  const [kind, setKind] = useState<Kind>('azure-devops')
  const [name, setName] = useState('')
  const [orgUrl, setOrgUrl] = useState('')
  const [org, setOrg] = useState('')
  const [provider, setProvider] = useState('openai-compatible')
  const [model, setModel] = useState('')
  const [baseUrl, setBaseUrl] = useState('')
  const [isLocal, setIsLocal] = useState(true)
  const [residency, setResidency] = useState('')
  const [authMethod, setAuthMethod] = useState<AuthMethod>('key-vault')
  const [authValue, setAuthValue] = useState('')
  const [authValue2, setAuthValue2] = useState('')

  function load() {
    api
      .listConnections()
      .then(setConns)
      .catch((e) => setError(String(e)))
  }
  useEffect(load, [api])

  function buildAuth(): Record<string, unknown> {
    switch (authMethod) {
      case 'key-vault':
        return { type: 'key-vault', uri: authValue }
      case 'github-app':
        return { type: 'github-app', installation_id: authValue }
      case 'entra-wif':
        return { type: 'entra-wif', tenant_id: authValue, client_id: authValue2 }
      case 'env-var':
        return { type: 'env-var', name: authValue }
      case 'inline':
        return { type: 'inline', value: authValue }
    }
  }

  async function submit(e: React.FormEvent) {
    e.preventDefault()
    setBusy(true)
    setError(null)
    try {
      const auth = buildAuth()
      let input: ConnectionInput
      if (kind === 'azure-devops') input = { name, kind, org_url: orgUrl, auth }
      else if (kind === 'github') input = { name, kind, org, auth }
      else
        input = {
          name,
          kind: 'llm',
          provider,
          model,
          base_url: baseUrl || undefined,
          is_local: isLocal,
          residency: residency || undefined,
          key: authMethod === 'env-var' && !authValue ? undefined : auth,
        }
      await api.createConnection(input)
      setName('')
      setAuthValue('')
      setAuthValue2('')
      load()
    } catch (e) {
      setError(String(e))
    } finally {
      setBusy(false)
    }
  }

  async function remove(id: string) {
    setError(null)
    try {
      await api.deleteConnection(id)
      load()
    } catch (e) {
      setError(String(e))
    }
  }

  const input =
    'w-full rounded-md border border-ink-800 bg-ink-900 px-3 py-2 text-sm text-ink-100 placeholder:text-ink-500'
  const label = 'mb-1 block text-xs font-medium text-ink-300'

  return (
    <main className="mx-auto w-full max-w-5xl flex-1 px-6 py-6">
      <h1 className="text-xl font-semibold text-ink-100">Connections</h1>
      <p className="mt-1 text-sm text-ink-300">
        Link Azure DevOps orgs, GitHub orgs, and LLM providers. Bifrost stores{' '}
        <span className="text-ink-100">references</span> (Key Vault, GitHub App, Entra) — never
        secret values. An inline secret is encrypted at rest as a fallback.
      </p>

      {error && (
        <div className="mt-4 rounded-lg border border-[var(--color-risk-red)]/40 bg-[var(--color-risk-red)]/10 px-4 py-2 text-sm text-[var(--color-risk-red)]">
          {error}
        </div>
      )}

      {/* Existing connections */}
      <section className="mt-6">
        <h2 className="mb-2 text-sm font-semibold text-ink-100">Configured</h2>
        {!conns ? (
          <div className="text-sm text-ink-300">Loading…</div>
        ) : conns.length === 0 ? (
          <div className="rounded-lg border border-dashed border-ink-800 px-4 py-6 text-center text-sm text-ink-400">
            No connections yet — add one below.
          </div>
        ) : (
          <ul className="space-y-2">
            {conns.map((c) => {
              const auth = c.kind.kind === 'llm' ? c.kind.key : c.kind.auth
              return (
                <li
                  key={c.id}
                  className="flex items-center justify-between rounded-lg border border-ink-800 bg-ink-900/50 px-4 py-3"
                >
                  <div className="min-w-0">
                    <div className="flex items-center gap-2">
                      <span className="font-medium text-ink-100">{c.name}</span>
                      <span className="rounded bg-ink-800 px-1.5 text-[10px] uppercase text-ink-300">
                        {c.kind.kind}
                      </span>
                    </div>
                    <div className="truncate font-mono text-xs text-ink-400">
                      {c.kind.kind === 'azure-devops' && c.kind.org_url}
                      {c.kind.kind === 'github' && c.kind.org}
                      {c.kind.kind === 'llm' && `${c.kind.provider} · ${c.kind.model}`}
                      {' · '}
                      {authSummary(auth)}
                    </div>
                  </div>
                  <button
                    onClick={() => remove(c.id)}
                    className="ml-3 shrink-0 rounded-md border border-ink-800 px-2 py-1 text-xs text-ink-300 hover:border-[var(--color-risk-red)] hover:text-[var(--color-risk-red)]"
                  >
                    Remove
                  </button>
                </li>
              )
            })}
          </ul>
        )}
      </section>

      {/* Add form */}
      <section className="mt-8 rounded-xl border border-ink-800 bg-ink-900/40 p-5">
        <h2 className="mb-4 text-sm font-semibold text-ink-100">Add a connection</h2>
        <form onSubmit={submit} className="grid grid-cols-1 gap-4 sm:grid-cols-2">
          <div>
            <label className={label}>Type</label>
            <select className={input} value={kind} onChange={(e) => setKind(e.target.value as Kind)}>
              {(Object.keys(KIND_LABEL) as Kind[]).map((k) => (
                <option key={k} value={k}>
                  {KIND_LABEL[k]}
                </option>
              ))}
            </select>
          </div>
          <div>
            <label className={label}>Name</label>
            <input className={input} value={name} onChange={(e) => setName(e.target.value)} required placeholder="Prod ADO" />
          </div>

          {kind === 'azure-devops' && (
            <div className="sm:col-span-2">
              <label className={label}>Organization URL</label>
              <input className={input} value={orgUrl} onChange={(e) => setOrgUrl(e.target.value)} required placeholder="https://dev.azure.com/contoso" />
            </div>
          )}
          {kind === 'github' && (
            <div className="sm:col-span-2">
              <label className={label}>GitHub org</label>
              <input className={input} value={org} onChange={(e) => setOrg(e.target.value)} required placeholder="contoso" />
            </div>
          )}
          {kind === 'llm' && (
            <>
              <div>
                <label className={label}>Provider</label>
                <select className={input} value={provider} onChange={(e) => setProvider(e.target.value)}>
                  {['openai-compatible', 'anthropic', 'gemini', 'github-models', 'ollama'].map((p) => (
                    <option key={p} value={p}>
                      {p}
                    </option>
                  ))}
                </select>
              </div>
              <div>
                <label className={label}>Model</label>
                <input className={input} value={model} onChange={(e) => setModel(e.target.value)} required placeholder="gemma-2-12b" />
              </div>
              <div>
                <label className={label}>Base URL (optional)</label>
                <input className={input} value={baseUrl} onChange={(e) => setBaseUrl(e.target.value)} placeholder="http://gemma.vm.internal:8000/v1" />
              </div>
              <div>
                <label className={label}>Data residency (optional)</label>
                <input className={input} value={residency} onChange={(e) => setResidency(e.target.value)} placeholder="eu / on-prem" />
              </div>
              <label className="flex items-center gap-2 text-sm text-ink-200 sm:col-span-2">
                <input type="checkbox" checked={isLocal} onChange={(e) => setIsLocal(e.target.checked)} />
                Runs on infrastructure we control (air-gap eligible)
              </label>
            </>
          )}

          {/* Auth — references first, inline last */}
          <div>
            <label className={label}>{kind === 'llm' ? 'API key' : 'Authentication'}</label>
            <select className={input} value={authMethod} onChange={(e) => setAuthMethod(e.target.value as AuthMethod)}>
              {(Object.keys(AUTH_LABEL) as AuthMethod[]).map((m) => (
                <option key={m} value={m}>
                  {AUTH_LABEL[m]}
                </option>
              ))}
            </select>
          </div>
          <div>
            <label className={label}>
              {authMethod === 'key-vault'
                ? 'Key Vault secret URI'
                : authMethod === 'github-app'
                  ? 'Installation id'
                  : authMethod === 'entra-wif'
                    ? 'Tenant id'
                    : authMethod === 'env-var'
                      ? 'Environment variable name'
                      : 'Secret value (encrypted at rest)'}
            </label>
            <input
              className={input}
              type={authMethod === 'inline' ? 'password' : 'text'}
              value={authValue}
              onChange={(e) => setAuthValue(e.target.value)}
              placeholder={authMethod === 'key-vault' ? 'https://kv.vault.azure.net/secrets/ado-pat' : ''}
            />
          </div>
          {authMethod === 'entra-wif' && (
            <div className="sm:col-span-2">
              <label className={label}>Client id</label>
              <input className={input} value={authValue2} onChange={(e) => setAuthValue2(e.target.value)} />
            </div>
          )}
          {authMethod === 'inline' && (
            <p className="sm:col-span-2 text-xs text-[var(--color-risk-amber)]">
              The value is encrypted with the server key and never shown again. Prefer a Key Vault
              reference where possible.
            </p>
          )}

          <div className="sm:col-span-2">
            <button
              type="submit"
              disabled={busy}
              className="rounded-lg bg-brand-500 px-4 py-2 text-sm font-medium text-ink-950 hover:bg-brand-400 disabled:opacity-60"
            >
              {busy ? 'Saving…' : 'Add connection'}
            </button>
          </div>
        </form>
      </section>
    </main>
  )
}
