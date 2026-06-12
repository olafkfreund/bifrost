// Capture portal screenshots for the docs showcase.
//
// Drives the installed Google Chrome (channel: 'chrome' — no browser download)
// against a mock-data Vite instance, so the shots use the synthetic `contoso`
// demo portfolio (rich + privacy-safe), never a real org. Run:
//
//   VITE_API=mock npm run dev -- --port 5174        # in one shell
//   node scripts/screenshots.mjs                    # in another
//
// Output → ../docs/assets/screenshots/*.png
import { chromium } from 'playwright-core'
import { fileURLToPath } from 'node:url'
import { dirname, resolve } from 'node:path'

const __dirname = dirname(fileURLToPath(import.meta.url))
const OUT = resolve(__dirname, '../../docs/assets/screenshots')
const BASE = process.env.SHOT_BASE ?? 'http://localhost:5174'
const VIEWPORT = { width: 1440, height: 960 }

// Use the system Chrome (NixOS path varies); override with CHROME_BIN.
const executablePath = process.env.CHROME_BIN
const browser = await chromium.launch(
  executablePath ? { executablePath, headless: true } : { channel: 'chrome', headless: true },
)

async function newPage(theme, { onboarded = true } = {}) {
  const ctx = await browser.newContext({ viewport: VIEWPORT, deviceScaleFactor: 2 })
  const pg = await ctx.newPage()
  await pg.addInitScript(
    ([t, ob]) => {
      localStorage.setItem('bifrost-theme', t)
      if (ob) localStorage.setItem('bifrost_onboarded', '1')
    },
    [theme, onboarded],
  )
  return { ctx, pg }
}

async function load(pg) {
  await pg.goto(BASE, { waitUntil: 'networkidle' })
  await pg.waitForTimeout(700)
}

const nav = (pg, name) => pg.locator('aside').getByRole('button', { name, exact: true }).click()
const shot = (pg, name, fullPage = false) =>
  pg.screenshot({ path: `${OUT}/${name}.png`, fullPage })

async function run() {
  // --- Gruvbox dark (primary): every view ---
  {
    const { ctx, pg } = await newPage('dark')
    await load(pg)
    await shot(pg, 'portfolio-heatmap')

    // table view
    await pg.getByRole('button', { name: /^table$/i }).click()
    await pg.waitForTimeout(300)
    await shot(pg, 'portfolio-table')
    await pg.getByRole('button', { name: /^heatmap$/i }).click()
    await pg.waitForTimeout(300)

    // pipeline drawer (risk factors) — click a known red tile
    await pg.getByRole('button', { name: /Reindex/i }).first().click()
    await pg.waitForTimeout(500)
    await shot(pg, 'risk-factors-panel')
    await pg.keyboard.press('Escape')
    await pg.waitForTimeout(300)

    // review queue
    await nav(pg, 'Review')
    await pg.waitForTimeout(500)
    await shot(pg, 'review-queue', true)

    // proposal review (three-pane) — open the first in-review row
    await pg.getByText('data-etl · Deploy', { exact: true }).first().click()
    await pg.waitForTimeout(2200) // Monaco diff render
    await shot(pg, 'proposal-review')
    await pg.keyboard.press('Escape')
    await pg.waitForTimeout(300)

    // settings: connections
    await nav(pg, 'Connections')
    await pg.waitForTimeout(400)
    await shot(pg, 'connections')

    // settings: routing
    await nav(pg, 'Routing')
    await pg.waitForTimeout(400)
    await shot(pg, 'routing')

    // docs & help
    await nav(pg, 'Docs & Help')
    await pg.waitForTimeout(400)
    await shot(pg, 'docs-help')

    await ctx.close()
  }

  // --- Onboarding wizard (fresh — not onboarded) ---
  {
    const { ctx, pg } = await newPage('dark', { onboarded: false })
    await load(pg)
    await pg.waitForTimeout(600)
    await shot(pg, 'onboarding-wizard')
    await ctx.close()
  }

  // --- Theme showcase: same portfolio across the other three themes ---
  for (const [theme, name] of [
    ['light', 'portfolio-gruvbox-light'],
    ['shadcn-dark', 'portfolio-shadcn-dark'],
    ['shadcn-light', 'portfolio-shadcn-light'],
  ]) {
    const { ctx, pg } = await newPage(theme)
    await load(pg)
    await shot(pg, name)
    await ctx.close()
  }
}

try {
  await run()
  console.log('screenshots written to', OUT)
} finally {
  await browser.close()
}
