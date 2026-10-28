use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use chimera_core_engine::dag::{DagScheduler, SchedulerConfig, Task, TaskExecutor, TaskId, TaskStatus};
use chimera_llm_client::{HttpClient, LLMClient, LLMConfig, Message, GenerationOptions, CompletionChunk, CompletionResponse, Provider};
use chimera_tools::{ShellExecutor, FileExecutor, ShellCommand, FileOps, FileOpType, SandboxConfig, ExecutionResult, ToolError};
use serde::{Deserialize, Serialize};
use tauri::{command, State, Emitter, Window};
use tokio::sync::{Mutex, RwLock};
use uuid::Uuid;

/// Shared application state.
pub struct AppState {
    pub scheduler: Arc<Mutex<Option<DagScheduler>>>,
    pub llm_client: Arc<RwLock<Option<Arc<dyn LLMClient>>>>,
    pub shell_executor: Arc<ShellExecutor>,
    pub file_executor: Arc<FileExecutor>,
}

impl Default for AppState {
    fn default() -> Self {
        let sandbox = SandboxConfig::default();
        Self {
            scheduler: Arc::new(Mutex::new(None)),
            llm_client: Arc::new(RwLock::new(None)),
            shell_executor: Arc::new(ShellExecutor::new(sandbox.clone())),
            file_executor: Arc::new(FileExecutor::new(sandbox)),
        }
    }
}

/// Request/Response types for IPC.

#[derive(Debug, Deserialize)]
pub struct CreateTaskRequest {
    pub name: String,
    pub payload: serde_json::Value,
    pub dependencies: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct CreateTaskResponse {
    pub task_id: String,
}

#[derive(Debug, Deserialize)]
pub struct RunSchedulerRequest {
    pub max_concurrent: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct SchedulerStatusResponse {
    pub tasks_scheduled: u64,
    pub tasks_completed: u64,
    pub tasks_failed: u64,
    pub queue_depth: usize,
    pub running_count: usize,
}

#[derive(Debug, Serialize)]
pub struct TaskInfo {
    pub id: String,
    pub name: String,
    pub status: String,
    pub dependencies: Vec<String>,
    pub error: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct LLMCompleteRequest {
    pub messages: Vec<Message>,
    pub options: Option<GenerationOptions>,
    pub provider: Option<Provider>,
    pub model: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct LLMStreamRequest {
    pub messages: Vec<Message>,
    pub options: Option<GenerationOptions>,
    pub provider: Option<Provider>,
    pub model: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct LLMCompleteResponse {
    pub id: String,
    pub model: String,
    pub content: String,
    pub finish_reason: Option<String>,
    pub usage: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
pub struct LLMStreamEvent {
    pub chunk: CompletionChunk,
    pub is_final: bool,
}

#[derive(Debug, Deserialize)]
pub struct ShellExecuteRequest {
    pub command: String,
    pub args: Vec<String>,
    pub working_dir: Option<String>,
    pub environment: HashMap<String, String>,
    pub timeout_secs: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct ShellExecuteResponse {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<i32>,
    pub duration_ms: u64,
}

#[derive(Debug, Deserialize)]
pub struct FileOperationRequest {
    pub op: FileOpType,
    pub path: String,
    pub destination: Option<String>,
    pub content: Option<String>,
    pub encoding: Option<String>,
    pub recursive: bool,
    pub create_parents: bool,
}

#[derive(Debug, Serialize)]
pub struct FileOperationResponse {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u64,
    pub metadata: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct ConfigureLLMRequest {
    pub provider: Provider,
    pub model: String,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
    pub organization: Option<String>,
    pub timeout_secs: Option<u64>,
    pub default_options: Option<GenerationOptions>,
}

#[derive(Debug, Serialize)]
pub struct ConfigureLLMResponse {
    pub success: bool,
    pub provider: Provider,
    pub model: String,
}

#[derive(Debug, Deserialize)]
pub struct ConfigureSandboxRequest {
    pub enabled: bool,
    pub working_directory: Option<String>,
    pub allowed_paths: Vec<String>,
    pub denied_paths: Vec<String>,
    pub allowed_commands: Vec<String>,
    pub denied_commands: Vec<String>,
    pub network_access: bool,
    pub timeout_secs: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct ConfigureSandboxResponse {
    pub success: bool,
}

#[derive(Debug, Serialize)]
pub struct HealthCheckResponse {
    pub llm_healthy: bool,
    pub scheduler_running: bool,
}

/// Initialize the DAG scheduler with a task executor.
#[command]
pub async fn init_scheduler(
    state: State<'_, AppState>,
    max_concurrent: Option<usize>,
) -> Result<(), String> {
    let mut scheduler_guard = state.scheduler.lock().await;
    
    if scheduler_guard.is_some() {
        return Err("Scheduler already initialized".into());
    }

    let config = SchedulerConfig {
        max_concurrent_tasks: max_concurrent.unwrap_or(num_cpus::get()),
        ..Default::default()
    };

    struct NoopExecutor;
    #[async_trait::async_trait]
    impl TaskExecutor for NoopExecutor {
        async fn execute(&self, _task: &mut Task) -> Result<(), chimera_core_engine::dag::SchedulerError> {
            Ok(())
        }
    }

    let scheduler = DagScheduler::new(Arc::new(NoopExecutor), config);
    *scheduler_guard = Some(scheduler);
    
    Ok(())
}

/// Add a task to the DAG scheduler.
#[command]
pub async fn add_task(
    state: State<'_, AppState>,
    request: CreateTaskRequest,
) -> Result<CreateTaskResponse, String> {
    let scheduler_guard = state.scheduler.lock().await;
    let scheduler = scheduler_guard.as_ref().ok_or("Scheduler not initialized")?;

    let mut task = Task::new(request.name, request.payload);
    
    let dep_ids: Result<Vec<TaskId>, _> = request.dependencies
        .into_iter()
        .map(|s| Uuid::parse_str(&s).map(TaskId).map_err(|e| e.to_string()))
        .collect();
    
    task.dependencies = dep_ids?;

    let task_id = scheduler.add_task(task).await.map_err(|e| e.to_string())?;

    Ok(CreateTaskResponse {
        task_id: task_id.0.to_string(),
    })
}

/// Run the scheduler until completion.
#[command]
pub async fn run_scheduler(
    state: State<'_, AppState>,
) -> Result<(), String> {
    let scheduler_guard = state.scheduler.lock().await;
    let scheduler = scheduler_guard.as_ref().ok_or("Scheduler not initialized")?;

    scheduler.run().await.map_err(|e| e.to_string())
}

/// Get scheduler status and metrics.
#[command]
pub async fn get_scheduler_status(
    state: State<'_, AppState>,
) -> Result<SchedulerStatusResponse, String> {
    let scheduler_guard = state.scheduler.lock().await;
    let scheduler = scheduler_guard.as_ref().ok_or("Scheduler not initialized")?;

    let metrics = scheduler.metrics().await;
    let running = scheduler.running_tasks.lock().await.len();

    Ok(SchedulerStatusResponse {
        tasks_scheduled: metrics.tasks_scheduled,
        tasks_completed: metrics.tasks_completed,
        tasks_failed: metrics.tasks_failed,
        queue_depth: metrics.queue_depth,
        running_count: running,
    })
}

/// Get all tasks with their status.
#[command]
pub async fn list_tasks(
    state: State<'_, AppState>,
) -> Result<Vec<TaskInfo>, String> {
    let scheduler_guard = state.scheduler.lock().await;
    let scheduler = scheduler_guard.as_ref().ok_or("Scheduler not initialized")?;

    let tasks = scheduler.tasks().await;
    Ok(tasks.into_iter().map(|t| TaskInfo {
        id: t.id.0.to_string(),
        name: t.name,
        status: format!("{:?}", t.status),
        dependencies: t.dependencies.iter().map(|d| d.0.to_string()).collect(),
        error: t.error,
    }).collect())
}

/// Configure and initialize the LLM client.
#[command]
pub async fn configure_llm(
    state: State<'_, AppState>,
    request: ConfigureLLMRequest,
) -> Result<ConfigureLLMResponse, String> {
    let mut config = LLMConfig {
        provider: request.provider,
        model: request.model.clone(),
        base_url: request.base_url,
        api_key: request.api_key,
        organization: request.organization,
        timeout: std::time::Duration::from_secs(request.timeout_secs.unwrap_or(60)),
        default_options: request.default_options.unwrap_or_default(),
        ..Default::default()
    };

    let http_client = HttpClient::new(config).map_err(|e| e.to_string())?;
    let client: Arc<dyn LLMClient> = Arc::new(http_client);

    *state.llm_client.write().await = Some(client);

    Ok(ConfigureLLMResponse {
        success: true,
        provider: request.provider,
        model: request.model,
    })
}

/// Perform a non-streaming LLM completion.
#[command]
pub async fn llm_complete(
    state: State<'_, AppState>,
    request: LLMCompleteRequest,
) -> Result<LLMCompleteResponse, String> {
    let client_guard = state.llm_client.read().await;
    let client = client_guard.as_ref().ok_or("LLM client not configured")?;

    let options = request.options.unwrap_or_default();
    let response = client.complete(request.messages, Some(options))
        .await
        .map_err(|e| e.to_string())?;

    let choice = response.choices.first().ok_or("No choices in response")?;
    let content = match &choice.message.content {
        chimera_llm_client::types::MessageContent::Text(t) => t.clone(),
        chimera_llm_client::types::MessageContent::Parts(parts) => {
            parts.iter().filter_map(|p| match p {
                chimera_llm_client::types::ContentPart::Text { text } => Some(text.clone()),
                _ => None,
            }).collect::<Vec<_>>().join("")
        }
    };

    Ok(LLMCompleteResponse {
        id: response.id,
        model: response.model,
        content,
        finish_reason: choice.finish_reason.map(|r| format!("{:?}", r)),
        usage: response.usage.map(|u| serde_json::to_value(u).unwrap()),
    })
}

/// Perform a streaming LLM completion (emits events via Tauri event system).
#[command]
pub async fn llm_stream(
    window: Window,
    state: State<'_, AppState>,
    request: LLMStreamRequest,
) -> Result<(), String> {
    let client_guard = state.llm_client.read().await;
    let client = client_guard.as_ref().ok_or("LLM client not configured")?;

    let options = request.options.unwrap_or_default();
    let mut stream = client.complete_stream(request.messages, Some(options))
        .await
        .map_err(|e| e.to_string())?;

    use futures_util::StreamExt;
    while let Some(chunk_result) = stream.next().await {
        match chunk_result {
            Ok(chunk) => {
                let is_final = chunk.choices.iter().any(|c| c.finish_reason.is_some());
                window.emit("llm-stream-chunk", LLMStreamEvent {
                    chunk,
                    is_final,
                }).map_err(|e| e.to_string())?;
                
                if is_final {
                    break;
                }
            }
            Err(e) => {
                window.emit("llm-stream-error", e.to_string()).map_err(|e| e.to_string())?;
                break;
            }
        }
    }

    Ok(())
}

/// Execute a shell command.
#[command]
pub async fn shell_execute(
    state: State<'_, AppState>,
    request: ShellExecuteRequest,
) -> Result<ShellExecuteResponse, String> {
    let mut cmd = ShellCommand::new(request.command)
        .with_args(request.args);

    if let Some(dir) = request.working_dir {
        cmd = cmd.with_working_dir(PathBuf::from(dir));
    }

    for (k, v) in request.environment {
        cmd = cmd.with_env(k, v);
    }

    if let Some(timeout) = request.timeout_secs {
        cmd = cmd.with_timeout(std::time::Duration::from_secs(timeout));
    }

    let result = state.shell_executor.execute(cmd)
        .await
        .map_err(|e| e.to_string())?;

    Ok(ShellExecuteResponse {
        success: result.success,
        stdout: result.stdout,
        stderr: result.stderr,
        exit_code: result.exit_code,
        duration_ms: result.duration_ms,
    })
}

/// Execute a file operation.
#[command]
pub async fn file_operation(
    state: State<'_, AppState>,
    request: FileOperationRequest,
) -> Result<FileOperationResponse, String> {
    let ops = FileOps {
        op: request.op,
        path: PathBuf::from(request.path),
        destination: request.destination.map(PathBuf::from),
        content: request.content,
        encoding: request.encoding,
        recursive: request.recursive,
        create_parents: request.create_parents,
    };

    let result = state.file_executor.execute(ops)
        .await
        .map_err(|e| e.to_string())?;

    Ok(FileOperationResponse {
        success: result.success,
        stdout: result.stdout,
        stderr: result.stderr,
        duration_ms: result.duration_ms,
        metadata: result.metadata,
    })
}

/// Configure sandbox settings for tool executors.
#[command]
pub async fn configure_sandbox(
    state: State<'_, AppState>,
    request: ConfigureSandboxRequest,
) -> Result<ConfigureSandboxResponse, String> {
    let sandbox = SandboxConfig {
        enabled: request.enabled,
        working_directory: request.working_directory.map(PathBuf::from),
        allowed_paths: request.allowed_paths.into_iter().map(PathBuf::from).collect(),
        denied_paths: request.denied_paths.into_iter().map(PathBuf::from).collect(),
        allowed_commands: request.allowed_commands,
        denied_commands: request.denied_commands,
        network_access: request.network_access,
        timeout: std::time::Duration::from_secs(request.timeout_secs.unwrap_or(30)),
        ..Default::default()
    };

    state.shell_executor = Arc::new(ShellExecutor::new(sandbox.clone()));
    state.file_executor = Arc::new(FileExecutor::new(sandbox));

    Ok(ConfigureSandboxResponse { success: true })
}

/// Health check for all subsystems.
#[command]
pub async fn health_check(
    state: State<'_, AppState>,
) -> Result<HealthCheckResponse, String> {
    let llm_healthy = state.llm_client.read().await.as_ref()
        .map(|c| c.health_check().await.unwrap_or(false))
        .unwrap_or(false);

    let scheduler_running = state.scheduler.lock().await.is_some();

    Ok(HealthCheckResponse {
        llm_healthy,
        scheduler_running,
    })
}