pub mod client;
pub mod stream;
pub mod types;

pub use client::{LLMClient, LLMClientBuilder, LLMError, LLMResult, CompletionStream};
pub use stream::{HttpClient, build_completion_body, merge_options};
pub use types::{
    ChunkChoice, Choice, CompletionChunk, CompletionResponse,
    ContentPart, Delta, EmbeddingData, EmbeddingInput,
    EmbeddingEncodingFormat, EmbeddingRequest, EmbeddingResponse,
    FinishReason, FunctionCall, FunctionCallDelta, FunctionDef,
    GenerationOptions, ImageBase64, ImageDetail, ImageUrl,
    LLMConfig, LogProbs, Message, MessageContent, Provider,
    PromptTokensDetails, ResponseFormat, Role, Tool, ToolCall,
    ToolCallDelta, ToolChoice, TokenLogProb, Usage,
    CompletionTokensDetails,
};