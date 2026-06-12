import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
// Self-hosted variable fonts (bundled by Vite — no runtime CDN, so air-gap safe).
// Mona Sans + Hubot Sans are GitHub's own typefaces; fitting for a GitHub
// migration tool. Weight axis only, to keep the bundle lean.
import '@fontsource-variable/mona-sans/wght.css'
import '@fontsource-variable/hubot-sans/wght.css'
import '@fontsource-variable/jetbrains-mono/wght.css'
import './index.css'
import App from './App.tsx'

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <App />
  </StrictMode>,
)
