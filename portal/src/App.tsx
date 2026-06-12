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
import { Routing } from './components/Routing'
import { OnboardingWizard } from './components/OnboardingWizard'
import { riskMeta } from './lib/format'
import { useTheme } from './lib/theme'

const api = createApi()

type View = 'heatmap' | 'table'
type Filter = RiskBand | 'all'
type Page = 'portfolio' | 'review' | 'connections' | 'routing' | 'docs'

/** Slugify a project name for use in a download filename. */
function slugify(s: string): string {
  return s
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')
}

/** Trigger a browser download of a blob with the given filename. */
function downloadBlob(blob: Blob, filename: string) {
  const url = URL.createObjectURL(blob)
  const a = document.createElement('a')
  a.href = url
  a.download = filename
  a.click()
  URL.revokeObjectURL(url)
}

export default function App() {
  const [portfolio, setPortfolio] = useState<Portfolio | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [selected, setSelected] = useState<Pipeline | null>(null)
  const [proposalFor, setProposalFor] = useState<Pipeline | null>(null)
  const [view, setView] = useState<View>('heatmap')
  const [filter, setFilter] = useState<Filter>('all')
  const [orgFilter, setOrgFilter] = useState<string>('all')
  const [reportProject, setReportProject] = useState<string>('')
  const [page, setPage] = useState<Page>('portfolio')
  const [showWizard, setShowWizard] = useState(() => !localStorage.getItem('bifrost_onboarded'))
  const [theme, toggleTheme, togglePalette] = useTheme()

  useEffect(() => {
    api.getPortfolio().then(setPortfolio).catch((e) => setError(String(e)))
  }, [])

  // Distinct source orgs (multi-org, #157). Empty/absent orgs are single-org.
  const orgs = useMemo(() => {
    if (!portfolio) return []
    return [...new Set(portfolio.pipelines.map((p) => p.org).filter((o): o is string => !!o))].sort()
  }, [portfolio])

  // Distinct projects, for the per-project report (#222). Different projects can
  // have different owners / change boards, so a report can be scoped to one.
  const projects = useMemo(() => {
    if (!portfolio) return []
    return [...new Set(portfolio.pipelines.map((p) => p.project).filter((p): p is string => !!p))].sort()
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
        onTogglePalette={togglePalette}
        page={page}
        onNavigate={setPage}
      />

      {page === 'docs' ? (
        <DocsPage />
      ) : page === 'connections' ? (
        <Connections api={api} />
      ) : page === 'routing' ? (
        <Routing api={api} />
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
                  className="bf-field px-3 py-1.5 text-xs"
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

              {/* pre-migration status report (#204/#220/#221/#222) — review before any change.
                  Scopeable to one project, since projects can have different owners / change boards. */}
              <div className="flex overflow-hidden rounded-lg border border-ink-800 text-xs">
                <span className="px-2 py-1.5 text-ink-400">Report</span>
                {projects.length > 1 && (
                  <select
                    value={reportProject}
                    onChange={(e) => setReportProject(e.target.value)}
                    className="border-l border-ink-800 bg-ink-900 px-2 py-1.5 text-ink-100"
                    title="Scope the report to one project (its owner / change board)"
                  >
                    <option value="">Whole estate</option>
                    {projects.map((p) => (
                      <option key={p} value={p}>
                        {p}
                      </option>
                    ))}
                  </select>
                )}
                <button
                  onClick={() => {
                    const slug = reportProject ? `-${slugify(reportProject)}` : ''
                    api
                      .getReport(reportProject || undefined)
                      .then((md) =>
                        downloadBlob(new Blob([md], { type: 'text/markdown' }), `migration-status-report${slug}.md`),
                      )
                      .catch((e) => setError(String(e)))
                  }}
                  title="Download the pre-migration status report (Markdown)"
                  className="border-l border-ink-800 px-3 py-1.5 text-ink-200 transition hover:bg-ink-850 hover:text-ink-100"
                >
                  .md
                </button>
                <button
                  onClick={() => {
                    const slug = reportProject ? `-${slugify(reportProject)}` : ''
                    api
                      .getReportPdf(reportProject || undefined)
                      .then((pdf) => downloadBlob(pdf, `migration-status-report${slug}.pdf`))
                      .catch((e) => setError(String(e)))
                  }}
                  title="Download the pre-migration status report (PDF) for the change board"
                  className="border-l border-ink-800 px-3 py-1.5 text-ink-200 transition hover:bg-ink-850 hover:text-ink-100"
                >
                  .pdf
                </button>
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
