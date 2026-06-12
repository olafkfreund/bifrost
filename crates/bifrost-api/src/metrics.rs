//! Control-plane metrics (#105): a small, dependency-free Prometheus registry.
//!
//! Tracks HTTP request counts + latencies (labelled by method, matched route, and
//! status) and a couple of domain counters (pipeline conversions, proposal
//! lifecycle transitions). Exposed in the Prometheus text exposition format at
//! `GET /metrics` (unauthenticated). Paths use the **matched route template**
//! (e.g. `/api/proposals/:id`) so per-id labels don't explode cardinality.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

use axum::extract::{MatchedPath, State};
use axum::http::header;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::{extract::Request, response::Response as AxumResponse};

use crate::Shared;

/// Histogram bucket upper bounds in milliseconds (cumulative, Prometheus-style).
const BUCKETS_MS: &[f64] = &[
    5.0, 10.0, 25.0, 50.0, 100.0, 250.0, 500.0, 1000.0, 2500.0, 5000.0, 10000.0,
];

#[derive(Default)]
struct Histogram {
    /// Cumulative counts, one per `BUCKETS_MS` bound.
    buckets: [u64; 11],
    sum_ms: f64,
    count: u64,
}

impl Histogram {
    fn observe(&mut self, ms: f64) {
        self.count += 1;
        self.sum_ms += ms;
        for (i, bound) in BUCKETS_MS.iter().enumerate() {
            if ms <= *bound {
                self.buckets[i] += 1;
            }
        }
    }
}

#[derive(Default)]
struct Inner {
    /// (method, route, status) -> request count.
    http_requests: HashMap<(String, String, u16), u64>,
    /// (method, route) -> latency histogram.
    http_latency: HashMap<(String, String), Histogram>,
    /// conversion outcome ("ok" | "error") -> count.
    conversions: HashMap<&'static str, u64>,
    /// target lifecycle state -> count.
    transitions: HashMap<String, u64>,
}

/// Thread-safe metrics registry. Held by value in `AppState` (already `Arc`-wrapped).
#[derive(Default)]
pub struct Metrics {
    inner: Mutex<Inner>,
}

impl Metrics {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record one HTTP request's outcome + latency.
    pub fn record_http(&self, method: &str, route: &str, status: u16, latency_ms: f64) {
        let mut g = self.inner.lock().unwrap();
        *g.http_requests
            .entry((method.to_string(), route.to_string(), status))
            .or_default() += 1;
        g.http_latency
            .entry((method.to_string(), route.to_string()))
            .or_default()
            .observe(latency_ms);
    }

    /// Count a pipeline conversion by outcome.
    pub fn inc_conversion(&self, ok: bool) {
        let mut g = self.inner.lock().unwrap();
        *g.conversions
            .entry(if ok { "ok" } else { "error" })
            .or_default() += 1;
    }

    /// Count a proposal lifecycle transition by target state.
    pub fn inc_transition(&self, to: &str) {
        let mut g = self.inner.lock().unwrap();
        *g.transitions.entry(to.to_string()).or_default() += 1;
    }

    /// Render the registry in Prometheus text exposition format.
    pub fn render(&self) -> String {
        let g = self.inner.lock().unwrap();
        let mut out = String::new();

        out.push_str("# HELP bifrost_http_requests_total Total HTTP requests.\n");
        out.push_str("# TYPE bifrost_http_requests_total counter\n");
        let mut reqs: Vec<_> = g.http_requests.iter().collect();
        reqs.sort_by(|a, b| a.0.cmp(b.0));
        for ((method, route, status), count) in reqs {
            out.push_str(&format!(
                "bifrost_http_requests_total{{method=\"{}\",path=\"{}\",status=\"{status}\"}} {count}\n",
                esc(method),
                esc(route),
            ));
        }

        out.push_str(
            "# HELP bifrost_http_request_duration_ms HTTP request latency (milliseconds).\n",
        );
        out.push_str("# TYPE bifrost_http_request_duration_ms histogram\n");
        let mut lat: Vec<_> = g.http_latency.iter().collect();
        lat.sort_by(|a, b| a.0.cmp(b.0));
        for ((method, route), hist) in lat {
            let labels = format!("method=\"{}\",path=\"{}\"", esc(method), esc(route));
            for (i, bound) in BUCKETS_MS.iter().enumerate() {
                out.push_str(&format!(
                    "bifrost_http_request_duration_ms_bucket{{{labels},le=\"{bound}\"}} {}\n",
                    hist.buckets[i],
                ));
            }
            out.push_str(&format!(
                "bifrost_http_request_duration_ms_bucket{{{labels},le=\"+Inf\"}} {}\n",
                hist.count,
            ));
            out.push_str(&format!(
                "bifrost_http_request_duration_ms_sum{{{labels}}} {}\n",
                hist.sum_ms,
            ));
            out.push_str(&format!(
                "bifrost_http_request_duration_ms_count{{{labels}}} {}\n",
                hist.count,
            ));
        }

        out.push_str("# HELP bifrost_conversions_total Pipeline conversions by outcome.\n");
        out.push_str("# TYPE bifrost_conversions_total counter\n");
        let mut conv: Vec<_> = g.conversions.iter().collect();
        conv.sort();
        for (outcome, count) in conv {
            out.push_str(&format!(
                "bifrost_conversions_total{{outcome=\"{outcome}\"}} {count}\n"
            ));
        }

        out.push_str(
            "# HELP bifrost_proposal_transitions_total Proposal lifecycle transitions by target state.\n",
        );
        out.push_str("# TYPE bifrost_proposal_transitions_total counter\n");
        let mut trans: Vec<_> = g.transitions.iter().collect();
        trans.sort();
        for (to, count) in trans {
            out.push_str(&format!(
                "bifrost_proposal_transitions_total{{to=\"{}\"}} {count}\n",
                esc(to),
            ));
        }

        out
    }
}

/// Escape a Prometheus label value (backslash, double-quote, newline).
fn esc(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

/// Middleware: time every request and record method / matched-route / status /
/// latency. Uses the matched route template so `:id` paths share one series.
pub async fn track_http(State(state): State<Shared>, req: Request, next: Next) -> AxumResponse {
    let method = req.method().as_str().to_string();
    let route = req
        .extensions()
        .get::<MatchedPath>()
        .map(|m| m.as_str().to_string())
        .unwrap_or_else(|| "unmatched".to_string());
    let start = Instant::now();
    let resp = next.run(req).await;
    let ms = start.elapsed().as_secs_f64() * 1000.0;
    state
        .metrics
        .record_http(&method, &route, resp.status().as_u16(), ms);
    resp
}

/// `GET /metrics` — Prometheus exposition (unauthenticated; no tenant data).
pub async fn metrics_handler(State(state): State<Shared>) -> Response {
    (
        [(header::CONTENT_TYPE, "text/plain; version=0.0.4")],
        state.metrics.render(),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_and_renders_http_counts_and_histogram() {
        let m = Metrics::new();
        m.record_http("GET", "/api/health", 200, 3.0);
        m.record_http("GET", "/api/health", 200, 40.0);
        m.record_http("POST", "/api/pipelines/:id/convert", 500, 120.0);

        let out = m.render();
        assert!(out.contains(
            "bifrost_http_requests_total{method=\"GET\",path=\"/api/health\",status=\"200\"} 2"
        ));
        assert!(out.contains(
            "bifrost_http_requests_total{method=\"POST\",path=\"/api/pipelines/:id/convert\",status=\"500\"} 1"
        ));
        // Cumulative histogram: both GET observations are <= 50ms.
        assert!(out.contains(
            "bifrost_http_request_duration_ms_bucket{method=\"GET\",path=\"/api/health\",le=\"50\"} 2"
        ));
        // Only the 3ms one is <= 5ms.
        assert!(out.contains(
            "bifrost_http_request_duration_ms_bucket{method=\"GET\",path=\"/api/health\",le=\"5\"} 1"
        ));
        assert!(out.contains(
            "bifrost_http_request_duration_ms_count{method=\"GET\",path=\"/api/health\"} 2"
        ));
    }

    #[test]
    fn records_domain_counters() {
        let m = Metrics::new();
        m.inc_conversion(true);
        m.inc_conversion(true);
        m.inc_conversion(false);
        m.inc_transition("approved");
        m.inc_transition("committed");

        let out = m.render();
        assert!(out.contains("bifrost_conversions_total{outcome=\"ok\"} 2"));
        assert!(out.contains("bifrost_conversions_total{outcome=\"error\"} 1"));
        assert!(out.contains("bifrost_proposal_transitions_total{to=\"approved\"} 1"));
        assert!(out.contains("bifrost_proposal_transitions_total{to=\"committed\"} 1"));
    }

    #[test]
    fn output_carries_help_and_type_lines() {
        let out = Metrics::new().render();
        assert!(out.contains("# TYPE bifrost_http_requests_total counter"));
        assert!(out.contains("# TYPE bifrost_http_request_duration_ms histogram"));
        assert!(out.contains("# TYPE bifrost_conversions_total counter"));
    }
}
