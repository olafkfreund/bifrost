//! Token/cost accounting, budgets, and frontier rate limiting (#104).
//!
//! Three concerns, kept out of the [`LlmProvider`] trait so orchestration and the
//! [`Router`](crate::Router) stay unchanged:
//!
//! - **Accounting.** A [`CostLedger`] tallies token usage and an estimated USD
//!   cost per provider for a job. Usage is reported by [`estimate_tokens`] when a
//!   provider doesn't return exact counts (most don't, today).
//! - **Budgets.** A [`TokenBudget`] caps total tokens per job; once it would be
//!   exceeded the next call fails with [`LlmError::BudgetExceeded`] rather than
//!   silently spending.
//! - **Rate limiting.** A [`RateLimiter`] caps concurrent in-flight calls to
//!   **frontier** providers (local providers are unmetered and never throttled).
//!
//! These are composed by [`MeteredProvider`], a decorator that wraps any provider
//! and is itself an [`LlmProvider`] — so it drops into the existing call path.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::Semaphore;

use crate::{build_gap_fill_prompt, GapFillRequest, GapFillResponse, LlmError, LlmProvider};

/// Tokens consumed by one or more LLM calls.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenUsage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
}

impl TokenUsage {
    pub fn total(&self) -> u64 {
        self.prompt_tokens + self.completion_tokens
    }

    fn add(&mut self, other: TokenUsage) {
        self.prompt_tokens += other.prompt_tokens;
        self.completion_tokens += other.completion_tokens;
    }
}

/// Estimate the token count of `text`. Real providers may report exact counts;
/// absent that, ~4 characters per token is the standard rule of thumb. Kept
/// deterministic so budgets and tests are reproducible.
pub fn estimate_tokens(text: &str) -> u64 {
    let chars = text.chars().count() as u64;
    chars.div_ceil(4)
}

/// Estimate the completion tokens a [`GapFillResponse`] represents, from its
/// textual fields (what the model actually emitted).
fn estimate_response_tokens(resp: &GapFillResponse) -> u64 {
    let mut text = String::new();
    text.push_str(&resp.proposed_yaml);
    text.push_str(&resp.rationale);
    for f in &resp.risk_flags {
        text.push_str(f);
    }
    for s in &resp.verify_steps {
        text.push_str(s);
    }
    estimate_tokens(&text)
}

/// USD price per 1M tokens (input, output) per provider. Local providers are free.
#[derive(Debug, Clone)]
pub struct PriceTable {
    prices: HashMap<String, (f64, f64)>,
}

impl Default for PriceTable {
    /// Frontier list prices (USD per 1M tokens) as of the plan's pricing table;
    /// local/seat-licensed providers are zero. Override per deployment with
    /// [`PriceTable::with_price`].
    fn default() -> Self {
        let mut prices = HashMap::new();
        prices.insert("anthropic".to_string(), (5.0, 25.0));
        prices.insert("gemini".to_string(), (1.25, 5.0));
        // Azure OpenAI — depends on the deployed model; default to a GPT-4o-class
        // rate, overridable per deployment with PriceTable::with_price.
        prices.insert("azure-openai".to_string(), (2.5, 10.0));
        // Copilot is seat-licensed — no per-token cost to attribute here.
        prices.insert("copilot".to_string(), (0.0, 0.0));
        // Local / self-hosted providers are free.
        prices.insert("ollama".to_string(), (0.0, 0.0));
        prices.insert("openai-compatible".to_string(), (0.0, 0.0));
        prices.insert("mock".to_string(), (0.0, 0.0));
        Self { prices }
    }
}

impl PriceTable {
    /// Set (or override) a provider's input/output price per 1M tokens.
    pub fn with_price(
        mut self,
        provider: impl Into<String>,
        input_per_million: f64,
        output_per_million: f64,
    ) -> Self {
        self.prices
            .insert(provider.into(), (input_per_million, output_per_million));
        self
    }

    /// Estimated USD cost of `usage` on `provider` (unknown providers cost $0).
    pub fn cost_usd(&self, provider: &str, usage: TokenUsage) -> f64 {
        let (input, output) = self.prices.get(provider).copied().unwrap_or((0.0, 0.0));
        (usage.prompt_tokens as f64 / 1_000_000.0) * input
            + (usage.completion_tokens as f64 / 1_000_000.0) * output
    }
}

/// Per-provider token + cost line, surfaced for attestation and cost control.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderCost {
    pub provider: String,
    pub usage: TokenUsage,
    pub cost_usd: f64,
}

/// A job's complete token/cost accounting, surfaced on the conversion outcome for
/// attestation and cost control.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct JobCost {
    pub calls: u64,
    pub total_tokens: u64,
    pub total_cost_usd: f64,
    pub by_provider: Vec<ProviderCost>,
}

impl JobCost {
    /// Snapshot the current state of `ledger`.
    pub fn from_ledger(ledger: &CostLedger) -> Self {
        Self {
            calls: ledger.calls(),
            total_tokens: ledger.total_tokens(),
            total_cost_usd: ledger.total_cost_usd(),
            by_provider: ledger.breakdown(),
        }
    }
}

#[derive(Debug, Default)]
struct LedgerInner {
    by_provider: HashMap<String, TokenUsage>,
    calls: u64,
}

/// A thread-safe, cloneable token/cost tally for one job. Clones share the same
/// underlying counters, so a [`MeteredProvider`] and the caller see one ledger.
#[derive(Debug, Clone)]
pub struct CostLedger {
    inner: Arc<Mutex<LedgerInner>>,
    prices: PriceTable,
}

impl CostLedger {
    pub fn new(prices: PriceTable) -> Self {
        Self {
            inner: Arc::new(Mutex::new(LedgerInner::default())),
            prices,
        }
    }

    /// Record one call's usage against `provider`.
    pub fn record(&self, provider: &str, usage: TokenUsage) {
        let mut g = self.inner.lock().unwrap();
        g.by_provider
            .entry(provider.to_string())
            .or_default()
            .add(usage);
        g.calls += 1;
    }

    /// Total tokens recorded across all providers.
    pub fn total_tokens(&self) -> u64 {
        self.inner
            .lock()
            .unwrap()
            .by_provider
            .values()
            .map(TokenUsage::total)
            .sum()
    }

    /// Total estimated USD cost across all providers.
    pub fn total_cost_usd(&self) -> f64 {
        let g = self.inner.lock().unwrap();
        g.by_provider
            .iter()
            .map(|(p, u)| self.prices.cost_usd(p, *u))
            .sum()
    }

    /// Number of recorded calls.
    pub fn calls(&self) -> u64 {
        self.inner.lock().unwrap().calls
    }

    /// Per-provider breakdown, sorted by provider name for stable output.
    pub fn breakdown(&self) -> Vec<ProviderCost> {
        let g = self.inner.lock().unwrap();
        let mut rows: Vec<ProviderCost> = g
            .by_provider
            .iter()
            .map(|(p, u)| ProviderCost {
                provider: p.clone(),
                usage: *u,
                cost_usd: self.prices.cost_usd(p, *u),
            })
            .collect();
        rows.sort_by(|a, b| a.provider.cmp(&b.provider));
        rows
    }
}

impl Default for CostLedger {
    fn default() -> Self {
        Self::new(PriceTable::default())
    }
}

/// A per-job ceiling on total tokens. The default is unlimited.
#[derive(Debug, Clone, Copy, Default)]
pub struct TokenBudget {
    max_tokens: Option<u64>,
}

impl TokenBudget {
    /// No ceiling.
    pub fn unlimited() -> Self {
        Self { max_tokens: None }
    }

    /// Cap the job at `max` total tokens.
    pub fn tokens(max: u64) -> Self {
        Self {
            max_tokens: Some(max),
        }
    }

    /// Whether spending `next` more tokens on top of `current` would breach the cap.
    pub fn would_exceed(&self, current: u64, next: u64) -> bool {
        match self.max_tokens {
            Some(max) => current.saturating_add(next) > max,
            None => false,
        }
    }
}

/// Caps concurrent in-flight calls (a client-side concurrency limiter). Cloning
/// shares the same permit pool. Applied by [`MeteredProvider`] to frontier
/// providers only.
#[derive(Debug, Clone)]
pub struct RateLimiter {
    sem: Arc<Semaphore>,
}

impl RateLimiter {
    /// Allow at most `max` concurrent calls.
    pub fn concurrency(max: usize) -> Self {
        Self {
            sem: Arc::new(Semaphore::new(max)),
        }
    }

    /// Available permits right now (for tests/metrics).
    pub fn available(&self) -> usize {
        self.sem.available_permits()
    }

    /// Try to take a permit without waiting.
    pub fn try_acquire(&self) -> Option<tokio::sync::OwnedSemaphorePermit> {
        self.sem.clone().try_acquire_owned().ok()
    }

    /// Acquire a permit, waiting if the cap is currently reached.
    async fn acquire(&self) -> tokio::sync::OwnedSemaphorePermit {
        // The semaphore is never closed, so acquire never errors.
        self.sem.clone().acquire_owned().await.unwrap()
    }
}

/// Decorator that meters any [`LlmProvider`]: enforces a token budget, rate-limits
/// frontier providers, and records usage into a shared [`CostLedger`]. It is itself
/// an [`LlmProvider`], so it composes anywhere a provider is expected.
pub struct MeteredProvider<'a> {
    inner: &'a dyn LlmProvider,
    ledger: CostLedger,
    budget: TokenBudget,
    limiter: Option<RateLimiter>,
}

impl<'a> MeteredProvider<'a> {
    /// Meter `inner`, recording into `ledger`. Unlimited budget, no rate limit.
    pub fn new(inner: &'a dyn LlmProvider, ledger: CostLedger) -> Self {
        Self {
            inner,
            ledger,
            budget: TokenBudget::unlimited(),
            limiter: None,
        }
    }

    /// Enforce a per-job token budget.
    pub fn with_budget(mut self, budget: TokenBudget) -> Self {
        self.budget = budget;
        self
    }

    /// Apply a rate limiter to frontier (non-local) calls.
    pub fn with_rate_limit(mut self, limiter: RateLimiter) -> Self {
        self.limiter = Some(limiter);
        self
    }
}

#[async_trait]
impl LlmProvider for MeteredProvider<'_> {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn is_local(&self) -> bool {
        self.inner.is_local()
    }

    async fn fill_gap(&self, req: &GapFillRequest) -> Result<GapFillResponse, LlmError> {
        // Budget gate: refuse to start a call whose input alone would breach the cap.
        let prompt = build_gap_fill_prompt(req);
        let prompt_tokens = estimate_tokens(&prompt);
        let used = self.ledger.total_tokens();
        if self.budget.would_exceed(used, prompt_tokens) {
            return Err(LlmError::BudgetExceeded(format!(
                "{used} tokens used; this call needs ~{prompt_tokens} more"
            )));
        }

        // Rate-limit frontier providers only; locals run unthrottled. The permit is
        // held across the inner call so the cap bounds *in-flight* concurrency.
        let _permit = match (&self.limiter, self.inner.is_local()) {
            (Some(limiter), false) => Some(limiter.acquire().await),
            _ => None,
        };

        let resp = self.inner.fill_gap(req).await?;

        let usage = TokenUsage {
            prompt_tokens,
            completion_tokens: estimate_response_tokens(&resp),
        };
        self.ledger.record(self.inner.name(), usage);
        Ok(resp)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MockLlmProvider;
    use bifrost_core::{Gap, GapKind};

    fn req() -> GapFillRequest {
        GapFillRequest {
            gap: Gap {
                kind: GapKind::UnsupportedStep,
                construct: "DownloadSecureFile@1".into(),
                detail: "no equivalent".into(),
            },
            source_snippet: "- task: DownloadSecureFile@1".into(),
            converted_yaml: "steps: []".into(),
            importer_message: "no equivalent".into(),
            repo_context: "dotnet".into(),
        }
    }

    /// A frontier (non-local) wrapper around the mock, for rate-limit/cost tests.
    struct FrontierMock;
    #[async_trait]
    impl LlmProvider for FrontierMock {
        fn name(&self) -> &str {
            "anthropic"
        }
        fn is_local(&self) -> bool {
            false
        }
        async fn fill_gap(&self, req: &GapFillRequest) -> Result<GapFillResponse, LlmError> {
            MockLlmProvider.fill_gap(req).await
        }
    }

    #[test]
    fn estimate_tokens_is_chars_over_four_rounded_up() {
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("abcd"), 1);
        assert_eq!(estimate_tokens("abcde"), 2);
    }

    #[test]
    fn cost_is_zero_for_local_and_priced_for_frontier() {
        let prices = PriceTable::default();
        let usage = TokenUsage {
            prompt_tokens: 1_000_000,
            completion_tokens: 1_000_000,
        };
        assert_eq!(prices.cost_usd("ollama", usage), 0.0);
        // anthropic: 1M in @ $5 + 1M out @ $25 = $30.
        assert_eq!(prices.cost_usd("anthropic", usage), 30.0);
        // Unknown provider falls back to free.
        assert_eq!(prices.cost_usd("who-dis", usage), 0.0);
    }

    #[tokio::test]
    async fn ledger_accumulates_tokens_cost_and_breakdown() {
        let ledger = CostLedger::default();
        let metered = MeteredProvider::new(&FrontierMock, ledger.clone());
        metered.fill_gap(&req()).await.unwrap();
        metered.fill_gap(&req()).await.unwrap();

        assert_eq!(ledger.calls(), 2);
        assert!(ledger.total_tokens() > 0);
        assert!(ledger.total_cost_usd() > 0.0, "frontier calls cost money");

        let breakdown = ledger.breakdown();
        assert_eq!(breakdown.len(), 1);
        assert_eq!(breakdown[0].provider, "anthropic");
        assert_eq!(breakdown[0].usage.total(), ledger.total_tokens());
    }

    #[tokio::test]
    async fn local_provider_calls_are_free() {
        let ledger = CostLedger::default();
        let metered = MeteredProvider::new(&MockLlmProvider, ledger.clone());
        metered.fill_gap(&req()).await.unwrap();
        assert!(ledger.total_tokens() > 0, "tokens are still counted");
        assert_eq!(ledger.total_cost_usd(), 0.0, "but local tokens are free");
    }

    #[tokio::test]
    async fn budget_blocks_the_call_that_would_exceed_it() {
        let ledger = CostLedger::default();
        // A 1-token budget is breached by the first real prompt.
        let metered = MeteredProvider::new(&MockLlmProvider, ledger.clone())
            .with_budget(TokenBudget::tokens(1));
        let err = metered.fill_gap(&req()).await.unwrap_err();
        assert!(matches!(err, LlmError::BudgetExceeded(_)));
        // Nothing was spent because the call never ran.
        assert_eq!(ledger.calls(), 0);
    }

    #[tokio::test]
    async fn budget_allows_calls_until_the_cap_then_stops() {
        let ledger = CostLedger::default();
        // Generous budget: first call succeeds, second is refused once usage is high.
        let one_call_tokens = {
            let probe = CostLedger::default();
            MeteredProvider::new(&MockLlmProvider, probe.clone())
                .fill_gap(&req())
                .await
                .unwrap();
            probe.total_tokens()
        };
        let metered = MeteredProvider::new(&MockLlmProvider, ledger.clone())
            .with_budget(TokenBudget::tokens(one_call_tokens + 1));
        metered.fill_gap(&req()).await.expect("first call fits");
        let err = metered.fill_gap(&req()).await.unwrap_err();
        assert!(matches!(err, LlmError::BudgetExceeded(_)));
        assert_eq!(ledger.calls(), 1, "only the first call ran");
    }

    #[test]
    fn rate_limiter_caps_concurrent_permits() {
        let limiter = RateLimiter::concurrency(2);
        let p1 = limiter.try_acquire();
        let p2 = limiter.try_acquire();
        assert!(p1.is_some() && p2.is_some());
        // The cap is reached — a third try fails without blocking.
        assert!(limiter.try_acquire().is_none());
        assert_eq!(limiter.available(), 0);
        drop(p1);
        assert_eq!(limiter.available(), 1);
        assert!(limiter.try_acquire().is_some());
    }

    #[tokio::test]
    async fn local_providers_bypass_an_exhausted_rate_limiter() {
        // Concurrency of 1, fully taken: a frontier call would block here, but a
        // local provider must not be throttled — proving locals bypass the limiter.
        let limiter = RateLimiter::concurrency(1);
        let _held = limiter.try_acquire().expect("take the only permit");
        let metered = MeteredProvider::new(&MockLlmProvider, CostLedger::default())
            .with_rate_limit(limiter.clone());
        // Completes (doesn't deadlock) because MockLlmProvider.is_local() == true.
        metered
            .fill_gap(&req())
            .await
            .expect("local call not throttled");
    }
}
