import { useEffect, useState } from 'react'
import type { BifrostApi, ConnectionInput } from '../api/client'
import type { ConnectionView, SecretRefView } from '../types'

type Kind = 'azure-devops' | 'github' | 'llm' | 'source'
type AuthMethod = 'key-vault' | 'github-app' | 'entra-wif' | 'env-var' | 'inline'

const KIND_LABEL: Record<Kind, string> = {
  'azure-devops': 'Azure DevOps',
  source: 'CI/CD source (Jenkins, GitLab, …)',
  github: 'GitHub',
  llm: 'LLM provider',
}

// CI/CD sources to migrate. base-url meaning + whether a username is needed
// varies per platform; Bitbucket is discovery-only (the Importer can't convert it).
const SOURCE_PLATFORMS: { id: string; label: string; urlLabel: string; needsUser: boolean }[] = [
  { id: 'jenkins', label: 'Jenkins', urlLabel: 'Server URL', needsUser: true },
  { id: 'gitlab', label: 'GitLab', urlLabel: 'Server URL (blank = gitlab.com)', needsUser: false },
  { id: 'bitbucket', label: 'Bitbucket (discovery only)', urlLabel: 'Workspace', needsUser: true },
  { id: 'circleci', label: 'CircleCI', urlLabel: 'Instance URL (blank = circleci.com)', needsUser: false },
  { id: 'travis', label: 'Travis CI', urlLabel: 'Instance URL (blank = api.travis-ci.com)', needsUser: false },
  { id: 'bamboo', label: 'Bamboo', urlLabel: 'Server URL', needsUser: false },
]

// Vault/identity references first; pasting the key/token directly (encrypted at
// rest) is the simple option for getting started without a vault.
const AUTH_LABEL: Record<AuthMethod, string> = {
  'key-vault': 'Azure Key Vault reference (recommended)',
  'github-app': 'GitHub App installation',
  'entra-wif': 'Entra workload identity',
  'env-var': 'Environment variable',
  inline: 'Paste key / token (encrypted at rest)',
}

// LLM providers: hosted frontier providers need only a key; the configurable /
// local ones also take a base URL and an air-gap flag. `keyHint` explains what
// the "key" is for each.
const LLM_PROVIDERS: Record<string, { label: string; hosted: boolean; keyHint: string }> = {
  anthropic: { label: 'Anthropic (Claude)', hosted: true, keyHint: 'Your Anthropic API key (sk-ant-…).' },
  gemini: { label: 'Google Gemini (AI Studio)', hosted: true, keyHint: 'Your Google AI Studio API key.' },
  'github-models': {
    label: 'GitHub Copilot / Models',
    hosted: true,
    keyHint: 'A GitHub token with the models scope (your Copilot/Models subscription).',
  },
  'openai-compatible': {
    label: 'OpenAI-compatible (vLLM, LiteLLM, Azure OpenAI, Bedrock gateway…)',
    hosted: false,
    keyHint: 'The gateway/API key, if the endpoint requires one (local servers often don’t).',
  },
  ollama: { label: 'Ollama (local)', hosted: false, keyHint: 'Usually none — Ollama is keyless.' },
}

// Known, ready-to-use model ids per provider for the model picker. The
// configurable/local providers (openai-compatible, ollama) are deployment-
// specific, so they stay free-text. "Custom…" lets you type any id.
const LLM_MODELS: Record<string, string[]> = {
  anthropic: [
    'claude-opus-4-8',
    'claude-sonnet-4-6',
    'claude-haiku-4-5',
    'claude-opus-4-7',
    'claude-opus-4-6',
  ],
  gemini: ['gemini-2.5-flash', 'gemini-2.5-pro', 'gemini-2.0-flash'],
  'github-models': ['gpt-4o', 'gpt-4o-mini', 'o1', 'o1-mini', 'llama-3.3-70b-instruct'],
}
const CUSTOM_MODEL = '__custom'

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
  const [platform, setPlatform] = useState('jenkins')
  const [username, setUsername] = useState('')
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

  // Switch provider and default to its first known model (clearing it for the
  // deployment-specific providers).
  function selectProvider(p: string) {
    setProvider(p)
    setModel(LLM_MODELS[p]?.[0] ?? '')
  }

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
      else if (kind === 'source')
        input = {
          name,
          kind: 'source',
          platform,
          base_url: baseUrl || undefined,
          username: username || undefined,
          auth,
        }
      else
        input = {
          name,
          kind: 'llm',
          provider,
          model,
          base_url: baseUrl || undefined,
          is_local: isLocal,
          key: authMethod === 'env-var' && !authValue ? undefined : auth,
        }
      await api.createConnection(input)
      setName('')
      setAuthValue('')
      setAuthValue2('')
      setUsername('')
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
        Link Azure DevOps, the other CI/CD sources to migrate (Jenkins, GitLab, CircleCI, Travis,
        Bamboo), GitHub orgs, and LLM providers. Bifrost stores{' '}
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
              const sourceDetail =
                c.kind.kind === 'source'
                  ? `${c.kind.platform}${c.kind.base_url ? ` · ${c.kind.base_url}` : ''}`
                  : ''
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
                      {c.kind.kind === 'source' && sourceDetail}
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
          {kind === 'source' &&
            (() => {
              const spec = SOURCE_PLATFORMS.find((p) => p.id === platform) ?? SOURCE_PLATFORMS[0]
              return (
                <>
                  <div>
                    <label className={label}>Platform</label>
                    <select className={input} value={platform} onChange={(e) => setPlatform(e.target.value)}>
                      {SOURCE_PLATFORMS.map((p) => (
                        <option key={p.id} value={p.id}>
                          {p.label}
                        </option>
                      ))}
                    </select>
                  </div>
                  <div>
                    <label className={label}>{spec.urlLabel}</label>
                    <input
                      className={input}
                      value={baseUrl}
                      onChange={(e) => setBaseUrl(e.target.value)}
                      placeholder={platform === 'bitbucket' ? 'my-workspace' : 'https://ci.example.com'}
                    />
                  </div>
                  {spec.needsUser && (
                    <div className="sm:col-span-2">
                      <label className={label}>Username</label>
                      <input
                        className={input}
                        value={username}
                        onChange={(e) => setUsername(e.target.value)}
                        placeholder="ci-bot"
                      />
                    </div>
                  )}
                </>
              )
            })()}
          {kind === 'llm' &&
            (() => {
              const spec = LLM_PROVIDERS[provider] ?? LLM_PROVIDERS['anthropic']
              return (
                <>
                  <div>
                    <label className={label}>Provider</label>
                    <select className={input} value={provider} onChange={(e) => selectProvider(e.target.value)}>
                      {Object.entries(LLM_PROVIDERS).map(([id, p]) => (
                        <option key={id} value={id}>
                          {p.label}
                        </option>
                      ))}
                    </select>
                  </div>
                  <div>
                    <label className={label}>Model</label>
                    {LLM_MODELS[provider] ? (
                      <>
                        <select
                          className={input}
                          value={LLM_MODELS[provider].includes(model) ? model : CUSTOM_MODEL}
                          onChange={(e) => setModel(e.target.value === CUSTOM_MODEL ? '' : e.target.value)}
                        >
                          {LLM_MODELS[provider].map((m) => (
                            <option key={m} value={m}>
                              {m}
                            </option>
                          ))}
                          <option value={CUSTOM_MODEL}>Custom…</option>
                        </select>
                        {!LLM_MODELS[provider].includes(model) && (
                          <input
                            className={`${input} mt-2`}
                            value={model}
                            onChange={(e) => setModel(e.target.value)}
                            required
                            placeholder="model id"
                          />
                        )}
                      </>
                    ) : (
                      <input
                        className={input}
                        value={model}
                        onChange={(e) => setModel(e.target.value)}
                        required
                        placeholder="gemma-2-12b"
                      />
                    )}
                  </div>
                  {/* Base URL + air-gap only apply to configurable/local endpoints. */}
                  {!spec.hosted && (
                    <>
                      <div className="sm:col-span-2">
                        <label className={label}>
                          {provider === 'ollama' ? 'Ollama base URL' : 'Base URL (the endpoint, incl. /v1)'}
                        </label>
                        <input
                          className={input}
                          value={baseUrl}
                          onChange={(e) => setBaseUrl(e.target.value)}
                          placeholder="http://gemma.vm.internal:8000/v1"
                        />
                      </div>
                      <label className="flex items-center gap-2 text-sm text-ink-200 sm:col-span-2">
                        <input type="checkbox" checked={isLocal} onChange={(e) => setIsLocal(e.target.checked)} />
                        In-network endpoint (air-gap eligible — usable when air-gap is on)
                      </label>
                    </>
                  )}
                </>
              )
            })()}

          {/* Auth — references first; "Paste key / token" is the no-vault option. */}
          <div>
            <label className={label}>{kind === 'llm' ? 'API key' : 'Authentication'}</label>
            <select className={input} value={authMethod} onChange={(e) => setAuthMethod(e.target.value as AuthMethod)}>
              {(Object.keys(AUTH_LABEL) as AuthMethod[]).map((m) => (
                <option key={m} value={m}>
                  {AUTH_LABEL[m]}
                </option>
              ))}
            </select>
            {kind === 'llm' && (
              <p className="mt-1 text-xs text-ink-400">
                {LLM_PROVIDERS[provider]?.keyHint} To paste it directly, choose{' '}
                <span className="text-ink-200">Paste key / token</span>.
              </p>
            )}
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
                      : kind === 'llm' || kind === 'source'
                        ? 'API key / token (encrypted at rest)'
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
