//! Conversion job orchestration (#47).
//!
//! A job fans out per-pipeline conversion across the portfolio with **bounded
//! concurrency** (a semaphore) and **resumability** — a pipeline whose proposal
//! is already in the store is skipped, so re-running a job picks up where a prior
//! one left off. Progress is broadcast as [`JobEvent`]s for the SSE stream (#44),
//! and a snapshot is kept for the status endpoint.

use std::collections::HashMap;
use std::sync::Arc;

use bifrost_core::AuditLog;
use serde::Serialize;
use serde_json::{json, Value};
use tokio::sync::{broadcast, Mutex, RwLock, Semaphore};
use tokio::task::JoinSet;

use crate::proposal_id_for;
use crate::store::{ProposalStore, StoredProposal};

/// Max pipelines converted at once (each may shell out to the Importer / call an LLM).
const CONCURRENCY: usize = 4;
/// Broadcast backlog before slow SSE subscribers start lagging.
const EVENT_CAP: usize = 256;

/// The outcome of converting one pipeline within a job.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobItem {
    pub pipeline_id: String,
    pub ok: bool,
    /// Already converted in a prior run — skipped (resumability).
    pub skipped: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// A progress event streamed to subscribers. `kind` is `started` | `item` | `done`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobEvent {
    pub job_id: String,
    pub kind: String,
    pub done: usize,
    pub total: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item: Option<JobItem>,
}

struct Snapshot {
    done: usize,
    items: Vec<JobItem>,
    finished: bool,
}

/// A running (or finished) conversion job.
pub struct JobState {
    pub id: String,
    pub total: usize,
    tx: broadcast::Sender<JobEvent>,
    snap: Mutex<Snapshot>,
}

impl JobState {
    /// Subscribe to live progress events.
    pub fn subscribe(&self) -> broadcast::Receiver<JobEvent> {
        self.tx.subscribe()
    }

    /// Current progress as JSON (for the status endpoint / SSE catch-up).
    pub async fn snapshot(&self) -> Value {
        let s = self.snap.lock().await;
        json!({
            "jobId": self.id,
            "total": self.total,
            "done": s.done,
            "finished": s.finished,
            "items": s.items,
        })
    }

    async fn emit_item(&self, item: JobItem) {
        let mut s = self.snap.lock().await;
        s.done += 1;
        s.items.push(item.clone());
        let ev = JobEvent {
            job_id: self.id.clone(),
            kind: "item".into(),
            done: s.done,
            total: self.total,
            item: Some(item),
        };
        let _ = self.tx.send(ev); // ignore: no subscribers is fine
    }

    async fn finish(&self) {
        let mut s = self.snap.lock().await;
        s.finished = true;
        let ev = JobEvent {
            job_id: self.id.clone(),
            kind: "done".into(),
            done: s.done,
            total: self.total,
            item: None,
        };
        let _ = self.tx.send(ev);
    }
}

/// In-memory registry of jobs (lifetime of the process; durable job records are
/// out of scope here — the proposals they produce are persisted by the store).
pub type JobRegistry = RwLock<HashMap<String, Arc<JobState>>>;

/// Spawn a conversion job over `(pipeline_id, project)` pairs, returning its
/// shared state. `project` is the pipeline's ADO project (for the live Docker
/// importer; `None` falls back to `BIFROST_PROJECT`/mock). The work runs on a
/// background task; callers register the returned state and stream its events.
pub fn spawn_convert_job(
    job_id: String,
    store: Arc<dyn ProposalStore>,
    pipelines: Vec<(String, Option<String>)>,
) -> Arc<JobState> {
    let (tx, _) = broadcast::channel(EVENT_CAP);
    let job = Arc::new(JobState {
        id: job_id,
        total: pipelines.len(),
        tx,
        snap: Mutex::new(Snapshot {
            done: 0,
            items: Vec::new(),
            finished: false,
        }),
    });

    let run = job.clone();
    tokio::spawn(async move {
        let _ = run.tx.send(JobEvent {
            job_id: run.id.clone(),
            kind: "started".into(),
            done: 0,
            total: run.total,
            item: None,
        });

        let sem = Arc::new(Semaphore::new(CONCURRENCY));
        let mut set = JoinSet::new();
        for (pid, project) in pipelines {
            // Acquire before spawning so at most CONCURRENCY tasks run at once.
            let permit = sem.clone().acquire_owned().await.expect("semaphore open");
            let store = store.clone();
            let job = run.clone();
            set.spawn(async move {
                let _permit = permit;
                let item = convert_one(store.as_ref(), &pid, project.as_deref()).await;
                job.emit_item(item).await;
            });
        }
        while set.join_next().await.is_some() {}
        run.finish().await;
    });

    job
}

/// Convert a single pipeline, skipping it if already in the store (resumability).
async fn convert_one(
    store: &dyn ProposalStore,
    pipeline_id: &str,
    project: Option<&str>,
) -> JobItem {
    let proposal_id = proposal_id_for(pipeline_id);
    if matches!(store.get(&proposal_id).await, Ok(Some(_))) {
        return JobItem {
            pipeline_id: pipeline_id.to_string(),
            ok: true,
            skipped: true,
            error: None,
        };
    }
    match crate::run_conversion(pipeline_id, project).await {
        Ok(outcome) => {
            let rec = StoredProposal {
                proposal: outcome.proposal,
                runbook: outcome.runbook,
                audit: AuditLog::new(),
            };
            match store.put(&rec).await {
                Ok(()) => JobItem {
                    pipeline_id: pipeline_id.to_string(),
                    ok: true,
                    skipped: false,
                    error: None,
                },
                Err(e) => JobItem {
                    pipeline_id: pipeline_id.to_string(),
                    ok: false,
                    skipped: false,
                    error: Some(e.to_string()),
                },
            }
        }
        Err(e) => JobItem {
            pipeline_id: pipeline_id.to_string(),
            ok: false,
            skipped: false,
            error: Some(e.to_string()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::InMemoryStore;

    /// Drive the runtime (cooperative, no timer feature) until the job finishes.
    async fn wait_finished(job: &Arc<JobState>) {
        for _ in 0..100_000 {
            if job.snapshot().await["finished"] == true {
                return;
            }
            tokio::task::yield_now().await;
        }
        panic!("job did not finish");
    }

    #[tokio::test]
    async fn job_converts_all_pipelines_and_reports_done() {
        let store: Arc<dyn ProposalStore> = Arc::new(InMemoryStore::default());
        let ids = vec![
            ("web-portal-ci".to_string(), None),
            ("payments-api-ci".to_string(), None),
        ];
        let job = spawn_convert_job("job-1".into(), store.clone(), ids);
        wait_finished(&job).await;

        let snap = job.snapshot().await;
        assert_eq!(snap["total"], 2);
        assert_eq!(snap["done"], 2);
        assert_eq!(snap["finished"], true);
        // Both proposals were persisted.
        assert!(store.get("prop-web-portal-ci").await.unwrap().is_some());
        assert!(store.get("prop-payments-api-ci").await.unwrap().is_some());
    }

    #[tokio::test]
    async fn job_skips_already_converted_pipelines() {
        let store: Arc<dyn ProposalStore> = Arc::new(InMemoryStore::default());
        let one = vec![("web-portal-ci".to_string(), None)];
        let job = spawn_convert_job("job-1".into(), store.clone(), one.clone());
        wait_finished(&job).await;

        // Second run over the same pipeline skips it (resumability).
        let job2 = spawn_convert_job("job-2".into(), store.clone(), one);
        wait_finished(&job2).await;
        assert_eq!(job2.snapshot().await["items"][0]["skipped"], true);
    }
}
