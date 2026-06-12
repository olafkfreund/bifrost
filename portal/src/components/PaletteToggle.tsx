import type { Theme } from '../lib/theme'
import { paletteOf } from '../lib/theme'

/** Switches the palette family (Gruvbox ↔ shadcn) while keeping the current
 * light/dark mode. Sits next to the mode toggle; Gruvbox stays the default. */
export function PaletteToggle({ theme, onToggle }: { theme: Theme; onToggle: () => void }) {
  const palette = paletteOf(theme)
  const next = palette === 'gruvbox' ? 'shadcn' : 'gruvbox'
  return (
    <button
      onClick={onToggle}
      title={`Switch to ${next} palette`}
      aria-label={`Switch to ${next} palette`}
      className="inline-flex h-8 items-center gap-1.5 rounded-lg border border-ink-700 bg-ink-850 px-3 text-xs font-medium text-ink-200 transition hover:border-ink-600 hover:text-ink-100"
    >
      {/* swatch */}
      <span className="inline-flex h-3 w-3 items-center justify-center rounded-sm bg-brand-500" />
      <span className="capitalize">{palette}</span>
    </button>
  )
}
