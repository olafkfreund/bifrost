import { useEffect, useMemo, useState } from 'react'
import type { Pipeline, Portfolio, RiskBand } from './types'
import { createApi } from './api/client'
import { Header } from './components/Header'
import { StatCards } from './components/StatCards'
import { Heatmap } from './components/Heatmap'
import { PipelineTable } from './components/PipelineTable'
import { PipelineDrawer } from './components/PipelineDrawer'
import { ProposalPanel } from './components/ProposalPanel'
import { DocsPage } from './components/DocsPage'
import { ReviewQueue } from './components/ReviewQueue'
import { Connections } from './components/Connections'
import { OnboardingWizard } from './components/OnboardingWizard'
import { riskMeta } from './lib/format'
import { useTheme } from './lib/theme'

const api = createApi()

type View = 'heatmap' | 'table'
type Filter = RiskBand | 'all'
type Page = 'portfolio' | 'review' | 'connections' | 'docs'

export default function App() {
  const [portfolio, setPortfolio] = useState<Portfolio | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [selected, setSelected] = useState<Pipeline | null>(null)
  const [proposalFor, setProposalFor] = useState<Pipeline | null>(null)
  const [view, setView] = useState<View>('heatmap')
  const [filter, setFilter] = useState<Filter>('all')
  const [orgFilter, setOrgFilter] = useState<string>('all')
  const [page, setPage] = useState<Page>('portfolio')
  const [showWizard, setShowWizard] = useState(() => !localStorage.getItem('bifrost_onboarded'))
  const [theme, toggleTheme] = useTheme()

  useEffect(() => {
    api.getPortfolio().then(setPortfolio).catch((e) => setError(String(e)))
  }, [])

  // Distinct source orgs (multi-org, #157). Empty/absent orgs are single-org.
  const orgs = useMemo(() => {
    if (!portfolio) return []
    return [...new Set(portfolio.pipelines.map((p) => p.org).filter((o): o is string => !!o))].sort()
  }, [portfolio])

  const filtered = useMemo(() => {
    if (!portfolio) return []
    return portfolio.pipelines
      .filter((p) => filter === 'all' || p.riskBand === filter)
      .filter((p) => orgFilter === 'all' || p.org === orgFilter)
  }, [portfolio, filter, orgFilter])

  return (
    <div className="flex min-h-full flex-col">
      <Header
        summary={portfolio?.summary ?? null}
        theme={theme}
        onToggleTheme={toggleTheme}
        page={page}
        onNavigate={setPage}
      />

      {page === 'docs' ? (
        <DocsPage />
      ) : page === 'connections' ? (
        <Connections api={api} />
      ) : error ? (
        <div className="flex flex-1 items-center justify-center text-[var(--color-risk-red)]">
          Failed to load portfolio: {error}
        </div>
      ) : !portfolio ? (
        <div className="flex flex-1 items-center justify-center text-ink-300">
          <div className="animate-pulse">Loading portfolio…</div>
        </div>
      ) : page === 'review' ? (
        <ReviewQueue
          pipelines={portfolio.pipelines}
          api={api}
          onSelect={setProposalFor}
          onRefresh={() => api.getPortfolio().then(setPortfolio).catch((e) => setError(String(e)))}
        />
      ) : (
        <main className="mx-auto w-full max-w-7xl flex-1 px-6 py-6">
          <div className="mb-5 flex flex-wrap items-end justify-between gap-3">
            <div>
              <h1 className="text-xl font-semibold text-ink-100">Portfolio</h1>
              <p className="text-sm text-ink-300">
                Migration risk across {portfolio.summary.totals.pipelines} pipelines · generated{' '}
                {new Date(portfolio.summary.generatedAt).toLocaleDateString()}
              </p>
            </div>

            <div className="flex items-center gap-2">
              {/* org switcher — only when the tenant spans multiple orgs (#157) */}
              {orgs.length > 1 && (
                <select
                  value={orgFilter}
                  onChange={(e) => setOrgFilter(e.target.value)}
                  className="rounded-lg border border-ink-800 bg-ink-900 px-3 py-1.5 text-xs text-ink-100"
                  title="Filter by source org"
                >
                  <option value="all">All orgs ({orgs.length})</option>
                  {orgs.map((o) => (
                    <option key={o} value={o}>
                      {o}
                    </option>
                  ))}
                </select>
              )}

              {/* risk filter */}
              <div className="flex overflow-hidden rounded-lg border border-ink-800 text-xs">
                {(['all', 'green', 'amber', 'red'] as Filter[]).map((b) => (
                  <button
                    key={b}
                    onClick={() => setFilter(b)}
                    className={`px-3 py-1.5 capitalize transition ${
                      filter === b ? 'bg-ink-800 text-ink-100' : 'text-ink-300 hover:bg-ink-850'
                    } ${b !== 'all' ? riskMeta[b as RiskBand].text : ''}`}
                  >
                    {b}
                  </button>
                ))}
              </div>

              {/* view toggle */}
              <div className="flex overflow-hidden rounded-lg border border-ink-800 text-xs">
                {(['heatmap', 'table'] as View[]).map((v) => (
                  <button
                    key={v}
                    onClick={() => setView(v)}
                    className={`px-3 py-1.5 capitalize transition ${
                      view === v ? 'bg-ink-800 text-ink-100' : 'text-ink-300 hover:bg-ink-850'
                    }`}
                  >
                    {v}
                  </button>
                ))}
              </div>
            </div>
          </div>

          <StatCards summary={portfolio.summary} />

          <div className="mt-6">
            {view === 'heatmap' ? (
              <Heatmap pipelines={filtered} onSelect={setSelected} />
            ) : (
              <PipelineTable pipelines={filtered} onSelect={setSelected} />
            )}
          </div>
        </main>
      )}

      <footer className="border-t border-ink-800 px-6 py-3 text-center text-xs text-ink-500">
        Risk is computed deterministically from the Importer audit + source inventory. Bifrost
        wraps the official GitHub migration tools; it never reimplements their conversion logic.
        {' · '}
        <button onClick={() => setShowWizard(true)} className="underline hover:text-ink-300">
          Run setup
        </button>
      </footer>

      {showWizard && (
        <OnboardingWizard
          api={api}
          onClose={() => setShowWizard(false)}
          onGoConnections={() => {
            setPage('connections')
            setShowWizard(false)
          }}
        />
      )}

      <PipelineDrawer
        pipeline={selected}
        onClose={() => setSelected(null)}
        onOpenProposal={(p) => {
          setSelected(null)
          setProposalFor(p)
        }}
      />
      <ProposalPanel pipeline={proposalFor} api={api} theme={theme} onClose={() => setProposalFor(null)} />
    </div>
  )
}
