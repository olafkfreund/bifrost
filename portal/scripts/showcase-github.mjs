// Record a video tour of the PUBLIC GitHub artifacts Bifrost created.
import { chromium } from 'playwright-core'
const OUT = process.env.OUT_DIR ?? '/tmp/bifrost-show/vid-github'
const SIZE = { width: 1280, height: 720 }
const executablePath = process.env.CHROME_BIN

const browser = await chromium.launch(
  executablePath ? { executablePath, headless: true } : { channel: 'chrome', headless: true },
)
const ctx = await browser.newContext({ viewport: SIZE, deviceScaleFactor: 1, recordVideo: { dir: OUT, size: SIZE } })
const pg = await ctx.newPage()
const wait = (ms) => pg.waitForTimeout(ms)

async function dismissCookies() {
  for (const name of [/reject all/i, /only necessary/i, /accept/i]) {
    try { await pg.getByRole('button', { name }).first().click({ timeout: 1500 }); break } catch {}
  }
}
async function visit(url, { hold = 4000, scroll = 0 } = {}) {
  try {
    await pg.goto(url, { waitUntil: 'domcontentloaded', timeout: 30000 })
    await wait(1500); await dismissCookies(); await wait(hold)
    if (scroll) { await pg.mouse.wheel(0, scroll); await wait(2200) }
    console.log('visited', url)
  } catch (e) { console.log('visit MISS', url, e.message.split('\n')[0]) }
}

// 1. repo home (README + azure-pipelines/ + the open PRs)
await visit('https://github.com/olafkfreund/contoso-payments', { hold: 4000, scroll: 400 })
// 2. the two Bifrost PRs
await visit('https://github.com/olafkfreund/contoso-payments/pulls', { hold: 3500 })
// 3. the converted workflow diff — shows the gap comments
await visit('https://github.com/olafkfreund/contoso-payments/pull/1/files', { hold: 4500, scroll: 500 })
// 4. the other two migrated repos
await visit('https://github.com/olafkfreund/northwind-logistics/pull/1/files', { hold: 3500, scroll: 400 })
await visit('https://github.com/olafkfreund/fabrikam-identity', { hold: 3500 })
// 5. the public program board
await visit('https://github.com/users/olafkfreund/projects/8', { hold: 4500 })

await ctx.close()
const path = await pg.video()?.path?.().catch(() => null)
await browser.close()
console.log('VIDEO', path || OUT)
