use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

/// Unique identifier for a tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ToolId(pub Uuid);

impl ToolId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for ToolId {
    fn default() -> Self {
        Self::new()
    }
}

/// Errors that can occur during tool execution.
#[derive(Debug, Error)]
pub enum ToolError {
    #[error("Tool not found: {0}")]
    NotFound(String),

    #[error("Permission denied: {0}")]
    PermissionDenied(String),

    #[error("Execution failed: {0}")]
    ExecutionFailed(String),

    #[error("Timeout: {0}")]
    Timeout(String),

    #[error("Invalid arguments: {0}")]
    InvalidArguments(String),

    #[error("Sandbox error: {0}")]
    SandboxError(String),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),
}

/// Result of a tool execution.
pub type ToolResult<T> = Result<T, ToolError>;

/// Execution result containing output and metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionResult {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<i32>,
    pub duration_ms: u64,
    pub metadata: HashMap<String, serde_json::Value>,
}

impl ExecutionResult {
    pub fn success(stdout: impl Into<String>) -> Self {
        Self {
            success: true,
            stdout: stdout.into(),
            stderr: String::new(),
            exit_code: Some(0),
            duration_ms: 0,
            metadata: HashMap::new(),
        }
    }

    pub fn failure(stderr: impl Into<String>, exit_code: i32) -> Self {
        Self {
            success: false,
            stdout: String::new(),
            stderr: stderr.into(),
            exit_code: Some(exit_code),
            duration_ms: 0,
            metadata: HashMap::new(),
        }
    }

    pub fn with_duration(mut self, duration_ms: u64) -> Self {
        self.duration_ms = duration_ms;
        self
    }

    pub fn with_metadata(mut self, key: impl Into<String>, value: serde_json::Value) -> Self {
        self.metadata.insert(key.into(), value);
        self
    }
}

/// Sandbox configuration for tool execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxConfig {
    pub enabled: bool,
    pub working_directory: Option<PathBuf>,
    pub allowed_paths: Vec<PathBuf>,
    pub denied_paths: Vec<PathBuf>,
    pub allowed_commands: Vec<String>,
    pub denied_commands: Vec<String>,
    pub network_access: bool,
    pub environment_variables: HashMap<String, String>,
    pub resource_limits: ResourceLimits,
    pub timeout: Duration,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            working_directory: None,
            allowed_paths: vec![],
            denied_paths: vec![],
            allowed_commands: vec![],
            denied_commands: vec![],
            network_access: false,
            environment_variables: HashMap::new(),
            resource_limits: ResourceLimits::default(),
            timeout: Duration::from_secs(30),
        }
    }
}

/// Resource limits for sandboxed execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceLimits {
    pub max_memory_mb: Option<u64>,
    pub max_cpu_percent: Option<u32>,
    pub max_processes: Option<u32>,
    pub max_file_size_mb: Option<u64>,
    pub max_open_files: Option<u32>,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            max_memory_mb: Some(512),
            max_cpu_percent: Some(50),
            max_processes: Some(10),
            max_file_size_mb: Some(100),
            max_open_files: Some(100),
        }
    }
}

/// Shell command execution request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShellCommand {
    pub command: String,
    pub args: Vec<String>,
    pub working_dir: Option<PathBuf>,
    pub environment: HashMap<String, String>,
    pub timeout: Option<Duration>,
    pub shell: Option<String>,
}

impl ShellCommand {
    pub fn new(command: impl Into<String>) -> Self {
        Self {
            command: command.into(),
            args: Vec::new(),
            working_dir: None,
            environment: HashMap::new(),
            timeout: None,
            shell: None,
        }
    }

    pub fn with_args(mut self, args: Vec<String>) -> Self {
        self.args = args;
        self
    }

    pub fn with_working_dir(mut self, dir: PathBuf) -> Self {
        self.working_dir = Some(dir);
        self
    }

    pub fn with_env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.environment.insert(key.into(), value.into());
        self
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }
}

/// File operation types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileOpType {
    Read,
    Write,
    Append,
    Delete,
    Copy,
    Move,
    CreateDir,
    RemoveDir,
    List,
    Exists,
    Metadata,
}

/// File operation request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileOps {
    pub op: FileOpType,
    pub path: PathBuf,
    pub destination: Option<PathBuf>,
    pub content: Option<String>,
    pub encoding: Option<String>,
    pub recursive: bool,
    pub create_parents: bool,
}

impl FileOps {
    pub fn read(path: PathBuf) -> Self {
        Self {
            op: FileOpType::Read,
            path,
            destination: None,
            content: None,
            encoding: Some("utf-8".to_string()),
            recursive: false,
            create_parents: false,
        }
    }

    pub fn write(path: PathBuf, content: String) -> Self {
        Self {
            op: FileOpType::Write,
            path,
            destination: None,
            content: Some(content),
            encoding: Some("utf-8".to_string()),
            recursive: false,
            create_parents: true,
        }
    }

    pub fn append(path: PathBuf, content: String) -> Self {
        Self {
            op: FileOpType::Append,
            path,
            destination: None,
            content: Some(content),
            encoding: Some("utf-8".to_string()),
            recursive: false,
            create_parents: true,
        }
    }

    pub fn delete(path: PathBuf, recursive: bool) -> Self {
        Self {
            op: FileOpType::Delete,
            path,
            destination: None,
            content: None,
            encoding: None,
            recursive,
            create_parents: false,
        }
    }

    pub fn copy(source: PathBuf, destination: PathBuf) -> Self {
        Self {
            op: FileOpType::Copy,
            path: source,
            destination: Some(destination),
            content: None,
            encoding: None,
            recursive: false,
            create_parents: true,
        }
    }

    pub fn move_(source: PathBuf, destination: PathBuf) -> Self {
        Self {
            op: FileOpType::Move,
            path: source,
            destination: Some(destination),
            content: None,
            encoding: None,
            recursive: false,
            create_parents: true,
        }
    }

    pub fn list(path: PathBuf, recursive: bool) -> Self {
        Self {
            op: FileOpType::List,
            path,
            destination: None,
            content: None,
            encoding: None,
            recursive,
            create_parents: false,
        }
    }
}

/// System information request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemInfo {
    pub include_cpu: bool,
    pub include_memory: bool,
    pub include_disk: bool,
    pub include_network: bool,
    pub include_processes: bool,
    pub include_gpu: bool,
}

impl Default for SystemInfo {
    fn default() -> Self {
        Self {
            include_cpu: true,
            include_memory: true,
            include_disk: true,
            include_network: false,
            include_processes: false,
            include_gpu: false,
        }
    }
}

/// System information response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemInfoResult {
    pub hostname: String,
    pub os: String,
    pub os_version: String,
    pub arch: String,
    pub cpu: Option<CpuInfo>,
    pub memory: Option<MemoryInfo>,
    pub disk: Option<DiskInfo>,
    pub network: Option<NetworkInfo>,
    pub processes: Option<Vec<ProcessInfo>>,
    pub gpu: Option<GpuInfo>,
    pub uptime_seconds: u64,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

/// CPU information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CpuInfo {
    pub brand: String,
    pub cores: u32,
    pub threads: u32,
    pub frequency_mhz: u32,
    pub usage_percent: f32,
}

/// Memory information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryInfo {
    pub total_bytes: u64,
    pub available_bytes: u64,
    pub used_bytes: u64,
    pub usage_percent: f32,
}

/// Disk information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiskInfo {
    pub total_bytes: u64,
    pub available_bytes: u64,
    pub used_bytes: u64,
    pub usage_percent: f32,
    pub partitions: Vec<DiskPartition>,
}

/// Disk partition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiskPartition {
    pub mount_point: String,
    pub filesystem: String,
    pub total_bytes: u64,
    pub available_bytes: u64,
}

/// Network information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkInfo {
    pub interfaces: Vec<NetworkInterface>,
}

/// Network interface.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkInterface {
    pub name: String,
    pub ip_addresses: Vec<String>,
    pub mac_address: Option<String>,
    pub is_up: bool,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
}

/// Process information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessInfo {
    pub pid: u32,
    pub name: String,
    pub cpu_percent: f32,
    pub memory_bytes: u64,
    pub status: String,
}

/// GPU information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GpuInfo {
    pub devices: Vec<GpuDevice>,
}

/// GPU device.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GpuDevice {
    pub name: String,
    pub vendor: String,
    pub memory_total_mb: u64,
    pub memory_used_mb: u64,
    pub utilization_percent: f32,
}

/// Tool definition for the registry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDef {
    pub id: ToolId,
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
    pub returns: serde_json::Value,
    pub sandbox_config: Option<SandboxConfig>,
}

/// Trait for implementing tools.
#[async_trait]
pub trait Tool: Send + Sync {
    /// Unique identifier for this tool.
    fn id(&self) -> ToolId;

    /// Human-readable name.
    fn name(&self) -> &str;

    /// Description of what the tool does.
    fn description(&self) -> &str;

    /// JSON schema for parameters.
    fn parameters_schema(&self) -> serde_json::Value;

    /// JSON schema for return value.
    fn returns_schema(&self) -> serde_json::Value;

    /// Optional sandbox configuration.
    fn sandbox_config(&self) -> Option<&SandboxConfig> {
        None
    }

    /// Execute the tool with the given arguments.
    async fn execute(&self, args: serde_json::Value) -> ToolResult<ExecutionResult>;

    /// Get the tool definition.
    fn to_def(&self) -> ToolDef {
        ToolDef {
            id: self.id(),
            name: self.name().to_string(),
            description: self.description().to_string(),
            parameters: self.parameters_schema(),
            returns: self.returns_schema(),
            sandbox_config: self.sandbox_config().cloned(),
        }
    }
}

/// Tool registry for managing available tools.
pub struct ToolRegistry {
    tools: HashMap<String, Box<dyn Tool>>,
    by_id: HashMap<ToolId, String>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
            by_id: HashMap::new(),
        }
    }

    pub fn register(&mut self, tool: Box<dyn Tool>) {
        let name = tool.name().to_string();
        let id = tool.id();
        self.by_id.insert(id, name.clone());
        self.tools.insert(name, tool);
    }

    pub fn get(&self, name: &str) -> Option<&dyn Tool> {
        self.tools.get(name).map(|t| t.as_ref())
    }

    pub fn get_by_id(&self, id: ToolId) -> Option<&dyn Tool> {
        self.by_id.get(&id).and_then(|name| self.tools.get(name)).map(|t| t.as_ref())
    }

    pub fn list(&self) -> Vec<ToolDef> {
        self.tools.values().map(|t| t.to_def()).collect()
    }

    pub fn remove(&mut self, name: &str) -> Option<Box<dyn Tool>> {
        if let Some(tool) = self.tools.remove(name) {
            self.by_id.remove(&tool.id());
            Some(tool)
        } else {
            None
        }
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_execution_result_helpers() {
        let success = ExecutionResult::success("output");
        assert!(success.success);
        assert_eq!(success.stdout, "output");

        let failure = ExecutionResult::failure("error", 1);
        assert!(!failure.success);
        assert_eq!(failure.stderr, "error");
        assert_eq!(failure.exit_code, Some(1));
    }

    #[test]
    fn test_shell_command_builder() {
        let cmd = ShellCommand::new("ls")
            .with_args(vec!["-la".to_string()])
            .with_working_dir("/tmp".into())
            .with_env("KEY", "value")
            .with_timeout(Duration::from_secs(10));

        assert_eq!(cmd.command, "ls");
        assert_eq!(cmd.args, vec!["-la"]);
        assert_eq!(cmd.working_dir, Some("/tmp".into()));
        assert_eq!(cmd.environment.get("KEY"), Some(&"value".to_string()));
        assert_eq!(cmd.timeout, Some(Duration::from_secs(10)));
    }

    #[test]
    fn test_file_ops_constructors() {
        let read = FileOps::read("/tmp/file.txt".into());
        assert_eq!(read.op, FileOpType::Read);
        assert_eq!(read.path, PathBuf::from("/tmp/file.txt"));

        let write = FileOps::write("/tmp/file.txt".into(), "content".to_string());
        assert_eq!(write.op, FileOpType::Write);
        assert_eq!(write.content, Some("content".to_string()));

        let list = FileOps::list("/tmp".into(), true);
        assert_eq!(list.op, FileOpType::List);
        assert!(list.recursive);
    }
}