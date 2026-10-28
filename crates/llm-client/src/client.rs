use std::pin::Pin;

use async_trait::async_trait;
use futures_core::Stream;
use reqwest::Error as ReqwestError;
use thiserror::Error;

use crate::types::{
    CompletionChunk, CompletionResponse, EmbeddingRequest, EmbeddingResponse,
    GenerationOptions, LLMConfig, Message,
};

/// Errors that can occur during LLM client operations.
#[derive(Debug, Error)]
pub enum LLMError {
    #[error("Request failed: {0}")]
    RequestFailed(#[from] ReqwestError),

    #[error("API error: {status} - {message}")]
    ApiError { status: u16, message: String },

    #[error("Invalid response: {0}")]
    InvalidResponse(String),

    #[error("Stream error: {0}")]
    StreamError(String),

    #[error("Configuration error: {0}")]
    ConfigError(String),

    #[error("Authentication error: {0}")]
    AuthError(String),

    #[error("Rate limited: retry after {retry_after:?}")]
    RateLimited { retry_after: Option<std::time::Duration> },

    #[error("Model not found: {0}")]
    ModelNotFound(String),

    #[error("Context length exceeded: {0}")]
    ContextLengthExceeded(String),

    #[error("Timeout: {0}")]
    Timeout(String),
}

/// Result type for LLM operations.
pub type LLMResult<T> = Result<T, LLMError>;

/// Streaming response type.
pub type CompletionStream = Pin<Box<dyn Stream<Item = LLMResult<CompletionChunk>> + Send>>;

/// Core trait for LLM providers. Implement this for each provider.
#[async_trait]
pub trait LLMClient: Send + Sync {
    /// Get the provider identifier.
    fn provider(&self) -> crate::types::Provider;

    /// Get the model name being used.
    fn model(&self) -> &str;

    /// Get the configuration.
    fn config(&self) -> &LLMConfig;

    /// Perform a standard (non-streaming) completion.
    async fn complete(
        &self,
        messages: Vec<Message>,
        options: Option<GenerationOptions>,
    ) -> LLMResult<CompletionResponse>;

    /// Perform a streaming completion.
    async fn complete_stream(
        &self,
        messages: Vec<Message>,
        options: Option<GenerationOptions>,
    ) -> LLMResult<CompletionStream>;

    /// Generate embeddings for the given input.
    async fn embed(&self, request: EmbeddingRequest) -> LLMResult<EmbeddingResponse>;

    /// List available models for this provider.
    async fn list_models(&self) -> LLMResult<Vec<String>>;

    /// Check if the client is healthy/ready.
    async fn health_check(&self) -> LLMResult<bool>;

    /// Get token count for messages (optional, provider-specific).
    async fn count_tokens(&self, messages: Vec<Message>) -> LLMResult<u32> {
        Err(LLMError::ConfigError("Token counting not supported".into()))
    }
}

/// Builder for creating LLM clients.
pub trait LLMClientBuilder: Send + Sync {
    type Client: LLMClient;

    /// Set the provider.
    fn provider(mut self, provider: crate::types::Provider) -> Self;

    /// Set the model.
    fn model(mut self, model: impl Into<String>) -> Self;

    /// Set the API key.
    fn api_key(mut self, api_key: impl Into<String>) -> Self;

    /// Set the base URL (for custom/self-hosted endpoints).
    fn base_url(mut self, base_url: impl Into<String>) -> Self;

    /// Set the organization (for providers that support it).
    fn organization(mut self, organization: impl Into<String>) -> Self;

    /// Set request timeout.
    fn timeout(mut self, timeout: std::time::Duration) -> Self;

    /// Set maximum retries.
    fn max_retries(mut self, max_retries: u32) -> Self;

    /// Set default generation options.
    fn default_options(mut self, options: GenerationOptions) -> Self;

    /// Add extra header.
    fn header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self;

    /// Add extra body parameter.
    fn extra_body(mut self, key: impl Into<String>, value: serde_json::Value) -> Self;

    /// Build the client.
    fn build(self) -> LLMResult<Self::Client>;
}