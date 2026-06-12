import { useEffect, useState } from 'react'

/** Theme = palette family × light/dark mode, encoded as the `data-theme` value.
 * Gruvbox keeps the bare `dark`/`light` values (backward-compatible with stored
 * prefs and the default `@theme` block); shadcn is the added palette. */
export type Theme = 'dark' | 'light' | 'shadcn-dark' | 'shadcn-light'
export type Palette = 'gruvbox' | 'shadcn'
export type Mode = 'dark' | 'light'

const KEY = 'bifrost-theme'
const ALL: Theme[] = ['dark', 'light', 'shadcn-dark', 'shadcn-light']

/** Whether a theme renders on a light surface. */
export function isLight(theme: Theme): boolean {
  return theme === 'light' || theme === 'shadcn-light'
}

/** The palette family of a theme. */
export function paletteOf(theme: Theme): Palette {
  return theme.startsWith('shadcn') ? 'shadcn' : 'gruvbox'
}

/** Compose a `data-theme` value from a palette family + mode. */
function compose(palette: Palette, mode: Mode): Theme {
  if (palette === 'shadcn') return mode === 'light' ? 'shadcn-light' : 'shadcn-dark'
  return mode
}

export function getInitialTheme(): Theme {
  const stored = localStorage.getItem(KEY)
  if (stored && (ALL as string[]).includes(stored)) return stored as Theme
  // Respect the OS preference on first visit; default to Gruvbox dark otherwise.
  const light = window.matchMedia?.('(prefers-color-scheme: light)').matches
  return light ? 'light' : 'dark'
}

export function applyTheme(theme: Theme): void {
  document.documentElement.dataset.theme = theme
}

/** Theme state synced to <html data-theme> and localStorage. Returns the current
 * theme plus a mode toggle (light↔dark) and a palette toggle (Gruvbox↔shadcn),
 * each preserving the other dimension. */
export function useTheme(): [Theme, () => void, () => void] {
  const [theme, setTheme] = useState<Theme>(getInitialTheme)

  useEffect(() => {
    applyTheme(theme)
    localStorage.setItem(KEY, theme)
  }, [theme])

  const toggleMode = () =>
    setTheme((t) => compose(paletteOf(t), isLight(t) ? 'dark' : 'light'))
  const togglePalette = () =>
    setTheme((t) =>
      compose(paletteOf(t) === 'gruvbox' ? 'shadcn' : 'gruvbox', isLight(t) ? 'light' : 'dark'),
    )

  return [theme, toggleMode, togglePalette]
}
