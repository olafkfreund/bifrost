import type { PortfolioSummary } from '../types'
import type { Theme } from '../lib/theme'
import { ThemeToggle } from './ThemeToggle'
import { Logo } from './Logo'

export function Header({
  summary,
  theme,
  onToggleTheme,
}: {
  summary: PortfolioSummary
  theme: Theme
  onToggleTheme: () => void
}) {
  return (
    <header className="border-b border-ink-800 bg-ink-900/60 backdrop-blur">
      <div className="mx-auto flex max-w-7xl items-center gap-4 px-6 py-4">
        <div className="flex items-center gap-3">
          <Logo className="h-7 w-7 text-brand-400" />
          <div className="leading-tight">
            <div className="font-semibold text-ink-100">Bifrost</div>
            <div className="text-xs text-ink-300">ADO → GitHub Actions migration</div>
          </div>
        </div>

        <div className="mx-2 h-8 w-px bg-ink-800" />

        <div className="flex items-center gap-2 text-sm">
          <span className="text-ink-300">org</span>
          <span className="rounded-md bg-ink-800 px-2 py-1 font-mono text-ink-100">{summary.org}</span>
        </div>

        <div className="ml-auto flex items-center gap-3 text-xs">
          {summary.airGap && (
            <span
              title="No pipeline data leaves the network — local model only"
              className="inline-flex items-center gap-1.5 rounded-full border px-2.5 py-1 font-medium"
              style={{
                color: 'var(--color-accent-aqua)',
                borderColor: 'color-mix(in srgb, var(--color-accent-aqua) 35%, transparent)',
                backgroundColor: 'var(--color-accent-aqua-dim)',
              }}
            >
              <span className="h-1.5 w-1.5 rounded-full" style={{ backgroundColor: 'var(--color-accent-aqua)' }} />
              Air-gap mode
            </span>
          )}
          <span
            title="Pinned tool versions for this audit run — recorded for attestation"
            className="hidden items-center gap-2 rounded-full border border-ink-700 bg-ink-850 px-2.5 py-1 font-mono text-ink-300 sm:inline-flex"
          >
            importer {summary.importerVersion}
          </span>
          <ThemeToggle theme={theme} onToggle={onToggleTheme} />
        </div>
      </div>
    </header>
  )
}
