use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use tokio::fs;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::Command;
use tokio::time::timeout;

use crate::types::{
    ExecutionResult, FileOps, FileOpType, SandboxConfig, ShellCommand, ToolError, ToolResult,
};

/// Executor for shell commands with sandboxing.
pub struct ShellExecutor {
    sandbox: SandboxConfig,
}

impl ShellExecutor {
    pub fn new(sandbox: SandboxConfig) -> Self {
        Self { sandbox }
    }

    /// Execute a shell command with sandbox restrictions.
    pub async fn execute(&self, cmd: ShellCommand) -> ToolResult<ExecutionResult> {
        let start = std::time::Instant::now();

        self.validate_command(&cmd)?;

        let working_dir = self.resolve_working_dir(cmd.working_dir.as_deref())?;
        let timeout_duration = cmd.timeout.unwrap_or(self.sandbox.timeout);

        let mut command = self.build_command(&cmd, &working_dir)?;

        let mut child = command
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to spawn: {}", e)))?;

        let stdout_handle = tokio::spawn(async move {
            let mut output = String::new();
            if let Some(stdout) = child.stdout.take() {
                let mut reader = BufReader::new(stdout);
                let _ = reader.read_to_string(&mut output).await;
            }
            output
        });

        let stderr_handle = tokio::spawn(async move {
            let mut output = String::new();
            if let Some(stderr) = child.stderr.take() {
                let mut reader = BufReader::new(stderr);
                let _ = reader.read_to_string(&mut output).await;
            }
            output
        });

        let wait_result = timeout(timeout_duration, child.wait()).await;

        let stdout = stdout_handle.await.unwrap_or_default();
        let stderr = stderr_handle.await.unwrap_or_default();

        let duration_ms = start.elapsed().as_millis() as u64;

        match wait_result {
            Ok(Ok(status)) => {
                let exit_code = status.code();
                let success = status.success();
                Ok(ExecutionResult {
                    success,
                    stdout,
                    stderr,
                    exit_code,
                    duration_ms,
                    metadata: HashMap::new(),
                })
            }
            Ok(Err(e)) => Ok(ExecutionResult::failure(
                format!("Process wait error: {}", e),
                -1,
            )
            .with_duration(duration_ms)),
            Err(_) => {
                let _ = child.kill().await;
                Ok(ExecutionResult::failure(
                    format!("Command timed out after {:?}", timeout_duration),
                    -1,
                )
                .with_duration(duration_ms))
            }
        }
    }

    /// Validate command against sandbox restrictions.
    fn validate_command(&self, cmd: &ShellCommand) -> ToolResult<()> {
        if !self.sandbox.enabled {
            return Ok(());
        }

        let base_cmd = Path::new(&cmd.command)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(&cmd.command);

        if !self.sandbox.allowed_commands.is_empty() {
            if !self.sandbox.allowed_commands.iter().any(|c| c == base_cmd) {
                return Err(ToolError::PermissionDenied(format!(
                    "Command '{}' not in allowed list",
                    base_cmd
                )));
            }
        }

        if self.sandbox.denied_commands.iter().any(|c| c == base_cmd) {
            return Err(ToolError::PermissionDenied(format!(
                "Command '{}' is denied",
                base_cmd
            )));
        }

        if let Some(ref working_dir) = cmd.working_dir {
            self.validate_path(working_dir)?;
        }

        Ok(())
    }

    /// Build the tokio Command with environment and shell.
    fn build_command(&self, cmd: &ShellCommand, working_dir: &Path) -> ToolResult<Command> {
        let mut command = if let Some(shell) = &cmd.shell {
            let mut c = Command::new(shell);
            c.arg("-c").arg(format!("{} {}", cmd.command, cmd.args.join(" ")));
            c
        } else {
            let mut c = Command::new(&cmd.command);
            c.args(&cmd.args);
            c
        };

        command.current_dir(working_dir);

        for (key, value) in &self.sandbox.environment_variables {
            command.env(key, value);
        }
        for (key, value) in &cmd.environment {
            command.env(key, value);
        }

        Ok(command)
    }

    /// Resolve working directory with sandbox constraints.
    fn resolve_working_dir(&self, requested: Option<&Path>) -> ToolResult<PathBuf> {
        let base = self.sandbox.working_directory.clone().unwrap_or_else(|| {
            std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
        });

        let target = match requested {
            Some(p) if p.is_absolute() => p.to_path_buf(),
            Some(p) => base.join(p),
            None => base,
        };

        self.validate_path(&target)?;
        Ok(target)
    }

    /// Validate a path against sandbox allow/deny lists.
    fn validate_path(&self, path: &Path) -> ToolResult<()> {
        if !self.sandbox.enabled {
            return Ok(());
        }

        let canonical = path
            .canonicalize()
            .unwrap_or_else(|_| path.to_path_buf());

        for denied in &self.sandbox.denied_paths {
            if canonical.starts_with(denied) {
                return Err(ToolError::PermissionDenied(format!(
                    "Path '{}' is in denied list",
                    path.display()
                )));
            }
        }

        if !self.sandbox.allowed_paths.is_empty() {
            let allowed = self
                .sandbox
                .allowed_paths
                .iter()
                .any(|p| canonical.starts_with(p));
            if !allowed {
                return Err(ToolError::PermissionDenied(format!(
                    "Path '{}' not in allowed list",
                    path.display()
                )));
            }
        }

        Ok(())
    }
}

/// Executor for file operations with sandboxing.
pub struct FileExecutor {
    sandbox: SandboxConfig,
}

impl FileExecutor {
    pub fn new(sandbox: SandboxConfig) -> Self {
        Self { sandbox }
    }

    /// Execute a file operation with sandbox restrictions.
    pub async fn execute(&self, ops: FileOps) -> ToolResult<ExecutionResult> {
        let start = std::time::Instant::now();

        self.validate_path(&ops.path)?;
        if let Some(ref dest) = ops.destination {
            self.validate_path(dest)?;
        }

        let result = match ops.op {
            FileOpType::Read => self.read_file(&ops).await,
            FileOpType::Write => self.write_file(&ops).await,
            FileOpType::Append => self.append_file(&ops).await,
            FileOpType::Delete => self.delete_path(&ops).await,
            FileOpType::Copy => self.copy_path(&ops).await,
            FileOpType::Move => self.move_path(&ops).await,
            FileOpType::CreateDir => self.create_dir(&ops).await,
            FileOpType::RemoveDir => self.remove_dir(&ops).await,
            FileOpType::List => self.list_dir(&ops).await,
            FileOpType::Exists => self.check_exists(&ops).await,
            FileOpType::Metadata => self.get_metadata(&ops).await,
        };

        result.map(|mut r| {
            r.duration_ms = start.elapsed().as_millis() as u64;
            r
        })
    }

    /// Validate path against sandbox restrictions.
    fn validate_path(&self, path: &Path) -> ToolResult<()> {
        if !self.sandbox.enabled {
            return Ok(());
        }

        let canonical = path
            .canonicalize()
            .unwrap_or_else(|_| path.to_path_buf());

        for denied in &self.sandbox.denied_paths {
            if canonical.starts_with(denied) {
                return Err(ToolError::PermissionDenied(format!(
                    "Path '{}' is in denied list",
                    path.display()
                )));
            }
        }

        if !self.sandbox.allowed_paths.is_empty() {
            let allowed = self
                .sandbox
                .allowed_paths
                .iter()
                .any(|p| canonical.starts_with(p));
            if !allowed {
                return Err(ToolError::PermissionDenied(format!(
                    "Path '{}' not in allowed list",
                    path.display()
                )));
            }
        }

        Ok(())
    }

    async fn read_file(&self, ops: &FileOps) -> ToolResult<ExecutionResult> {
        let encoding = ops.encoding.as_deref().unwrap_or("utf-8");
        let content = if encoding == "utf-8" {
            fs::read_to_string(&ops.path)
                .await
                .map_err(|e| ToolError::ExecutionFailed(format!("Read failed: {}", e)))?
        } else {
            let bytes = fs::read(&ops.path)
                .await
                .map_err(|e| ToolError::ExecutionFailed(format!("Read failed: {}", e)))?;
            String::from_utf8_lossy(&bytes).to_string()
        };

        Ok(ExecutionResult::success(content).with_metadata(
            "path",
            serde_json::json!(ops.path.to_string_lossy()),
        ))
    }

    async fn write_file(&self, ops: &FileOps) -> ToolResult<ExecutionResult> {
        if let Some(content) = &ops.content {
            if ops.create_parents {
                if let Some(parent) = ops.path.parent() {
                    fs::create_dir_all(parent)
                        .await
                        .map_err(|e| ToolError::ExecutionFailed(format!("Create dir failed: {}", e)))?;
                }
            }

            fs::write(&ops.path, content)
                .await
                .map_err(|e| ToolError::ExecutionFailed(format!("Write failed: {}", e)))?;

            Ok(ExecutionResult::success("File written").with_metadata(
                "path",
                serde_json::json!(ops.path.to_string_lossy()),
            ))
        } else {
            Err(ToolError::InvalidArguments("Write requires content".into()))
        }
    }

    async fn append_file(&self, ops: &FileOps) -> ToolResult<ExecutionResult> {
        if let Some(content) = &ops.content {
            if ops.create_parents {
                if let Some(parent) = ops.path.parent() {
                    fs::create_dir_all(parent)
                        .await
                        .map_err(|e| ToolError::ExecutionFailed(format!("Create dir failed: {}", e)))?;
                }
            }

            let mut file = fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&ops.path)
                .await
                .map_err(|e| ToolError::ExecutionFailed(format!("Open failed: {}", e)))?;

            use tokio::io::AsyncWriteExt;
            file.write_all(content.as_bytes())
                .await
                .map_err(|e| ToolError::ExecutionFailed(format!("Append failed: {}", e)))?;

            Ok(ExecutionResult::success("Content appended").with_metadata(
                "path",
                serde_json::json!(ops.path.to_string_lossy()),
            ))
        } else {
            Err(ToolError::InvalidArguments("Append requires content".into()))
        }
    }

    async fn delete_path(&self, ops: &FileOps) -> ToolResult<ExecutionResult> {
        if ops.recursive {
            fs::remove_dir_all(&ops.path)
                .await
                .map_err(|e| ToolError::ExecutionFailed(format!("Delete failed: {}", e)))?;
        } else {
            fs::remove_file(&ops.path)
                .await
                .map_err(|e| ToolError::ExecutionFailed(format!("Delete failed: {}", e)))?;
        }

        Ok(ExecutionResult::success("Deleted").with_metadata(
            "path",
            serde_json::json!(ops.path.to_string_lossy()),
        ))
    }

    async fn copy_path(&self, ops: &FileOps) -> ToolResult<ExecutionResult> {
        let dest = ops.destination.as_ref().ok_or_else(|| {
            ToolError::InvalidArguments("Copy requires destination".into())
        })?;

        if ops.path.is_dir() {
            self.copy_dir(&ops.path, dest).await?;
        } else {
            if ops.create_parents {
                if let Some(parent) = dest.parent() {
                    fs::create_dir_all(parent)
                        .await
                        .map_err(|e| ToolError::ExecutionFailed(format!("Create dir failed: {}", e)))?;
                }
            }
            fs::copy(&ops.path, dest)
                .await
                .map_err(|e| ToolError::ExecutionFailed(format!("Copy failed: {}", e)))?;
        }

        Ok(ExecutionResult::success("Copied").with_metadata(
            "source",
            serde_json::json!(ops.path.to_string_lossy()),
        ).with_metadata(
            "destination",
            serde_json::json!(dest.to_string_lossy()),
        ))
    }

    async fn copy_dir(&self, src: &Path, dst: &Path) -> ToolResult<()> {
        fs::create_dir_all(dst)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Create dir failed: {}", e)))?;

        let mut entries = fs::read_dir(src)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Read dir failed: {}", e)))?;

        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Dir entry failed: {}", e)))?
        {
            let src_path = entry.path();
            let dst_path = dst.join(entry.file_name());

            if src_path.is_dir() {
                self.copy_dir(&src_path, &dst_path).await?;
            } else {
                fs::copy(&src_path, &dst_path)
                    .await
                    .map_err(|e| ToolError::ExecutionFailed(format!("Copy failed: {}", e)))?;
            }
        }

        Ok(())
    }

    async fn move_path(&self, ops: &FileOps) -> ToolResult<ExecutionResult> {
        let dest = ops.destination.as_ref().ok_or_else(|| {
            ToolError::InvalidArguments("Move requires destination".into())
        })?;

        if ops.create_parents {
            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent)
                    .await
                    .map_err(|e| ToolError::ExecutionFailed(format!("Create dir failed: {}", e)))?;
            }
        }

        fs::rename(&ops.path, dest)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Move failed: {}", e)))?;

        Ok(ExecutionResult::success("Moved").with_metadata(
            "source",
            serde_json::json!(ops.path.to_string_lossy()),
        ).with_metadata(
            "destination",
            serde_json::json!(dest.to_string_lossy()),
        ))
    }

    async fn create_dir(&self, ops: &FileOps) -> ToolResult<ExecutionResult> {
        fs::create_dir_all(&ops.path)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Create dir failed: {}", e)))?;

        Ok(ExecutionResult::success("Directory created").with_metadata(
            "path",
            serde_json::json!(ops.path.to_string_lossy()),
        ))
    }

    async fn remove_dir(&self, ops: &FileOps) -> ToolResult<ExecutionResult> {
        if ops.recursive {
            fs::remove_dir_all(&ops.path)
                .await
                .map_err(|e| ToolError::ExecutionFailed(format!("Remove dir failed: {}", e)))?;
        } else {
            fs::remove_dir(&ops.path)
                .await
                .map_err(|e| ToolError::ExecutionFailed(format!("Remove dir failed: {}", e)))?;
        }

        Ok(ExecutionResult::success("Directory removed").with_metadata(
            "path",
            serde_json::json!(ops.path.to_string_lossy()),
        ))
    }

    async fn list_dir(&self, ops: &FileOps) -> ToolResult<ExecutionResult> {
        let mut entries = fs::read_dir(&ops.path)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("List failed: {}", e)))?;

        let mut items = Vec::new();
        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Dir entry failed: {}", e)))?
        {
            let mut item = serde_json::json!({
                "name": entry.file_name().to_string_lossy(),
                "path": entry.path().to_string_lossy(),
            });

            if let Ok(meta) = entry.metadata().await {
                item["is_dir"] = serde_json::json!(meta.is_dir());
                item["is_file"] = serde_json::json!(meta.is_file());
                item["size"] = serde_json::json!(meta.len());
                if let Ok(modified) = meta.modified() {
                    item["modified"] = serde_json::json!(modified
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_secs());
                }
            }

            items.push(item);

            if !ops.recursive && entry.path().is_dir() {
                continue;
            }
        }

        Ok(ExecutionResult::success(serde_json::to_string(&items).unwrap()).with_metadata(
            "path",
            serde_json::json!(ops.path.to_string_lossy()),
        ))
    }

    async fn check_exists(&self, ops: &FileOps) -> ToolResult<ExecutionResult> {
        let exists = ops.path.exists();
        Ok(ExecutionResult::success(serde_json::to_string(&exists).unwrap()).with_metadata(
            "path",
            serde_json::json!(ops.path.to_string_lossy()),
        ))
    }

    async fn get_metadata(&self, ops: &FileOps) -> ToolResult<ExecutionResult> {
        let meta = fs::metadata(&ops.path)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Metadata failed: {}", e)))?;

        let result = serde_json::json!({
            "is_dir": meta.is_dir(),
            "is_file": meta.is_file(),
            "size": meta.len(),
            "readonly": meta.permissions().readonly(),
            "modified": meta.modified()
                .map(|t| t.duration_since(std::time::UNIX_EPOCH).unwrap().as_secs())
                .ok(),
            "created": meta.created()
                .map(|t| t.duration_since(std::time::UNIX_EPOCH).unwrap().as_secs())
                .ok(),
        });

        Ok(ExecutionResult::success(serde_json::to_string(&result).unwrap()).with_metadata(
            "path",
            serde_json::json!(ops.path.to_string_lossy()),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{SandboxConfig, ShellCommand, FileOps, FileOpType};
    use std::path::PathBuf;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_shell_executor_basic() {
        let sandbox = SandboxConfig {
            enabled: false,
            ..Default::default()
        };
        let executor = ShellExecutor::new(sandbox);

        let cmd = ShellCommand::new("echo").with_args(vec!["hello".to_string()]);
        let result = executor.execute(cmd).await.unwrap();

        assert!(result.success);
        assert!(result.stdout.contains("hello"));
        assert_eq!(result.exit_code, Some(0));
    }

    #[tokio::test]
    async fn test_file_executor_write_read() {
        let dir = tempdir().unwrap();
        let sandbox = SandboxConfig {
            enabled: false,
            ..Default::default()
        };
        let executor = FileExecutor::new(sandbox);

        let file_path = dir.path().join("test.txt");

        let write_ops = FileOps::write(file_path.clone(), "hello world".to_string());
        let result = executor.execute(write_ops).await.unwrap();
        assert!(result.success);

        let read_ops = FileOps::read(file_path);
        let result = executor.execute(read_ops).await.unwrap();
        assert!(result.success);
        assert_eq!(result.stdout.trim(), "hello world");
    }

    #[tokio::test]
    async fn test_file_executor_list() {
        let dir = tempdir().unwrap();
        let sandbox = SandboxConfig {
            enabled: false,
            ..Default::default()
        };
        let executor = FileExecutor::new(sandbox);

        fs::write(dir.path().join("a.txt"), "a").await.unwrap();
        fs::write(dir.path().join("b.txt"), "b").await.unwrap();
        fs::create_dir(dir.path().join("sub")).await.unwrap();

        let list_ops = FileOps::list(dir.path().to_path_buf(), false);
        let result = executor.execute(list_ops).await.unwrap();
        assert!(result.success);

        let items: serde_json::Value = serde_json::from_str(&result.stdout).unwrap();
        assert!(items.as_array().unwrap().len() >= 3);
    }

    #[tokio::test]
    async fn test_sandbox_path_validation() {
        let dir = tempdir().unwrap();
        let sandbox = SandboxConfig {
            enabled: true,
            allowed_paths: vec![dir.path().to_path_buf()],
            denied_paths: vec![],
            ..Default::default()
        };
        let executor = FileExecutor::new(sandbox);

        let outside = PathBuf::from("/etc/passwd");
        let read_ops = FileOps::read(outside);
        let result = executor.execute(read_ops).await;
        assert!(matches!(result, Err(ToolError::PermissionDenied(_))));
    }
}