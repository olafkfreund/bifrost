import type { PortfolioSummary } from '../types'
import type { Theme } from '../lib/theme'
import { ThemeToggle } from './ThemeToggle'
import { PaletteToggle } from './PaletteToggle'
import { Logo } from './Logo'

/** Slim top bar: identity (logo + slogan), the audited org, the air-gap status,
 * and the appearance toggles. Page navigation lives in the left Sidebar. */
export function Header({
  summary,
  theme,
  onToggleTheme,
  onTogglePalette,
}: {
  summary: PortfolioSummary | null
  theme: Theme
  onToggleTheme: () => void
  onTogglePalette: () => void
}) {
  return (
    <header className="border-b border-ink-800 bg-ink-900/60 backdrop-blur">
      <div className="flex items-center gap-4 px-5 py-3">
        <div className="flex items-center gap-3">
          <Logo className="h-7 w-7 text-brand-400" />
          <div className="leading-tight">
            <div className="font-display text-[15px] font-semibold tracking-tight text-ink-100">Bifrost</div>
            <div className="text-xs text-ink-300">Azure DevOps · Jenkins · GitLab → GitHub Actions</div>
          </div>
        </div>

        {summary && (
          <>
            <div className="mx-1 h-8 w-px bg-ink-800" />
            <div className="flex items-center gap-2 text-sm">
              <span className="text-ink-300">org</span>
              <span className="tnum rounded-md bg-ink-800 px-2 py-1 font-mono text-ink-100">{summary.org}</span>
            </div>
          </>
        )}

        <div className="ml-auto flex items-center gap-2.5 text-xs">
          {summary?.airGap && (
            <span
              title="No public egress — only in-network providers are used"
              className="inline-flex shrink-0 items-center gap-1.5 whitespace-nowrap rounded-lg border px-2.5 py-1 font-medium"
              style={{
                color: 'var(--color-accent-aqua)',
                borderColor: 'color-mix(in srgb, var(--color-accent-aqua) 35%, transparent)',
                backgroundColor: 'var(--color-accent-aqua-dim)',
              }}
            >
              <span className="h-1.5 w-1.5 shrink-0 rounded-full" style={{ backgroundColor: 'var(--color-accent-aqua)' }} />
              Air-gap
            </span>
          )}
          <PaletteToggle theme={theme} onToggle={onTogglePalette} />
          <ThemeToggle theme={theme} onToggle={onToggleTheme} />
        </div>
      </div>
    </header>
  )
}
