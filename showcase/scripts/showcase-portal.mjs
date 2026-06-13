// Record a video walkthrough of the LIVE Bifrost portal for the showcase.
// Drives system Chrome headless against the live portal (VITE_API=http -> :8099).
import { chromium } from 'playwright-core'
const BASE = process.env.SHOT_BASE ?? 'http://localhost:5173'
const OUT = process.env.OUT_DIR ?? '/tmp/bifrost-show/vid-portal'
const SIZE = { width: 1280, height: 720 }
const executablePath = process.env.CHROME_BIN

const browser = await chromium.launch(
  executablePath ? { executablePath, headless: true } : { channel: 'chrome', headless: true },
)
const ctx = await browser.newContext({
  viewport: SIZE,
  deviceScaleFactor: 1,
  recordVideo: { dir: OUT, size: SIZE },
})
const pg = await ctx.newPage()
await pg.addInitScript(() => {
  localStorage.setItem('bifrost-theme', 'dark')
  localStorage.setItem('bifrost_onboarded', '1')
})

const wait = (ms) => pg.waitForTimeout(ms)
async function nav(name, hold = 3000) {
  try {
    await pg.locator('aside').getByRole('button', { name, exact: true }).click({ timeout: 5000 })
    await wait(hold)
    console.log('nav', name, 'ok')
  } catch (e) { console.log('nav', name, 'MISS', e.message.split('\n')[0]) }
}
async function clickMain(name, hold = 2500) {
  try {
    await pg.getByRole('button', { name, exact: true }).first().click({ timeout: 4000 })
    await wait(hold)
    console.log('tab', name, 'ok')
  } catch (e) { console.log('tab', name, 'MISS', e.message.split('\n')[0]) }
}

await pg.goto(BASE, { waitUntil: 'networkidle' })
await wait(3500)                       // Portfolio heatmap — 3 projects, 6 pipelines

await nav('Program board', 3000)       // the new feature (Board tab default)
await clickMain('roadmap', 3000)       // wave roadmap timeline (DOM text is lowercase)
await clickMain('issues', 3000)        // per-pipeline issues
await clickMain('board', 2400)         // back to the kanban mirror

await nav('Forecast', 3000)
await nav('Readiness', 3000)
await nav('Program', 2600)

await nav('Review', 2600)              // proposals / 3-pane diff
// open the converted Contoso-Payments-CI proposal to reveal the source-vs-converted diff + gaps
try {
  const row = pg.locator('main').getByText('Contoso-Payments-CI', { exact: false }).first()
  await row.scrollIntoViewIfNeeded()
  await row.click({ timeout: 4000 })
  await wait(4500)                     // let Monaco render the 3-pane diff
  console.log('opened proposal panel')
} catch (e) { console.log('open proposal MISS', e.message.split('\n')[0]) }

await nav('Portfolio', 2500)           // end back on the heatmap

await ctx.close()                      // finalize the video
const path = await pg.video()?.path?.().catch(() => null)
await browser.close()
console.log('VIDEO', path || OUT)
