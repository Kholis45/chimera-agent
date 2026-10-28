use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::{Mutex, Notify, Semaphore};
use uuid::Uuid;

/// Unique identifier for a task in the DAG.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TaskId(pub Uuid);

impl TaskId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for TaskId {
    fn default() -> Self {
        Self::new()
    }
}

/// Current execution state of a task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
}

/// Errors that can occur during task scheduling and execution.
#[derive(Debug, Error)]
pub enum SchedulerError {
    #[error("Task not found: {0:?}")]
    TaskNotFound(TaskId),

    #[error("Circular dependency detected involving task: {0:?}")]
    CircularDependency(TaskId),

    #[error("Task {0:?} is not ready to execute (dependencies incomplete)")]
    NotReady(TaskId),

    #[error("Task {0:?} already exists")]
    DuplicateTask(TaskId),

    #[error("Scheduler is shutting down")]
    ShuttingDown,

    #[error("Task execution failed: {0}")]
    ExecutionFailed(String),

    #[error("Concurrency limit exceeded")]
    ConcurrencyLimitExceeded,
}

/// A single unit of work in the DAG.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: TaskId,
    pub name: String,
    pub dependencies: Vec<TaskId>,
    pub status: TaskStatus,
    pub payload: serde_json::Value,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub started_at: Option<chrono::DateTime<chrono::Utc>>,
    pub completed_at: Option<chrono::DateTime<chrono::Utc>>,
    pub error: Option<String>,
}

impl Task {
    pub fn new(name: impl Into<String>, payload: serde_json::Value) -> Self {
        Self {
            id: TaskId::new(),
            name: name.into(),
            dependencies: Vec::new(),
            status: TaskStatus::Pending,
            payload,
            created_at: chrono::Utc::now(),
            started_at: None,
            completed_at: None,
            error: None,
        }
    }

    pub fn with_dependencies(mut self, deps: Vec<TaskId>) -> Self {
        self.dependencies = deps;
        self
    }

    pub fn is_ready(&self, completed: &HashSet<TaskId>) -> bool {
        self.status == TaskStatus::Pending
            && self.dependencies.iter().all(|d| completed.contains(d))
    }
}

/// Trait for executing a task. Implement this for custom task logic.
#[async_trait]
pub trait TaskExecutor: Send + Sync {
    async fn execute(&self, task: &mut Task) -> Result<(), SchedulerError>;
}

/// Configuration for the DAG scheduler.
#[derive(Debug, Clone)]
pub struct SchedulerConfig {
    pub max_concurrent_tasks: usize,
    pub task_timeout: Duration,
    pub enable_metrics: bool,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            max_concurrent_tasks: num_cpus::get(),
            task_timeout: Duration::from_secs(300),
            enable_metrics: true,
        }
    }
}

/// Metrics collected during scheduling.
#[derive(Debug, Default, Clone, Serialize)]
pub struct SchedulerMetrics {
    pub tasks_scheduled: u64,
    pub tasks_completed: u64,
    pub tasks_failed: u64,
    pub total_execution_time_ms: u64,
    pub queue_depth: usize,
}

/// Directed Acyclic Graph Task Scheduler.
/// Manages task dependencies, execution order, and concurrency.
pub struct DagScheduler {
    tasks: Arc<Mutex<HashMap<TaskId, Task>>>,
    adjacency: Arc<Mutex<HashMap<TaskId, Vec<TaskId>>>>,
    reverse_adjacency: Arc<Mutex<HashMap<TaskId, Vec<TaskId>>>>,
    ready_queue: Arc<Mutex<VecDeque<TaskId>>>,
    running_tasks: Arc<Mutex<HashSet<TaskId>>>,
    completed_tasks: Arc<Mutex<HashSet<TaskId>>>,
    failed_tasks: Arc<Mutex<HashSet<TaskId>>>,
    config: SchedulerConfig,
    metrics: Arc<Mutex<SchedulerMetrics>>,
    executor: Arc<dyn TaskExecutor>,
    semaphore: Arc<Semaphore>,
    shutdown_notify: Arc<Notify>,
    is_shutdown: Arc<Mutex<bool>>,
}

impl DagScheduler {
    /// Create a new scheduler with the given executor and configuration.
    pub fn new(executor: Arc<dyn TaskExecutor>, config: SchedulerConfig) -> Self {
        let semaphore = Arc::new(Semaphore::new(config.max_concurrent_tasks));
        Self {
            tasks: Arc::new(Mutex::new(HashMap::new())),
            adjacency: Arc::new(Mutex::new(HashMap::new())),
            reverse_adjacency: Arc::new(Mutex::new(HashMap::new())),
            ready_queue: Arc::new(Mutex::new(VecDeque::new())),
            running_tasks: Arc::new(Mutex::new(HashSet::new())),
            completed_tasks: Arc::new(Mutex::new(HashSet::new())),
            failed_tasks: Arc::new(Mutex::new(HashSet::new())),
            config,
            metrics: Arc::new(Mutex::new(SchedulerMetrics::default())),
            executor,
            semaphore,
            shutdown_notify: Arc::new(Notify::new()),
            is_shutdown: Arc::new(Mutex::new(false)),
        }
    }

    /// Add a task to the DAG. Returns the task ID.
    pub async fn add_task(&self, mut task: Task) -> Result<TaskId, SchedulerError> {
        let task_id = task.id;

        let mut tasks = self.tasks.lock().await;
        if tasks.contains_key(&task_id) {
            return Err(SchedulerError::DuplicateTask(task_id));
        }

        // Validate dependencies exist
        for dep in &task.dependencies {
            if !tasks.contains_key(dep) {
                return Err(SchedulerError::TaskNotFound(*dep));
            }
        }

        // Check for cycles
        if self.would_create_cycle(&task).await {
            return Err(SchedulerError::CircularDependency(task_id));
        }

        // Update adjacency lists
        let mut adjacency = self.adjacency.lock().await;
        let mut reverse_adjacency = self.reverse_adjacency.lock().await;

        for dep in &task.dependencies {
            adjacency.entry(*dep).or_default().push(task_id);
            reverse_adjacency.entry(task_id).or_default().push(*dep);
        }

        // Determine if task is ready to run
        let completed = self.completed_tasks.lock().await;
        let is_ready = task.is_ready(&completed);
        drop(completed);

        if is_ready {
            self.ready_queue.lock().await.push_back(task_id);
        }

        tasks.insert(task_id, task);

        if self.config.enable_metrics {
            self.metrics.lock().await.tasks_scheduled += 1;
        }

        Ok(task_id)
    }

    /// Check if adding this task would create a cycle.
    async fn would_create_cycle(&self, task: &Task) -> bool {
        let mut visited = HashSet::new();
        let mut stack = Vec::new();

        for dep in &task.dependencies {
            stack.push(*dep);
        }

        let adjacency = self.adjacency.lock().await;

        while let Some(current) = stack.pop() {
            if current == task.id {
                return true;
            }
            if visited.insert(current) {
                if let Some(children) = adjacency.get(&current) {
                    stack.extend(children);
                }
            }
        }

        false
    }

    /// Get the next ready task from the queue.
    pub async fn pop_ready(&self) -> Option<TaskId> {
        self.ready_queue.lock().await.pop_front()
    }

    /// Mark a task as running.
    pub async fn mark_running(&self, task_id: TaskId) -> Result<(), SchedulerError> {
        let mut tasks = self.tasks.lock().await;
        let task = tasks.get_mut(&task_id).ok_or(SchedulerError::TaskNotFound(task_id))?;

        if task.status != TaskStatus::Pending {
            return Err(SchedulerError::NotReady(task_id));
        }

        task.status = TaskStatus::Running;
        task.started_at = Some(chrono::Utc::now());

        self.running_tasks.lock().await.insert(task_id);
        Ok(())
    }

    /// Mark a task as completed successfully.
    pub async fn mark_completed(&self, task_id: TaskId) -> Result<(), SchedulerError> {
        let mut tasks = self.tasks.lock().await;
        let task = tasks.get_mut(&task_id).ok_or(SchedulerError::TaskNotFound(task_id))?;

        task.status = TaskStatus::Completed;
        task.completed_at = Some(chrono::Utc::now());

        self.running_tasks.lock().await.remove(&task_id);
        self.completed_tasks.lock().await.insert(task_id);

        // Unblock dependent tasks
        self.unblock_dependents(task_id).await;

        if self.config.enable_metrics {
            let mut metrics = self.metrics.lock().await;
            metrics.tasks_completed += 1;
            if let (Some(start), Some(end)) = (task.started_at, task.completed_at) {
                metrics.total_execution_time_ms += (end - start).num_milliseconds() as u64;
            }
        }

        Ok(())
    }

    /// Mark a task as failed.
    pub async fn mark_failed(&self, task_id: TaskId, error: String) -> Result<(), SchedulerError> {
        let mut tasks = self.tasks.lock().await;
        let task = tasks.get_mut(&task_id).ok_or(SchedulerError::TaskNotFound(task_id))?;

        task.status = TaskStatus::Failed;
        task.completed_at = Some(chrono::Utc::now());
        task.error = Some(error.clone());

        self.running_tasks.lock().await.remove(&task_id);
        self.failed_tasks.lock().await.insert(task_id);

        // Fail dependent tasks
        self.fail_dependents(task_id, error).await;

        if self.config.enable_metrics {
            self.metrics.lock().await.tasks_failed += 1;
        }

        Ok(())
    }

    /// Unblock tasks that depend on the completed task.
    async fn unblock_dependents(&self, completed_id: TaskId) {
        let adjacency = self.adjacency.lock().await;
        let dependents = adjacency.get(&completed_id).cloned().unwrap_or_default();
        drop(adjacency);

        let completed = self.completed_tasks.lock().await;
        let mut ready_queue = self.ready_queue.lock().await;

        for dep in dependents {
            let tasks = self.tasks.lock().await;
            if let Some(task) = tasks.get(&dep) {
                if task.is_ready(&completed) {
                    ready_queue.push_back(dep);
                }
            }
        }
    }

    /// Fail tasks that depend on the failed task.
    async fn fail_dependents(&self, failed_id: TaskId, error: String) {
        let adjacency = self.adjacency.lock().await;
        let dependents = adjacency.get(&failed_id).cloned().unwrap_or_default();
        drop(adjacency);

        for dep in dependents {
            let _ = self.mark_failed(dep, format!("Dependency failed: {}", error)).await;
        }
    }

    /// Execute the next available task.
    pub async fn execute_next(&self) -> Result<bool, SchedulerError> {
        let permit = self
            .semaphore
            .acquire()
            .await
            .map_err(|_| SchedulerError::ConcurrencyLimitExceeded)?;

        let task_id = match self.pop_ready().await {
            Some(id) => id,
            None => {
                drop(permit);
                return Ok(false);
            }
        };

        self.mark_running(task_id).await?;

        let executor = self.executor.clone();
        let scheduler = self.clone();

        tokio::spawn(async move {
            let mut task = {
                let tasks = scheduler.tasks.lock().await;
                tasks.get(&task_id).cloned()
            };

            if let Some(mut task) = task {
                let result = executor.execute(&mut task).await;

                match result {
                    Ok(_) => {
                        let _ = scheduler.mark_completed(task_id).await;
                    }
                    Err(e) => {
                        let _ = scheduler.mark_failed(task_id, e.to_string()).await;
                    }
                }
            }

            drop(permit);
            scheduler.shutdown_notify.notify_one();
        });

        Ok(true)
    }

    /// Run the scheduler until all tasks complete or shutdown is signaled.
    pub async fn run(&self) -> Result<(), SchedulerError> {
        loop {
            if *self.is_shutdown.lock().await {
                return Err(SchedulerError::ShuttingDown);
            }

            let has_work = self.execute_next().await?;

            if !has_work {
                let running = self.running_tasks.lock().await.len();
                let ready = self.ready_queue.lock().await.len();

                if running == 0 && ready == 0 {
                    break;
                }

                // Wait for a task to complete or new work to arrive
                tokio::select! {
                    _ = self.shutdown_notify.notified() => {}
                    _ = tokio::time::sleep(Duration::from_millis(100)) => {}
                }
            }
        }
        Ok(())
    }

    /// Signal the scheduler to shut down gracefully.
    pub async fn shutdown(&self) {
        *self.is_shutdown.lock().await = true;
        self.shutdown_notify.notify_waiters();
    }

    /// Get current scheduler metrics.
    pub async fn metrics(&self) -> SchedulerMetrics {
        let mut metrics = self.metrics.lock().await.clone();
        metrics.queue_depth = self.ready_queue.lock().await.len();
        metrics
    }

    /// Get all tasks.
    pub async fn tasks(&self) -> Vec<Task> {
        self.tasks.lock().await.values().cloned().collect()
    }

    /// Get a specific task by ID.
    pub async fn get_task(&self, task_id: TaskId) -> Option<Task> {
        self.tasks.lock().await.get(&task_id).cloned()
    }
}

impl Clone for DagScheduler {
    fn clone(&self) -> Self {
        Self {
            tasks: self.tasks.clone(),
            adjacency: self.adjacency.clone(),
            reverse_adjacency: self.reverse_adjacency.clone(),
            ready_queue: self.ready_queue.clone(),
            running_tasks: self.running_tasks.clone(),
            completed_tasks: self.completed_tasks.clone(),
            failed_tasks: self.failed_tasks.clone(),
            config: self.config.clone(),
            metrics: self.metrics.clone(),
            executor: self.executor.clone(),
            semaphore: self.semaphore.clone(),
            shutdown_notify: self.shutdown_notify.clone(),
            is_shutdown: self.is_shutdown.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    struct TestExecutor {
        counter: Arc<AtomicU32>,
        should_fail: bool,
    }

    #[async_trait]
    impl TaskExecutor for TestExecutor {
        async fn execute(&self, task: &mut Task) -> Result<(), SchedulerError> {
            self.counter.fetch_add(1, Ordering::SeqCst);
            if self.should_fail {
                return Err(SchedulerError::ExecutionFailed("test failure".into()));
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_simple_dag() {
        let executor = Arc::new(TestExecutor {
            counter: Arc::new(AtomicU32::new(0)),
            should_fail: false,
        });
        let scheduler = DagScheduler::new(executor.clone(), SchedulerConfig::default());

        let task_a = Task::new("A", serde_json::json!({}));
        let task_b = Task::new("B", serde_json::json!({})).with_dependencies(vec![task_a.id]);
        let task_c = Task::new("C", serde_json::json!({})).with_dependencies(vec![task_b.id]);

        scheduler.add_task(task_a).await.unwrap();
        scheduler.add_task(task_b).await.unwrap();
        scheduler.add_task(task_c).await.unwrap();

        scheduler.run().await.unwrap();

        assert_eq!(executor.counter.load(Ordering::SeqCst), 3);

        let tasks = scheduler.tasks().await;
        for task in tasks {
            assert_eq!(task.status, TaskStatus::Completed);
        }
    }

    #[tokio::test]
    async fn test_cycle_detection() {
        let executor = Arc::new(TestExecutor {
            counter: Arc::new(AtomicU32::new(0)),
            should_fail: false,
        });
        let scheduler = DagScheduler::new(executor, SchedulerConfig::default());

        let task_a = Task::new("A", serde_json::json!({}));
        let task_b = Task::new("B", serde_json::json!({})).with_dependencies(vec![task_a.id]);

        scheduler.add_task(task_a.clone()).await.unwrap();
        scheduler.add_task(task_b.clone()).await.unwrap();

        // Try to add A with dependency on B (creates cycle)
        let task_a_cycle = Task::new("A", serde_json::json!({})).with_dependencies(vec![task_b.id]);
        let result = scheduler.add_task(task_a_cycle).await;
        assert!(matches!(result, Err(SchedulerError::CircularDependency(_))));
    }

    #[tokio::test]
    async fn test_failure_propagation() {
        let executor = Arc::new(TestExecutor {
            counter: Arc::new(AtomicU32::new(0)),
            should_fail: true,
        });
        let scheduler = DagScheduler::new(executor.clone(), SchedulerConfig::default());

        let task_a = Task::new("A", serde_json::json!({}));
        let task_b = Task::new("B", serde_json::json!({})).with_dependencies(vec![task_a.id]);

        scheduler.add_task(task_a).await.unwrap();
        scheduler.add_task(task_b).await.unwrap();

        scheduler.run().await.unwrap();

        let tasks = scheduler.tasks().await;
        let failed_count = tasks.iter().filter(|t| t.status == TaskStatus::Failed).count();
        assert_eq!(failed_count, 2);
    }
}