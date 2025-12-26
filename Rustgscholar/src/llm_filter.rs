//! LLM-based relevance filtering for academic papers.
//!
//! This module provides concurrent LLM API calls to classify papers
//! as relevant, irrelevant, or uncertain based on user-provided keywords.

use crate::error::{GscholarError, Result};
use crate::prompts::relevance_filter::{build_user_prompt, SYSTEM_PROMPT};
use crate::unified::UnifiedResult;
use futures::stream::{self, StreamExt};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Semaphore;
use tracing::{debug, info, warn};

/// Maximum concurrent LLM API requests
const MAX_CONCURRENT_REQUESTS: usize = 10;

/// Request timeout in seconds
const REQUEST_TIMEOUT_SECS: u64 = 60;

/// LLM configuration
#[derive(Debug, Clone)]
pub struct LlmConfig {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    pub filter_help: String,
}

/// Filter result for a single paper
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilterResult {
    pub id: String,
    pub title: String,
    pub label: String,
    pub confidence: f64,
    /// Evidence as comma-separated string for CSV compatibility
    pub evidence: String,
    pub reason: String,
}

/// Token usage tracking
#[derive(Debug, Clone, Default)]
pub struct TokenUsage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
}

/// Accumulated token usage with atomic counters
struct AtomicTokenUsage {
    prompt_tokens: AtomicU64,
    completion_tokens: AtomicU64,
    total_tokens: AtomicU64,
}

impl AtomicTokenUsage {
    fn new() -> Self {
        Self {
            prompt_tokens: AtomicU64::new(0),
            completion_tokens: AtomicU64::new(0),
            total_tokens: AtomicU64::new(0),
        }
    }

    fn add(&self, usage: &TokenUsage) {
        self.prompt_tokens.fetch_add(usage.prompt_tokens, Ordering::Relaxed);
        self.completion_tokens.fetch_add(usage.completion_tokens, Ordering::Relaxed);
        self.total_tokens.fetch_add(usage.total_tokens, Ordering::Relaxed);
    }

    fn get(&self) -> TokenUsage {
        TokenUsage {
            prompt_tokens: self.prompt_tokens.load(Ordering::Relaxed),
            completion_tokens: self.completion_tokens.load(Ordering::Relaxed),
            total_tokens: self.total_tokens.load(Ordering::Relaxed),
        }
    }
}

/// OpenAI-compatible API response structures
#[derive(Debug, Deserialize)]
struct ChatCompletionResponse {
    choices: Vec<ChatChoice>,
    usage: Option<ApiUsage>,
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    message: ChatMessage,
}

#[derive(Debug, Deserialize)]
struct ChatMessage {
    content: String,
}

#[derive(Debug, Deserialize)]
struct ApiUsage {
    prompt_tokens: u64,
    completion_tokens: u64,
    total_tokens: u64,
}

/// Paper data for LLM input (subset of UnifiedResult)
#[derive(Debug, Serialize)]
struct PaperForLlm {
    id: String,
    title: String,
    abstract_text: String,
    tldr: String,
    journal: String,
    author: String,
    date: String,
}

impl From<&UnifiedResult> for PaperForLlm {
    fn from(r: &UnifiedResult) -> Self {
        Self {
            id: r.doi.clone(),
            title: r.title.clone(),
            abstract_text: r.abstract_text.clone(),
            tldr: r.tldr.clone(),
            journal: r.journal.clone(),
            author: r.author.clone(),
            date: r.date.clone(),
        }
    }
}

/// Filter papers using LLM with concurrent requests.
///
/// Each paper is sent as a separate API request for maximum parallelism.
/// Results are collected and returned with total token usage.
pub async fn filter_papers(
    config: &LlmConfig,
    papers: &[UnifiedResult],
) -> Result<(Vec<FilterResult>, TokenUsage)> {
    if papers.is_empty() {
        return Ok((Vec::new(), TokenUsage::default()));
    }

    info!(
        count = papers.len(),
        model = %config.model,
        "Starting LLM relevance filtering"
    );

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
        .build()
        .map_err(|e| GscholarError::Config(format!("Failed to build HTTP client: {}", e)))?;

    let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT_REQUESTS));
    let token_usage = Arc::new(AtomicTokenUsage::new());
    let client = Arc::new(client);
    let config = Arc::new(config.clone());

    // Process papers concurrently
    let results: Vec<FilterResult> = stream::iter(papers.iter().enumerate())
        .map(|(idx, paper)| {
            let semaphore = Arc::clone(&semaphore);
            let token_usage = Arc::clone(&token_usage);
            let client = Arc::clone(&client);
            let config = Arc::clone(&config);

            async move {
                let _permit = semaphore.acquire().await.ok()?;
                
                match filter_single_paper(&client, &config, paper, idx).await {
                    Ok((result, usage)) => {
                        token_usage.add(&usage);
                        Some(result)
                    }
                    Err(e) => {
                        warn!(
                            idx = idx,
                            title = %paper.title.chars().take(50).collect::<String>(),
                            error = %e,
                            "Failed to filter paper"
                        );
                        // Return uncertain for failed requests
                        Some(FilterResult {
                            id: paper.doi.clone(),
                            title: paper.title.clone(),
                            label: "uncertain".to_string(),
                            confidence: 0.0,
                            evidence: String::new(),
                            reason: format!("API error: {}", e),
                        })
                    }
                }
            }
        })
        .buffer_unordered(MAX_CONCURRENT_REQUESTS)
        .filter_map(|r| async { r })
        .collect()
        .await;

    let final_usage = token_usage.get();
    info!(
        filtered = results.len(),
        prompt_tokens = final_usage.prompt_tokens,
        completion_tokens = final_usage.completion_tokens,
        "LLM filtering complete"
    );

    Ok((results, final_usage))
}

/// Filter a single paper via LLM API
async fn filter_single_paper(
    client: &reqwest::Client,
    config: &LlmConfig,
    paper: &UnifiedResult,
    idx: usize,
) -> Result<(FilterResult, TokenUsage)> {
    let paper_data = PaperForLlm::from(paper);
    let paper_json = serde_json::to_string_pretty(&paper_data)
        .map_err(|e| GscholarError::Parse(format!("Failed to serialize paper: {}", e)))?;

    let user_prompt = build_user_prompt(&config.filter_help, &paper_json);

    // Build OpenAI-compatible request
    let request_body = serde_json::json!({
        "model": config.model,
        "messages": [
            {"role": "system", "content": SYSTEM_PROMPT},
            {"role": "user", "content": user_prompt}
        ],
        "temperature": 0.1,
        "max_tokens": 20000
    });

    let api_url = format!("{}/chat/completions", config.base_url.trim_end_matches('/'));

    debug!(idx = idx, "Sending LLM request");

    let response = client
        .post(&api_url)
        .header("Content-Type", "application/json")
        .header("Authorization", format!("Bearer {}", config.api_key))
        .json(&request_body)
        .send()
        .await
        .map_err(GscholarError::Network)?;

    if !response.status().is_success() {
        let status = response.status();
        let error_text = response.text().await.unwrap_or_default();
        return Err(GscholarError::Api {
            code: status.as_u16() as i32,
            message: format!("LLM API error: {} - {}", status, error_text),
        });
    }

    let api_response: ChatCompletionResponse = response
        .json()
        .await
        .map_err(|e| GscholarError::Parse(format!("Failed to parse LLM response: {}", e)))?;

    // Extract usage
    let usage = api_response.usage.map(|u| TokenUsage {
        prompt_tokens: u.prompt_tokens,
        completion_tokens: u.completion_tokens,
        total_tokens: u.total_tokens,
    }).unwrap_or_default();

    // Parse LLM output
    let content = api_response
        .choices
        .first()
        .map(|c| c.message.content.clone())
        .unwrap_or_default();

    let result = parse_llm_response(&content, &paper.doi, &paper.title)?;

    debug!(
        idx = idx,
        label = %result.label,
        "Paper classified"
    );

    Ok((result, usage))
}

/// Parse LLM JSON response into FilterResult
fn parse_llm_response(content: &str, id: &str, title: &str) -> Result<FilterResult> {
    // Try to extract JSON from the response (handle markdown code blocks)
    let json_str = extract_json(content);

    #[derive(Deserialize)]
    struct LlmOutput {
        label: String,
        confidence: f64,
        evidence: Vec<String>,
        reason: String,
    }

    match serde_json::from_str::<LlmOutput>(&json_str) {
        Ok(output) => Ok(FilterResult {
            id: id.to_string(),
            title: title.to_string(),
            label: output.label,
            confidence: output.confidence,
            evidence: output.evidence.join(", "),
            reason: output.reason,
        }),
        Err(e) => {
            // Log truncated content for debugging (first 200 chars)
            let preview: String = content.chars().take(200).collect();
            info!(
                error = %e,
                content_preview = %preview,
                "LLM output parse failed - treating as uncertain"
            );
            // Return uncertain for parse failures
            Ok(FilterResult {
                id: id.to_string(),
                title: title.to_string(),
                label: "uncertain".to_string(),
                confidence: 0.0,
                evidence: String::new(),
                reason: format!("Parse error: {}", e),
            })
        }
    }
}

/// Extract JSON from LLM response (handles markdown code blocks)
fn extract_json(content: &str) -> String {
    let trimmed = content.trim();
    
    // Check for markdown code block
    if trimmed.starts_with("```") {
        let lines: Vec<&str> = trimmed.lines().collect();
        if lines.len() >= 2 {
            let start = if lines[0].starts_with("```json") || lines[0] == "```" { 1 } else { 0 };
            let end = if lines.last().map(|l| l.trim()) == Some("```") {
                lines.len() - 1
            } else {
                lines.len()
            };
            return lines[start..end].join("\n");
        }
    }
    
    // Try to find JSON object in the text
    if let Some(start) = trimmed.find('{') {
        if let Some(end) = trimmed.rfind('}') {
            return trimmed[start..=end].to_string();
        }
    }
    
    trimmed.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_json_plain() {
        let input = r#"{"label": "relevant", "confidence": 0.9, "evidence": [], "reason": "test"}"#;
        let result = extract_json(input);
        assert!(result.contains("\"label\": \"relevant\""));
    }

    #[test]
    fn test_extract_json_code_block() {
        let input = r#"```json
{"label": "relevant", "confidence": 0.9, "evidence": [], "reason": "test"}
```"#;
        let result = extract_json(input);
        assert!(result.contains("\"label\": \"relevant\""));
    }

    #[test]
    fn test_extract_json_with_text() {
        let input = r#"Here is the result: {"label": "irrelevant", "confidence": 0.8, "evidence": ["stock"], "reason": "not related"}"#;
        let result = extract_json(input);
        assert!(result.starts_with('{'));
        assert!(result.ends_with('}'));
    }

    #[test]
    fn test_parse_llm_response() {
        let content = r#"{"label": "relevant", "confidence": 0.95, "evidence": ["landslide", "slope"], "reason": "Explicitly involves landslide research"}"#;
        let result = parse_llm_response(content, "10.1234/test", "Test Paper").unwrap();
        assert_eq!(result.label, "relevant");
        assert_eq!(result.confidence, 0.95);
        assert!(result.evidence.contains("landslide"));
        assert!(result.evidence.contains("slope"));
    }
}
