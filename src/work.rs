//! Local parallel work and cooperative cancellation primitives.

use std::error::Error;
use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use rayon::prelude::*;
use rayon::{ThreadPool, ThreadPoolBuilder};

#[derive(Debug, Clone, Default)]
pub struct JobContext {
    runner: JobRunner,
    cancel: CancellationToken,
}

impl JobContext {
    #[must_use]
    pub fn new(runner: JobRunner, cancel: CancellationToken) -> Self {
        Self { runner, cancel }
    }

    pub fn from_jobs(jobs: Option<usize>) -> Result<Self, JobRunnerBuildError> {
        Ok(Self::new(
            JobRunner::from_jobs(jobs)?,
            CancellationToken::new(),
        ))
    }

    #[must_use]
    pub fn runner(&self) -> &JobRunner {
        &self.runner
    }

    #[must_use]
    pub fn cancel(&self) -> &CancellationToken {
        &self.cancel
    }

    #[must_use]
    pub fn policy(&self) -> JobRunnerPolicy {
        self.runner.policy()
    }
}

#[derive(Debug, Clone, Default)]
pub struct JobRunner {
    execution: Execution,
}

impl JobRunner {
    #[must_use]
    pub fn automatic() -> Self {
        Self {
            execution: Execution::Global,
        }
    }

    #[must_use]
    pub fn inline() -> Self {
        Self {
            execution: Execution::Inline,
        }
    }

    /// Build a runner from an optional worker count.
    ///
    /// `None` uses the global Rayon pool, `Some(0)` runs on the caller thread,
    /// and any other value creates a private worker pool.
    pub fn from_jobs(jobs: Option<usize>) -> Result<Self, JobRunnerBuildError> {
        match jobs {
            None => Ok(Self::automatic()),
            Some(0) => Ok(Self::inline()),
            Some(workers) => Self::with_workers(workers),
        }
    }

    /// Build a runner backed by a private worker pool.
    ///
    /// `workers == 0` returns an inline runner.
    pub fn with_workers(workers: usize) -> Result<Self, JobRunnerBuildError> {
        if workers == 0 {
            return Ok(Self::inline());
        }

        ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .map(|pool| Self {
                execution: Execution::Pool(Arc::new(pool)),
            })
            .map_err(JobRunnerBuildError)
    }

    #[must_use]
    pub fn is_inline(&self) -> bool {
        matches!(self.execution, Execution::Inline)
    }

    #[must_use]
    pub fn policy(&self) -> JobRunnerPolicy {
        match &self.execution {
            Execution::Inline => JobRunnerPolicy::Inline,
            Execution::Global => JobRunnerPolicy::Automatic,
            Execution::Pool(pool) => JobRunnerPolicy::Workers(pool.current_num_threads()),
        }
    }

    pub fn install<R, F>(&self, f: F) -> R
    where
        R: Send,
        F: FnOnce() -> R + Send,
    {
        match &self.execution {
            Execution::Inline | Execution::Global => f(),
            Execution::Pool(pool) => pool.install(f),
        }
    }

    pub fn join<A, B, FA, FB>(&self, left: FA, right: FB) -> (A, B)
    where
        A: Send,
        B: Send,
        FA: FnOnce() -> A + Send,
        FB: FnOnce() -> B + Send,
    {
        match &self.execution {
            Execution::Inline => (left(), right()),
            Execution::Global => rayon::join(left, right),
            Execution::Pool(pool) => pool.install(|| rayon::join(left, right)),
        }
    }

    pub fn map<T, R, F>(&self, items: &[T], f: F) -> Vec<R>
    where
        T: Sync,
        R: Send,
        F: Fn(&T) -> R + Send + Sync,
    {
        match &self.execution {
            Execution::Inline => items.iter().map(f).collect(),
            Execution::Global => items.par_iter().map(f).collect(),
            Execution::Pool(pool) => pool.install(|| items.par_iter().map(f).collect()),
        }
    }

    pub fn try_map<T, R, E, F>(&self, items: &[T], f: F) -> Result<Vec<R>, E>
    where
        T: Sync,
        R: Send,
        E: Send,
        F: Fn(&T) -> Result<R, E> + Send + Sync,
    {
        match &self.execution {
            Execution::Inline => items.iter().map(f).collect(),
            Execution::Global => items.par_iter().map(f).collect(),
            Execution::Pool(pool) => pool.install(|| items.par_iter().map(f).collect()),
        }
    }

    pub fn map_until_cancelled<T, R, F>(
        &self,
        items: &[T],
        cancel: &CancellationToken,
        f: F,
    ) -> JobBatch<R>
    where
        T: Sync,
        R: Send,
        F: Fn(&T) -> R + Send + Sync,
    {
        let mapped = match &self.execution {
            Execution::Inline => {
                let mut out = Vec::with_capacity(items.len());
                for item in items {
                    if cancel.is_cancelled() {
                        out.push(None);
                    } else {
                        out.push(Some(f(item)));
                    }
                }
                out
            }
            Execution::Global => items
                .par_iter()
                .map(|item| {
                    if cancel.is_cancelled() {
                        None
                    } else {
                        Some(f(item))
                    }
                })
                .collect(),
            Execution::Pool(pool) => pool.install(|| {
                items
                    .par_iter()
                    .map(|item| {
                        if cancel.is_cancelled() {
                            None
                        } else {
                            Some(f(item))
                        }
                    })
                    .collect()
            }),
        };
        collect_job_batch(mapped, cancel.is_cancelled())
    }

    pub fn try_map_until_cancelled<T, R, E, F>(
        &self,
        items: &[T],
        cancel: &CancellationToken,
        f: F,
    ) -> Result<JobBatch<R>, E>
    where
        T: Sync,
        R: Send,
        E: Send,
        F: Fn(&T) -> Result<R, E> + Send + Sync,
    {
        let mapped = match &self.execution {
            Execution::Inline => {
                let mut out = Vec::with_capacity(items.len());
                for item in items {
                    if cancel.is_cancelled() {
                        out.push(None);
                        continue;
                    }

                    match f(item) {
                        Ok(value) => out.push(Some(value)),
                        Err(error) => {
                            cancel.cancel();
                            return Err(error);
                        }
                    }
                }
                out
            }
            Execution::Global => items
                .par_iter()
                .map(|item| try_map_item_until_cancelled(item, cancel, &f))
                .collect::<Result<Vec<_>, _>>()?,
            Execution::Pool(pool) => pool.install(|| {
                items
                    .par_iter()
                    .map(|item| try_map_item_until_cancelled(item, cancel, &f))
                    .collect::<Result<Vec<_>, _>>()
            })?,
        };
        Ok(collect_job_batch(mapped, cancel.is_cancelled()))
    }

    pub fn for_each_until_cancelled<T, F>(
        &self,
        items: &[T],
        cancel: &CancellationToken,
        f: F,
    ) -> JobBatch<()>
    where
        T: Sync,
        F: Fn(&T) + Send + Sync,
    {
        let completed = AtomicUsize::new(0);
        let skipped = AtomicUsize::new(0);
        let run = |item: &T| {
            if cancel.is_cancelled() {
                skipped.fetch_add(1, Ordering::Relaxed);
            } else {
                f(item);
                completed.fetch_add(1, Ordering::Relaxed);
            }
        };

        match &self.execution {
            Execution::Inline => items.iter().for_each(run),
            Execution::Global => items.par_iter().for_each(run),
            Execution::Pool(pool) => pool.install(|| items.par_iter().for_each(run)),
        }

        let completed = completed.load(Ordering::Relaxed);
        let skipped = skipped.load(Ordering::Relaxed);
        JobBatch::new(
            vec![(); completed],
            skipped,
            cancel.is_cancelled() || skipped > 0,
        )
    }
}

#[derive(Debug, Clone, Default)]
pub struct CancellationToken {
    flag: Arc<AtomicBool>,
}

impl CancellationToken {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.flag.store(true, Ordering::Relaxed);
    }

    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::Relaxed)
    }

    #[must_use]
    pub fn shared_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.flag)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobBatch<R> {
    completed: Vec<R>,
    skipped: usize,
    cancelled: bool,
}

impl<R> JobBatch<R> {
    fn new(completed: Vec<R>, skipped: usize, cancelled: bool) -> Self {
        Self {
            completed,
            skipped,
            cancelled,
        }
    }

    #[must_use]
    pub fn completed(&self) -> &[R] {
        &self.completed
    }

    #[must_use]
    pub const fn skipped(&self) -> usize {
        self.skipped
    }

    #[must_use]
    pub const fn was_cancelled(&self) -> bool {
        self.cancelled
    }

    #[must_use]
    pub fn into_completed(self) -> Vec<R> {
        self.completed
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobRunnerPolicy {
    Inline,
    Automatic,
    Workers(usize),
}

impl fmt::Display for JobRunnerPolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Inline => f.write_str("caller thread"),
            Self::Automatic => f.write_str("automatic worker pool"),
            Self::Workers(workers) => write!(f, "{workers} worker(s)"),
        }
    }
}

#[derive(Debug, Clone, Default)]
enum Execution {
    Inline,
    #[default]
    Global,
    Pool(Arc<ThreadPool>),
}

#[derive(Debug)]
pub struct JobRunnerBuildError(rayon::ThreadPoolBuildError);

impl fmt::Display for JobRunnerBuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl Error for JobRunnerBuildError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.0)
    }
}

fn collect_job_batch<R>(mapped: Vec<Option<R>>, cancelled: bool) -> JobBatch<R> {
    let skipped = mapped.iter().filter(|item| item.is_none()).count();
    let completed = mapped.into_iter().flatten().collect();
    JobBatch::new(completed, skipped, cancelled || skipped > 0)
}

fn try_map_item_until_cancelled<T, R, E, F>(
    item: &T,
    cancel: &CancellationToken,
    f: &F,
) -> Result<Option<R>, E>
where
    F: Fn(&T) -> Result<R, E>,
{
    if cancel.is_cancelled() {
        return Ok(None);
    }

    match f(item) {
        Ok(value) => Ok(Some(value)),
        Err(error) => {
            cancel.cancel();
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    #[test]
    fn inline_map_keeps_order() {
        let runner = JobRunner::inline();
        assert_eq!(runner.map(&[1, 2, 3], |value| value * 2), vec![2, 4, 6]);
    }

    #[test]
    fn cancellation_skips_inline_items_after_flag() {
        let runner = JobRunner::inline();
        let cancel = CancellationToken::new();
        let batch = runner.map_until_cancelled(&[1, 2, 3], &cancel, |value| {
            if *value == 1 {
                cancel.cancel();
            }
            *value
        });

        assert_eq!(batch.completed(), &[1]);
        assert_eq!(batch.skipped(), 2);
        assert!(batch.was_cancelled());
    }

    #[test]
    fn explicit_zero_jobs_is_inline() {
        let runner = JobRunner::from_jobs(Some(0)).expect("inline runner should build");
        assert!(runner.is_inline());
    }

    #[test]
    fn install_runs_work_on_runner() {
        let runner = JobRunner::with_workers(2).expect("worker pool should build");
        assert_eq!(runner.install(|| 42), 42);
    }

    #[test]
    fn join_runs_both_sides() {
        let runner = JobRunner::with_workers(2).expect("worker pool should build");
        assert_eq!(runner.join(|| 20, || 22), (20, 22));
    }

    #[test]
    fn inline_for_each_runs_side_effects() {
        let runner = JobRunner::inline();
        let cancel = CancellationToken::new();
        let seen = Mutex::new(Vec::new());

        let batch = runner.for_each_until_cancelled(&[1, 2, 3], &cancel, |value| {
            seen.lock()
                .expect("mutex should not be poisoned")
                .push(*value);
        });

        assert_eq!(
            *seen.lock().expect("mutex should not be poisoned"),
            vec![1, 2, 3]
        );
        assert_eq!(batch.completed().len(), 3);
        assert_eq!(batch.skipped(), 0);
        assert!(!batch.was_cancelled());
    }

    #[test]
    fn for_each_cancellation_skips_remaining_items() {
        let runner = JobRunner::inline();
        let cancel = CancellationToken::new();

        let batch = runner.for_each_until_cancelled(&[1, 2, 3], &cancel, |value| {
            if *value == 1 {
                cancel.cancel();
            }
        });

        assert_eq!(batch.completed().len(), 1);
        assert_eq!(batch.skipped(), 2);
        assert!(batch.was_cancelled());
    }
}
