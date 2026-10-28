use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use futures_core::Stream;
use reqwest::{Client, RequestBuilder, Response, StatusCode};
use serde::de::DeserializeOwned;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_util::codec::{BytesCodec, FramedRead};

use crate::client::{LLMError, LLMResult};
use crate::types::{CompletionChunk, GenerationOptions, LLMConfig, Message};

const SSE_PREFIX: &str = "data: ";
const SSE_DONE: &str = "[DONE]";

/// HTTP client wrapper with retry logic.
#[derive(Clone)]
pub struct HttpClient {
    client: Client,
    config: LLMConfig,
}

impl HttpClient {
    pub fn new(config: LLMConfig) -> LLMResult<Self> {
        let client = Client::builder()
            .timeout(config.timeout)
            .build()
            .map_err(LLMError::RequestFailed)?;

        Ok(Self { client, config })
    }

    /// Build a request with common headers and auth.
    fn build_request(&self, method: reqwest::Method, url: &str) -> RequestBuilder {
        let mut builder = self
            .client
            .request(method, url)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json");

        if let Some(api_key) = &self.config.api_key {
            builder = builder.bearer_auth(api_key);
        }

        if let Some(org) = &self.config.organization {
            builder = builder.header("OpenAI-Organization", org);
        }

        for (key, value) in &self.config.extra_headers {
            builder = builder.header(key, value);
        }

        builder
    }

    /// Execute a request with retries and error handling.
    pub async fn execute<T: DeserializeOwned>(
        &self,
        mut request: RequestBuilder,
    ) -> LLMResult<T> {
        let mut last_error = None;

        for attempt in 0..=self.config.max_retries {
            let response = match request.try_clone() {
                Some(req) => match req.send().await {
                    Ok(resp) => resp,
                    Err(e) => {
                        last_error = Some(LLMError::RequestFailed(e));
                        if attempt < self.config.max_retries {
                            tokio::time::sleep(Duration::from_millis(100 * (1 << attempt))).await;
                            continue;
                        }
                        break;
                    }
                },
                None => {
                    last_error = Some(LLMError::RequestFailed(
                        reqwest::Error::from(reqwest::ErrorKind::Request),
                    ));
                    break;
                }
            };

            return self.handle_response(response).await;
        }

        Err(last_error.unwrap_or(LLMError::RequestFailed(
            reqwest::Error::from(reqwest::ErrorKind::Request),
        )))
    }

    /// Execute a streaming request.
    pub async fn execute_stream(
        &self,
        request: RequestBuilder,
    ) -> LLMResult<Pin<Box<dyn Stream<Item = LLMResult<CompletionChunk>> + Send>>> {
        let response = request.send().await.map_err(LLMError::RequestFailed)?;
        self.handle_streaming_response(response).await
    }

    /// Handle non-streaming response.
    async fn handle_response<T: DeserializeOwned>(&self, response: Response) -> LLMResult<T> {
        let status = response.status();

        if status.is_success() {
            let body = response
                .bytes()
                .await
                .map_err(LLMError::RequestFailed)?;
            serde_json::from_slice(&body).map_err(|e| {
                LLMError::InvalidResponse(format!("Failed to parse response: {}", e))
            })
        } else {
            let error_body = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            Err(self.map_api_error(status, error_body))
        }
    }

    /// Handle streaming response (SSE).
    async fn handle_streaming_response(
        &self,
        response: Response,
    ) -> LLMResult<Pin<Box<dyn Stream<Item = LLMResult<CompletionChunk>> + Send>>> {
        let status = response.status();

        if !status.is_success() {
            let error_body = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            return Err(self.map_api_error(status, error_body));
        }

        let (tx, rx) = mpsc::channel(32);
        let stream = response.bytes_stream();

        tokio::spawn(async move {
            let mut framed = FramedRead::new(stream.into_async_read(), BytesCodec::new());
            let mut buffer = String::new();

            while let Some(chunk) = framed.next().await {
                match chunk {
                    Ok(bytes) => {
                        buffer.push_str(&String::from_utf8_lossy(&bytes));

                        while let Some(line_end) = buffer.find('\n') {
                            let line = buffer.drain(..=line_end).collect::<String>();
                            let line = line.trim();

                            if line.is_empty() {
                                continue;
                            }

                            if let Some(data) = line.strip_prefix(SSE_PREFIX) {
                                if data == SSE_DONE {
                                    return;
                                }

                                match serde_json::from_str::<CompletionChunk>(data) {
                                    Ok(chunk) => {
                                        if tx.send(Ok(chunk)).await.is_err() {
                                            return;
                                        }
                                    }
                                    Err(e) => {
                                        let _ = tx
                                            .send(Err(LLMError::InvalidResponse(format!(
                                                "Failed to parse chunk: {}",
                                                e
                                            ))))
                                            .await;
                                        return;
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        let _ = tx
                            .send(Err(LLMError::StreamError(e.to_string())))
                            .await;
                        return;
                    }
                }
            }
        });

        Ok(Box::pin(ReceiverStream::new(rx)))
    }

    /// Map HTTP status to LLMError.
    fn map_api_error(&self, status: StatusCode, body: String) -> LLMError {
        match status.as_u16() {
            400 => LLMError::ApiError {
                status: 400,
                message: body,
            },
            401 => LLMError::AuthError(body),
            403 => LLMError::AuthError(body),
            404 => LLMError::ModelNotFound(body),
            429 => {
                let retry_after = response_headers_retry_after(&body).or_else(|| {
                    // Try parsing from body if it's JSON
                    serde_json::from_str::<serde_json::Value>(&body)
                        .ok()
                        .and_then(|v| v.get("retry_after"))
                        .and_then(|v| v.as_u64())
                        .map(Duration::from_secs)
                });
                LLMError::RateLimited { retry_after }
            }
            500..=599 => LLMError::ApiError {
                status: status.as_u16(),
                message: body,
            },
            _ => LLMError::ApiError {
                status: status.as_u16(),
                message: body,
            },
        }
    }
}

/// Extract retry-after from response headers (placeholder for actual header access).
fn response_headers_retry_after(_body: &str) -> Option<Duration> {
    // In real implementation, extract from response.headers()
    None
}

/// Merge default options with request-specific options.
pub fn merge_options(
    defaults: &GenerationOptions,
    request: Option<GenerationOptions>,
) -> GenerationOptions {
    let mut merged = defaults.clone();

    if let Some(req) = request {
        merged.temperature = req.temperature.or(merged.temperature);
        merged.top_p = req.top_p.or(merged.top_p);
        merged.top_k = req.top_k.or(merged.top_k);
        merged.max_tokens = req.max_tokens.or(merged.max_tokens);
        merged.min_tokens = req.min_tokens.or(merged.min_tokens);
        merged.presence_penalty = req.presence_penalty.or(merged.presence_penalty);
        merged.frequency_penalty = req.frequency_penalty.or(merged.frequency_penalty);
        merged.repetition_penalty = req.repetition_penalty.or(merged.repetition_penalty);
        merged.stop = req.stop.or(merged.stop);
        merged.seed = req.seed.or(merged.seed);
        merged.logit_bias = req.logit_bias.or(merged.logit_bias);
        merged.user = req.user.or(merged.user);
        merged.stream = req.stream;
        merged.tools = req.tools.or(merged.tools);
        merged.tool_choice = req.tool_choice.or(merged.tool_choice);
        merged.response_format = req.response_format.or(merged.response_format);
        merged.parallel_tool_calls = req.parallel_tool_calls.or(merged.parallel_tool_calls);
    }

    merged
}

/// Build the request body for completion.
pub fn build_completion_body(
    messages: Vec<Message>,
    options: &GenerationOptions,
    extra_body: &std::collections::HashMap<String, serde_json::Value>,
) -> serde_json::Value {
    let mut body = serde_json::json!({
        "messages": messages,
        "stream": options.stream,
    });

    if let Some(temp) = options.temperature {
        body["temperature"] = serde_json::json!(temp);
    }
    if let Some(top_p) = options.top_p {
        body["top_p"] = serde_json::json!(top_p);
    }
    if let Some(top_k) = options.top_k {
        body["top_k"] = serde_json::json!(top_k);
    }
    if let Some(max_tokens) = options.max_tokens {
        body["max_tokens"] = serde_json::json!(max_tokens);
    }
    if let Some(min_tokens) = options.min_tokens {
        body["min_tokens"] = serde_json::json!(min_tokens);
    }
    if let Some(presence_penalty) = options.presence_penalty {
        body["presence_penalty"] = serde_json::json!(presence_penalty);
    }
    if let Some(frequency_penalty) = options.frequency_penalty {
        body["frequency_penalty"] = serde_json::json!(frequency_penalty);
    }
    if let Some(repetition_penalty) = options.repetition_penalty {
        body["repetition_penalty"] = serde_json::json!(repetition_penalty);
    }
    if let Some(stop) = &options.stop {
        body["stop"] = serde_json::json!(stop);
    }
    if let Some(seed) = options.seed {
        body["seed"] = serde_json::json!(seed);
    }
    if let Some(logit_bias) = &options.logit_bias {
        body["logit_bias"] = serde_json::json!(logit_bias);
    }
    if let Some(user) = &options.user {
        body["user"] = serde_json::json!(user);
    }
    if let Some(tools) = &options.tools {
        body["tools"] = serde_json::json!(tools);
    }
    if let Some(tool_choice) = &options.tool_choice {
        body["tool_choice"] = serde_json::json!(tool_choice);
    }
    if let Some(response_format) = &options.response_format {
        body["response_format"] = serde_json::json!(response_format);
    }
    if let Some(parallel_tool_calls) = options.parallel_tool_calls {
        body["parallel_tool_calls"] = serde_json::json!(parallel_tool_calls);
    }

    // Merge extra body parameters
    for (key, value) in extra_body {
        body[key] = value.clone();
    }

    body
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{GenerationOptions, Message, Role};

    #[test]
    fn test_merge_options() {
        let defaults = GenerationOptions {
            temperature: Some(0.5),
            max_tokens: Some(100),
            ..Default::default()
        };

        let request = Some(GenerationOptions {
            temperature: Some(0.8),
            top_p: Some(0.9),
            ..Default::default()
        });

        let merged = merge_options(&defaults, request);

        assert_eq!(merged.temperature, Some(0.8));
        assert_eq!(merged.top_p, Some(0.9));
        assert_eq!(merged.max_tokens, Some(100));
    }

    #[test]
    fn test_build_completion_body() {
        let messages = vec![Message {
            role: Role::User,
            content: crate::types::MessageContent::Text("Hello".to_string()),
            name: None,
            tool_calls: None,
            tool_call_id: None,
        }];

        let options = GenerationOptions {
            temperature: Some(0.7),
            max_tokens: Some(50),
            stream: true,
            ..Default::default()
        };

        let body = build_completion_body(messages, &options, &Default::default());

        assert_eq!(body["stream"], true);
        assert_eq!(body["temperature"], 0.7);
        assert_eq!(body["max_tokens"], 50);
        assert!(body["messages"].is_array());
    }
}