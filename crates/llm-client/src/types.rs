use std::collections::HashMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Configuration for an LLM provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LLMConfig {
    pub provider: Provider,
    pub model: String,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
    pub organization: Option<String>,
    pub timeout: Duration,
    pub max_retries: u32,
    pub default_options: GenerationOptions,
    pub extra_headers: HashMap<String, String>,
    pub extra_body: HashMap<String, serde_json::Value>,
}

impl Default for LLMConfig {
    fn default() -> Self {
        Self {
            provider: Provider::OpenAI,
            model: "gpt-4o-mini".to_string(),
            base_url: None,
            api_key: None,
            organization: None,
            timeout: Duration::from_secs(60),
            max_retries: 3,
            default_options: GenerationOptions::default(),
            extra_headers: HashMap::new(),
            extra_body: HashMap::new(),
        }
    }
}

/// Supported LLM providers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    OpenAI,
    Anthropic,
    Google,
    Mistral,
    Cohere,
    Ollama,
    Custom,
}

/// Role of a message in a conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

/// A single message in a conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: MessageContent,
    pub name: Option<String>,
    pub tool_calls: Option<Vec<ToolCall>>,
    pub tool_call_id: Option<String>,
}

/// Content of a message - can be text or multi-modal.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MessageContent {
    Text(String),
    Parts(Vec<ContentPart>),
}

/// A part of multi-modal message content.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentPart {
    Text { text: String },
    ImageUrl { image_url: ImageUrl },
    ImageBase64 { image_base64: ImageBase64 },
}

/// Image URL reference.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageUrl {
    pub url: String,
    pub detail: Option<ImageDetail>,
}

/// Base64 encoded image.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageBase64 {
    pub data: String,
    pub media_type: String,
    pub detail: Option<ImageDetail>,
}

/// Image detail level for vision models.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowcase")]
pub enum ImageDetail {
    Low,
    High,
    Auto,
}

/// Tool call made by the model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub function: FunctionCall,
    pub r#type: String,
}

/// Function call details.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionCall {
    pub name: String,
    pub arguments: String,
}

/// Tool definition for function calling.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tool {
    pub r#type: String,
    pub function: FunctionDef,
}

/// Function definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionDef {
    pub name: String,
    pub description: Option<String>,
    pub parameters: serde_json::Value,
}

/// Options for controlling generation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerationOptions {
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
    pub top_k: Option<u32>,
    pub max_tokens: Option<u32>,
    pub min_tokens: Option<u32>,
    pub presence_penalty: Option<f32>,
    pub frequency_penalty: Option<f32>,
    pub repetition_penalty: Option<f32>,
    pub stop: Option<Vec<String>>,
    pub seed: Option<u64>,
    pub logit_bias: Option<HashMap<String, f32>>,
    pub user: Option<String>,
    pub stream: bool,
    pub tools: Option<Vec<Tool>>,
    pub tool_choice: Option<ToolChoice>,
    pub response_format: Option<ResponseFormat>,
    pub parallel_tool_calls: Option<bool>,
}

/// Tool choice configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ToolChoice {
    None,
    Auto,
    Required,
    Function { name: String },
}

/// Response format configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ResponseFormat {
    Text,
    JsonObject,
    JsonSchema { schema: serde_json::Value },
}

impl Default for GenerationOptions {
    fn default() -> Self {
        Self {
            temperature: Some(0.7),
            top_p: Some(1.0),
            top_k: None,
            max_tokens: None,
            min_tokens: None,
            presence_penalty: None,
            frequency_penalty: None,
            repetition_penalty: None,
            stop: None,
            seed: None,
            logit_bias: None,
            user: None,
            stream: false,
            tools: None,
            tool_choice: None,
            response_format: None,
            parallel_tool_calls: Some(true),
        }
    }
}

/// Raw completion chunk for streaming.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionChunk {
    pub id: String,
    pub model: String,
    pub created: u64,
    pub choices: Vec<ChunkChoice>,
    pub usage: Option<Usage>,
}

/// A single choice in a completion chunk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkChoice {
    pub index: u32,
    pub delta: Delta,
    pub finish_reason: Option<FinishReason>,
    pub logprobs: Option<LogProbs>,
}

/// Delta content in a streaming chunk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Delta {
    pub role: Option<Role>,
    pub content: Option<String>,
    pub tool_calls: Option<Vec<ToolCallDelta>>,
}

/// Tool call delta in streaming.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallDelta {
    pub index: u32,
    pub id: Option<String>,
    pub function: Option<FunctionCallDelta>,
}

/// Function call delta.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionCallDelta {
    pub name: Option<String>,
    pub arguments: Option<String>,
}

/// Reason why generation finished.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FinishReason {
    Stop,
    Length,
    ToolCalls,
    ContentFilter,
    Error,
}

/// Token log probabilities.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogProbs {
    pub content: Option<Vec<TokenLogProb>>,
}

/// Single token log probability.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenLogProb {
    pub token: String,
    pub logprob: f32,
    pub bytes: Option<Vec<u8>>,
}

/// Token usage statistics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Usage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
    pub prompt_tokens_details: Option<PromptTokensDetails>,
    pub completion_tokens_details: Option<CompletionTokensDetails>,
}

/// Prompt token details.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptTokensDetails {
    pub cached_tokens: Option<u32>,
    pub audio_tokens: Option<u32>,
}

/// Completion token details.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionTokensDetails {
    pub reasoning_tokens: Option<u32>,
    pub audio_tokens: Option<u32>,
}

/// Complete completion response (non-streaming).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionResponse {
    pub id: String,
    pub model: String,
    pub created: u64,
    pub choices: Vec<Choice>,
    pub usage: Option<Usage>,
    pub system_fingerprint: Option<String>,
}

/// A single choice in a completion response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Choice {
    pub index: u32,
    pub message: Message,
    pub finish_reason: Option<FinishReason>,
    pub logprobs: Option<LogProbs>,
}

/// Embedding request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingRequest {
    pub model: String,
    pub input: EmbeddingInput,
    pub encoding_format: Option<EmbeddingEncodingFormat>,
    pub dimensions: Option<u32>,
    pub user: Option<String>,
}

/// Embedding input - string or array of strings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum EmbeddingInput {
    Single(String),
    Multiple(Vec<String>),
}

/// Embedding encoding format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EmbeddingEncodingFormat {
    Float,
    Base64,
}

/// Embedding response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingResponse {
    pub object: String,
    pub data: Vec<EmbeddingData>,
    pub model: String,
    pub usage: Usage,
}

/// Single embedding data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingData {
    pub object: String,
    pub index: u32,
    pub embedding: Vec<f32>,
}