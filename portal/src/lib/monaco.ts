// Monaco wiring for the review diff.
//
// Bifrost must stay air-gap-capable, so we never let @monaco-editor/react pull
// the editor from a CDN at runtime — we hand it the locally-installed
// `monaco-editor` and bundle its worker through Vite. No network at runtime.
//
// We import the editor *API* (not the full `monaco-editor` meta-package, which
// would bundle every language + the TS/JSON/CSS/HTML language services) and add
// just the YAML tokenizer. That keeps the bundle small while staying entirely
// local — YAML needs only the built-in tokenizer, so the base editor worker is
// the only worker required.
import * as monaco from 'monaco-editor/esm/vs/editor/editor.api'
import 'monaco-editor/esm/vs/basic-languages/yaml/yaml.contribution'
import editorWorker from 'monaco-editor/esm/vs/editor/editor.worker?worker'
import { loader } from '@monaco-editor/react'

// Gruvbox-flavoured themes so the diff matches the rest of the portal.
monaco.editor.defineTheme('bifrost-dark', {
  base: 'vs-dark',
  inherit: true,
  rules: [],
  colors: {
    'editor.background': '#1d2021',
    'editor.foreground': '#ebdbb2',
    'editorLineNumber.foreground': '#665c54',
    'editorGutter.background': '#1d2021',
    'diffEditor.insertedTextBackground': '#b8bb2622',
    'diffEditor.removedTextBackground': '#fb493422',
  },
})
monaco.editor.defineTheme('bifrost-light', {
  base: 'vs',
  inherit: true,
  rules: [],
  colors: {
    'editor.background': '#fbf1c7',
    'editor.foreground': '#3c3836',
    'editorLineNumber.foreground': '#bdae93',
    'editorGutter.background': '#fbf1c7',
    'diffEditor.insertedTextBackground': '#79740e22',
    'diffEditor.removedTextBackground': '#9d000622',
  },
})

;(self as unknown as { MonacoEnvironment: monaco.Environment }).MonacoEnvironment = {
  getWorker: () => new editorWorker(),
}

loader.config({ monaco })

export { monaco }
