import { useEffect, useMemo, useState } from 'react'
import type { Pipeline, Portfolio, RiskBand } from './types'
import { createApi } from './api/client'
import { Header } from './components/Header'
import { StatCards } from './components/StatCards'
import { Heatmap } from './components/Heatmap'
import { PipelineTable } from './components/PipelineTable'
import { PipelineDrawer } from './components/PipelineDrawer'
import { riskMeta } from './lib/format'
import { useTheme } from './lib/theme'

const api = createApi()

type View = 'heatmap' | 'table'
type Filter = RiskBand | 'all'

export default function App() {
  const [portfolio, setPortfolio] = useState<Portfolio | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [selected, setSelected] = useState<Pipeline | null>(null)
  const [view, setView] = useState<View>('heatmap')
  const [filter, setFilter] = useState<Filter>('all')
  const [theme, toggleTheme] = useTheme()

  useEffect(() => {
    api.getPortfolio().then(setPortfolio).catch((e) => setError(String(e)))
  }, [])

  const filtered = useMemo(() => {
    if (!portfolio) return []
    return filter === 'all' ? portfolio.pipelines : portfolio.pipelines.filter((p) => p.riskBand === filter)
  }, [portfolio, filter])

  if (error) {
    return (
      <div className="flex h-full items-center justify-center text-[var(--color-risk-red)]">
        Failed to load portfolio: {error}
      </div>
    )
  }

  if (!portfolio) {
    return (
      <div className="flex h-full items-center justify-center text-ink-300">
        <div className="animate-pulse">Loading portfolio…</div>
      </div>
    )
  }

  return (
    <div className="flex min-h-full flex-col">
      <Header summary={portfolio.summary} theme={theme} onToggleTheme={toggleTheme} />

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

      <footer className="border-t border-ink-800 px-6 py-3 text-center text-xs text-ink-500">
        Mock data — wired to the Rust control plane in M1/M2. Bifrost wraps the official GitHub
        migration tools; it never reimplements their conversion logic.
      </footer>

      <PipelineDrawer pipeline={selected} onClose={() => setSelected(null)} />
    </div>
  )
}
