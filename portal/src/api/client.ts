import type { Portfolio } from '../types'
import { mockPortfolio } from '../data/portfolio'

// The portal depends only on this interface. Today it's backed by mock fixtures;
// once the Rust control plane exists, `HttpBifrostApi` implements the same
// contract against `/api/...` and nothing in the UI changes.
export interface BifrostApi {
  getPortfolio(): Promise<Portfolio>
}

class MockBifrostApi implements BifrostApi {
  async getPortfolio(): Promise<Portfolio> {
    // Simulate a little latency so loading states are exercised.
    await new Promise((r) => setTimeout(r, 350))
    return mockPortfolio
  }
}

class HttpBifrostApi implements BifrostApi {
  private readonly base: string
  constructor(base = '/api') {
    this.base = base
  }

  async getPortfolio(): Promise<Portfolio> {
    const res = await fetch(`${this.base}/portfolio`)
    if (!res.ok) throw new Error(`portfolio request failed: ${res.status}`)
    return (await res.json()) as Portfolio
  }
}

// Flip to the real backend with `VITE_API=http`. Defaults to mock so the portal
// runs standalone with zero backend.
export function createApi(): BifrostApi {
  const mode = import.meta.env.VITE_API ?? 'mock'
  return mode === 'http' ? new HttpBifrostApi() : new MockBifrostApi()
}
