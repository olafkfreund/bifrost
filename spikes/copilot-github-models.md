# Spike: Copilot / GitHub Models as an LLM provider (#18)

> Status: Done · Milestone: M0 · Type: spike
> Outcome: **Usable provider via the GitHub Models API** — implemented as `bifrost-llm/src/copilot.rs` (#41), kept opt-in.

## Question

Is "Copilot" a usable LLM provider for Bifrost's `LlmProvider` layer via the GitHub Models API,
or is it a positioning term that should be routed to Claude/Gemini under the hood? What
constraints does it put on the LLM epic?

## Method

Implement a provider against the GitHub Models inference API and make a live grounded gap-fill
call; review the auth model, available models, rate limits, and Terms of Service.

## Findings

- **API access confirmed working.** `POST https://models.github.ai/inference/chat/completions`
  is **OpenAI-compatible** (chat-completions shape). Auth is `Authorization: Bearer <token>` where
  the token (`GITHUB_MODELS_TOKEN`, falling back to `GITHUB_TOKEN`) has **`models`** access. The
  default model `openai/gpt-4o-mini` returned a valid grounded gap-fill.
- So **Copilot/GitHub Models is a real provider**, not merely a positioning term. Because the
  endpoint is OpenAI-compatible, a thin adapter behind the `LlmProvider` trait is enough — no
  vendor SDK (consistent with the hard rule that orchestration calls only the trait).
- It exposes **multiple model families** (OpenAI, Llama, Mistral, etc.) through one endpoint, so a
  single provider can route across models by id.

## Constraints for the LLM epic

- **Experimentation surface.** GitHub Models is positioned for experimentation/prototyping, with
  its own **rate limits** and a **Terms of Service** distinct from a production inference contract.
  Production use needs an explicit **ToS / business sign-off** — this is a business decision, not a
  technical blocker, and is **out of scope for this spike**.
- **Opt-in + off by default.** The provider only activates when `GITHUB_MODELS_TOKEN` (or a
  `models`-scoped `GITHUB_TOKEN`) is set; it is never reached silently. The ToS caveat is recorded
  in the `copilot.rs` module docs.
- **Air-gap.** In air-gap mode the router forces everything local (Ollama/llama.cpp); GitHub
  Models — like any frontier/hosted provider — is disabled, and no pipeline data leaves the box.
- **Routing policy.** Treat it like any hosted provider in the `Router`: bulk/cheap gap-fills go to
  a local or Haiku-class model; reserve hosted models (incl. GitHub Models) for harder reasoning,
  and only with the ToS sign-off above.

## Decision

Keep the GitHub Models provider (`copilot.rs`) as a first-class, **opt-in** `LlmProvider`. No code
change is required from this spike — it documents the constraints the LLM epic already honours. The
remaining production ToS sign-off is tracked with the business owner, separate from engineering.
