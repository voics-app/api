//! Bounded blocking work: a semaphore caps how many CPU/memory-heavy jobs run
//! at once, and callers that wait too long are turned away.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Semaphore;

use application::{AppError, AppResult};

#[derive(Debug, Clone)]
pub struct BlockingGate {
    permits: Arc<Semaphore>,
    queue_timeout: Duration,
}

impl BlockingGate {
    /// `concurrency` jobs run at once; others queue for up to `queue_timeout`.
    pub fn new(concurrency: usize, queue_timeout: Duration) -> Self {
        Self {
            permits: Arc::new(Semaphore::new(concurrency.max(1))),
            queue_timeout,
        }
    }

    /// Runs `job` on the blocking pool once a permit is free. The permit is
    /// held until the job finishes, even if the caller is dropped meanwhile.
    /// Saturation yields [`AppError::RateLimited`] (retryable, 429).
    pub async fn run<T, F>(&self, job: F) -> AppResult<T>
    where
        T: Send + 'static,
        F: FnOnce() -> AppResult<T> + Send + 'static,
    {
        let permit =
            match tokio::time::timeout(self.queue_timeout, self.permits.clone().acquire_owned())
                .await
            {
                Ok(Ok(permit)) => permit,
                Ok(Err(closed)) => return Err(AppError::internal(closed)),
                Err(_) => {
                    return Err(AppError::RateLimited {
                        retry_after_secs: self.queue_timeout.as_secs().max(1),
                    })
                }
            };
        tokio::task::spawn_blocking(move || {
            let outcome = job();
            drop(permit);
            outcome
        })
        .await
        .map_err(AppError::internal)?
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn never_runs_more_jobs_than_the_permit_count() {
        let gate = BlockingGate::new(2, Duration::from_secs(30));
        let running = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let mut tasks = Vec::new();
        for _ in 0..8 {
            let (gate, running, peak) = (gate.clone(), running.clone(), peak.clone());
            tasks.push(tokio::spawn(async move {
                gate.run(move || {
                    let now = running.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(now, Ordering::SeqCst);
                    std::thread::sleep(Duration::from_millis(40));
                    running.fetch_sub(1, Ordering::SeqCst);
                    Ok(())
                })
                .await
            }));
        }
        for task in tasks {
            task.await.unwrap().unwrap();
        }
        assert_eq!(peak.load(Ordering::SeqCst), 2);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn waiters_past_the_queue_timeout_get_a_retryable_error() {
        let gate = BlockingGate::new(1, Duration::from_millis(50));
        let busy = {
            let gate = gate.clone();
            tokio::spawn(async move {
                gate.run(|| {
                    std::thread::sleep(Duration::from_millis(300));
                    Ok(())
                })
                .await
            })
        };
        tokio::time::sleep(Duration::from_millis(30)).await;

        let turned_away = gate.run(|| Ok(())).await;
        assert!(matches!(
            turned_away,
            Err(AppError::RateLimited {
                retry_after_secs: 1
            })
        ));
        busy.await.unwrap().unwrap();
        assert!(gate.run(|| Ok(())).await.is_ok(), "permit is released");
    }
}
