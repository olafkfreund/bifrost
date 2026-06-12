import { useEffect, useRef, useState } from 'react'
import type { BifrostApi } from '../api/client'

type Msg = { role: 'user' | 'assistant'; content: string; provider?: string }

const GREETING: Msg = {
  role: 'assistant',
  content:
    "I'm the migration assistant. Ask me about your portfolio — cost, risk, coverage, or a specific project. I can explain and advise; I can't change anything.",
}

export function ChatPopout({ api }: { api: BifrostApi }) {
  const [open, setOpen] = useState(false)
  const [msgs, setMsgs] = useState<Msg[]>([GREETING])
  const [input, setInput] = useState('')
  const [busy, setBusy] = useState(false)
  const bodyRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    bodyRef.current?.scrollTo({ top: bodyRef.current.scrollHeight, behavior: 'smooth' })
  }, [msgs, open])

  // Esc closes the panel.
  useEffect(() => {
    if (!open) return
    const onKey = (e: KeyboardEvent) => e.key === 'Escape' && setOpen(false)
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [open])

  async function send() {
    const q = input.trim()
    if (!q || busy) return
    setInput('')
    setMsgs((m) => [...m, { role: 'user', content: q }])
    setBusy(true)
    try {
      const { reply, provider } = await api.chat(q)
      setMsgs((m) => [...m, { role: 'assistant', content: reply, provider }])
    } catch (e) {
      setMsgs((m) => [...m, { role: 'assistant', content: `Sorry — ${String(e)}` }])
    } finally {
      setBusy(false)
    }
  }

  if (!open) {
    return (
      <button
        onClick={() => setOpen(true)}
        aria-label="Open migration assistant"
        title="Migration assistant"
        className="fixed bottom-5 right-5 z-40 flex h-12 w-12 items-center justify-center rounded-full border border-ink-700 bg-brand-500 text-ink-950 shadow-[var(--elevation-pop)] transition hover:bg-brand-400"
      >
        <svg width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
          <path d="M21 11.5a8.38 8.38 0 0 1-8.5 8.5 8.5 8.5 0 0 1-3.8-.9L3 21l1.9-5.7A8.38 8.38 0 0 1 4 11.5 8.5 8.5 0 0 1 12.5 3 8.38 8.38 0 0 1 21 11.5z" />
        </svg>
      </button>
    )
  }

  return (
    <div className="bf-dialog fixed bottom-5 right-5 z-40 flex h-[34rem] max-h-[80vh] w-[24rem] max-w-[calc(100vw-2.5rem)] flex-col overflow-hidden rounded-xl">
      {/* header */}
      <div className="flex items-center justify-between border-b border-ink-800 px-4 py-3">
        <div>
          <div className="font-display text-sm font-semibold text-ink-100">Migration assistant</div>
          <div className="text-[11px] text-ink-400">Grounded in your portfolio · query-only</div>
        </div>
        <button onClick={() => setOpen(false)} aria-label="Close" className="rounded-md p-1 text-ink-300 hover:bg-ink-800 hover:text-ink-100">
          ✕
        </button>
      </div>

      {/* messages */}
      <div ref={bodyRef} className="flex-1 space-y-3 overflow-y-auto px-4 py-3">
        {msgs.map((m, i) => (
          <div key={i} className={m.role === 'user' ? 'flex justify-end' : 'flex justify-start'}>
            <div
              className={`max-w-[85%] rounded-lg px-3 py-2 text-sm ${
                m.role === 'user' ? 'bg-brand-500 text-ink-950' : 'bg-ink-850 text-ink-200'
              }`}
            >
              {m.content}
              {m.provider && m.provider !== 'offline-demo' && (
                <div className="mt-1 text-[10px] text-ink-400">via {m.provider}</div>
              )}
            </div>
          </div>
        ))}
        {busy && <div className="text-xs text-ink-400">Thinking…</div>}
      </div>

      {/* input */}
      <div className="border-t border-ink-800 p-3">
        <div className="flex items-end gap-2">
          <textarea
            value={input}
            onChange={(e) => setInput(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter' && !e.shiftKey) {
                e.preventDefault()
                send()
              }
            }}
            rows={1}
            placeholder="Ask about cost, risk, coverage…"
            className="bf-field max-h-28 flex-1 resize-none px-3 py-2 text-sm"
          />
          <button
            onClick={send}
            disabled={busy || !input.trim()}
            className="rounded-lg bg-brand-500 px-3 py-2 text-sm font-medium text-ink-950 transition hover:bg-brand-400 disabled:opacity-40"
          >
            Send
          </button>
        </div>
      </div>
    </div>
  )
}
