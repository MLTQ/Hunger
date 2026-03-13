use anyhow::{Context, Result, anyhow};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    config::Config,
    models::{CheapSignals, LinkHint, LlmNovelty, PageDigest, PageRecord},
};

#[derive(Clone)]
pub struct OpenAiCompatibleClient {
    http: reqwest::Client,
    base_url: String,
    api_key: String,
    chat_model: String,
    embedding_model: Option<String>,
}

impl OpenAiCompatibleClient {
    pub fn new(config: &Config) -> Result<Self> {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        let auth_value = HeaderValue::from_str(&format!("Bearer {}", config.llm_api_key))
            .context("invalid HUNGER_LLM_API_KEY header")?;
        headers.insert(AUTHORIZATION, auth_value);

        let http = reqwest::Client::builder()
            .timeout(config.llm_timeout)
            .default_headers(headers)
            .build()
            .context("failed to build reqwest client")?;

        Ok(Self {
            http,
            base_url: config.llm_base_url.trim_end_matches('/').to_string(),
            api_key: config.llm_api_key.clone(),
            chat_model: config.llm_model.clone(),
            embedding_model: config.embedding_model.clone(),
        })
    }

    pub async fn embed_text(&self, text: &str) -> Result<Option<Vec<f32>>> {
        let Some(model) = self.embedding_model.as_ref() else {
            return Ok(None);
        };

        let request = EmbeddingRequest { model, input: text };
        let response = self
            .http
            .post(format!("{}/embeddings", self.base_url))
            .json(&request)
            .send()
            .await
            .context("failed to call embedding endpoint")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(anyhow!("embedding request failed with {status}: {body}"));
        }

        let payload: EmbeddingResponse = response
            .json()
            .await
            .context("invalid embedding response")?;
        Ok(payload.data.into_iter().next().map(|item| item.embedding))
    }

    pub async fn score_novelty(
        &self,
        candidate: &PageDigest,
        similar_pages: &[PageRecord],
        same_domain_pages: &[PageRecord],
        cheap_signals: &CheapSignals,
    ) -> Result<LlmNovelty> {
        let prompt = build_prompt(candidate, similar_pages, same_domain_pages, cheap_signals);
        let request = ChatCompletionRequest {
            model: self.chat_model.clone(),
            temperature: Some(0.1),
            max_tokens: Some(420),
            response_format: Some(ResponseFormat::json_object()),
            messages: vec![
                Message {
                    role: "system".to_string(),
                    content: SYSTEM_PROMPT.to_string(),
                },
                Message {
                    role: "user".to_string(),
                    content: prompt,
                },
            ],
        };
        let content = self
            .send_chat_completion(request, "chat completion endpoint")
            .await?;

        match parse_llm_json(&content) {
            Ok(novelty) => Ok(novelty),
            Err(parse_error) => {
                tracing::debug!(
                    error = %parse_error,
                    raw_response = %compact_text(&content, 360),
                    "primary novelty response was not directly parseable; attempting JSON repair"
                );
                self.repair_novelty_response(&content)
                    .await
                    .with_context(|| {
                        format!(
                            "model response was not usable as JSON; primary parse error: {parse_error}"
                        )
                    })
            }
        }
    }

    pub fn auth_mode(&self) -> &'static str {
        if self.api_key.is_empty() {
            "none"
        } else {
            "bearer"
        }
    }
}

const SYSTEM_PROMPT: &str = r#"You are scoring novelty for a memory-based crawler.
Judge novelty relative to the supplied crawler memory, not relative to vague pretraining knowledge.
Reward new facts, new entities, new relationships, and promising source trails.
Penalize restatements, shallow directory pages, boilerplate, and repetitive near-duplicates.
Put your reasoning inside the JSON fields instead of before or after the object.
Return a single valid JSON object, with no markdown fences and no surrounding text."#;

const REPAIR_SYSTEM_PROMPT: &str = r#"You repair crawler novelty judgments into valid JSON.
Preserve the source reasoning in the `analysis` field.
If a score is missing, infer conservatively from the provided text.
Return a single valid JSON object and no other text."#;

fn build_prompt(
    candidate: &PageDigest,
    similar_pages: &[PageRecord],
    same_domain_pages: &[PageRecord],
    cheap_signals: &CheapSignals,
) -> String {
    let nearby = similar_pages
        .iter()
        .take(4)
        .map(|page| format!("- {} | {} | {}", page.url, page.title, page.summary))
        .collect::<Vec<_>>()
        .join("\n");

    let same_domain = same_domain_pages
        .iter()
        .take(3)
        .map(|page| format!("- {} | {}", page.url, page.summary))
        .collect::<Vec<_>>()
        .join("\n");

    let outbound = candidate
        .outbound_links
        .iter()
        .take(10)
        .map(|link| format!("- {} | {}", link.url, link.anchor))
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        r#"Candidate page:
url: {url}
title: {title}
summary: {summary}
key_points:
{key_points}
entities:
{entities}
claims:
{claims}
outbound_links:
{outbound}

Nearby pages already in memory:
{nearby}

Recent pages on the same domain:
{same_domain}

Cheap signals:
- semantic_distance: {semantic_distance:.3}
- new_token_ratio: {new_token_ratio:.3}
- domain_redundancy: {domain_redundancy:.3}
- local_graph_diversity: {local_graph_diversity:.3}
- content_delta_from_last_snapshot: {content_delta:.3}
- boilerplate_penalty: {boilerplate_penalty:.3}

Return JSON:
{{
  "factual_novelty": 0.0-1.0,
  "entity_novelty": 0.0-1.0,
  "relational_novelty": 0.0-1.0,
  "temporal_novelty": 0.0-1.0,
  "expected_crawl_value": 0.0-1.0,
  "analysis": "2-6 sentences explaining the judgment",
  "concise_reason": "one sentence",
  "likely_novel_aspects": ["..."],
  "likely_redundant_aspects": ["..."],
  "recommended_action": "expand|sample|ignore",
  "recommended_link_hints": [{{"link_url": "...", "reason": "...", "priority": 0.0-1.0}}]
}}"#,
        url = candidate.url,
        title = compact_text(&candidate.title, 120),
        summary = compact_text(&candidate.short_summary, 340),
        key_points = candidate
            .key_points
            .iter()
            .take(4)
            .map(|point| compact_text(point, 140))
            .map(|point| format!("- {point}"))
            .collect::<Vec<_>>()
            .join("\n"),
        entities = candidate
            .entities
            .iter()
            .take(10)
            .map(|entity| compact_text(entity, 40))
            .map(|entity| format!("- {entity}"))
            .collect::<Vec<_>>()
            .join("\n"),
        claims = candidate
            .claims
            .iter()
            .take(4)
            .map(|claim| compact_text(claim, 180))
            .map(|claim| format!("- {claim}"))
            .collect::<Vec<_>>()
            .join("\n"),
        outbound = compact_lines(&outbound, 900),
        nearby = compact_lines(&nearby, 1200),
        same_domain = compact_lines(&same_domain, 800),
        semantic_distance = cheap_signals.semantic_distance,
        new_token_ratio = cheap_signals.new_token_ratio,
        domain_redundancy = cheap_signals.domain_redundancy,
        local_graph_diversity = cheap_signals.local_graph_diversity,
        content_delta = cheap_signals.content_delta_from_last_snapshot,
        boilerplate_penalty = cheap_signals.boilerplate_penalty,
    )
}

impl OpenAiCompatibleClient {
    async fn repair_novelty_response(&self, raw_content: &str) -> Result<LlmNovelty> {
        let request = ChatCompletionRequest {
            model: self.chat_model.clone(),
            temperature: Some(0.0),
            max_tokens: Some(320),
            response_format: Some(ResponseFormat::json_object()),
            messages: vec![
                Message {
                    role: "system".to_string(),
                    content: REPAIR_SYSTEM_PROMPT.to_string(),
                },
                Message {
                    role: "user".to_string(),
                    content: format!(
                        r#"Convert the following crawler novelty judgment into valid JSON with this shape:
{{
  "factual_novelty": 0.0-1.0,
  "entity_novelty": 0.0-1.0,
  "relational_novelty": 0.0-1.0,
  "temporal_novelty": 0.0-1.0,
  "expected_crawl_value": 0.0-1.0,
  "analysis": "2-6 sentences explaining the judgment",
  "concise_reason": "one sentence",
  "likely_novel_aspects": ["..."],
  "likely_redundant_aspects": ["..."],
  "recommended_action": "expand|sample|ignore",
  "recommended_link_hints": [{{"link_url": "...", "reason": "...", "priority": 0.0-1.0}}]
}}

Raw model output:
{raw_content}"#
                    ),
                },
            ],
        };
        let content = self
            .send_chat_completion(request, "repair chat completion endpoint")
            .await?;

        parse_llm_json(&content)
    }

    async fn send_chat_completion(
        &self,
        request: ChatCompletionRequest,
        endpoint_label: &str,
    ) -> Result<String> {
        let mut request = request;

        loop {
            let response = self
                .http
                .post(format!("{}/chat/completions", self.base_url))
                .json(&request)
                .send()
                .await
                .with_context(|| format!("failed to call {endpoint_label}"))?;

            if response.status().is_success() {
                let payload: ChatCompletionResponse = response
                    .json()
                    .await
                    .with_context(|| format!("invalid {endpoint_label} response"))?;
                return payload
                    .choices
                    .into_iter()
                    .next()
                    .map(|choice| choice.message.content)
                    .ok_or_else(|| anyhow!("{endpoint_label} returned no choices"));
            }

            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            if should_retry_without_response_format(
                status.as_u16(),
                &body,
                request.response_format.as_ref(),
            ) {
                tracing::debug!(
                    status = %status,
                    body = %compact_text(&body, 240),
                    "chat endpoint rejected response_format; retrying without structured-output hint"
                );
                request.response_format = None;
                continue;
            }

            return Err(anyhow!("{endpoint_label} failed with {status}: {body}"));
        }
    }
}

fn parse_llm_json(content: &str) -> Result<LlmNovelty> {
    let json =
        extract_json_object(content).ok_or_else(|| anyhow!("model response was not JSON"))?;
    let mut novelty: LlmNovelty =
        serde_json::from_str(&json).context("failed to parse model JSON")?;

    clamp_novelty(&mut novelty);
    Ok(novelty)
}

fn extract_json_object(content: &str) -> Option<String> {
    let trimmed = content.trim();
    if serde_json::from_str::<Value>(trimmed).is_ok() {
        return Some(trimmed.to_string());
    }

    if let Some(fenced) = extract_fenced_json(trimmed) {
        return Some(fenced);
    }

    for (start, ch) in trimmed.char_indices() {
        if ch != '{' {
            continue;
        }

        if let Some(candidate) = extract_balanced_json(&trimmed[start..]) {
            return Some(candidate);
        }
    }

    None
}

fn extract_fenced_json(content: &str) -> Option<String> {
    let fence_start = content.find("```")?;
    let rest = &content[fence_start + 3..];
    let rest = rest
        .strip_prefix("json")
        .map(str::trim_start)
        .unwrap_or(rest);
    let fence_end = rest.find("```")?;
    let candidate = rest[..fence_end].trim();
    serde_json::from_str::<Value>(candidate)
        .ok()
        .map(|_| candidate.to_string())
}

fn extract_balanced_json(content: &str) -> Option<String> {
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;

    for (idx, ch) in content.char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }

        match ch {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    let candidate = &content[..=idx];
                    if serde_json::from_str::<Value>(candidate).is_ok() {
                        return Some(candidate.to_string());
                    }
                }
            }
            _ => {}
        }
    }

    None
}

fn should_retry_without_response_format(
    status_code: u16,
    body: &str,
    response_format: Option<&ResponseFormat>,
) -> bool {
    if status_code != 400 || response_format.is_none() {
        return false;
    }

    let normalized = body.to_ascii_lowercase();
    normalized.contains("response_format")
        && (normalized.contains("json_object")
            || normalized.contains("json_schema")
            || normalized.contains("must be 'json_schema' or 'text'")
            || normalized.contains("unsupported"))
}

fn compact_lines(value: &str, max_chars: usize) -> String {
    compact_text(value, max_chars)
}

fn compact_text(value: &str, max_chars: usize) -> String {
    let normalized = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.chars().count() <= max_chars {
        normalized
    } else {
        let compact = normalized.chars().take(max_chars).collect::<String>();
        format!("{compact}...")
    }
}

fn clamp_novelty(novelty: &mut LlmNovelty) {
    novelty.factual_novelty = novelty.factual_novelty.clamp(0.0, 1.0);
    novelty.entity_novelty = novelty.entity_novelty.clamp(0.0, 1.0);
    novelty.relational_novelty = novelty.relational_novelty.clamp(0.0, 1.0);
    novelty.temporal_novelty = novelty.temporal_novelty.clamp(0.0, 1.0);
    novelty.expected_crawl_value = novelty.expected_crawl_value.clamp(0.0, 1.0);
    for LinkHint { priority, .. } in &mut novelty.recommended_link_hints {
        *priority = priority.clamp(0.0, 1.0);
    }
}

#[derive(Debug, Serialize)]
struct EmbeddingRequest<'a> {
    model: &'a str,
    input: &'a str,
}

#[derive(Debug, Deserialize)]
struct EmbeddingResponse {
    data: Vec<EmbeddingData>,
}

#[derive(Debug, Deserialize)]
struct EmbeddingData {
    embedding: Vec<f32>,
}

#[derive(Clone, Debug, Serialize)]
struct ChatCompletionRequest {
    model: String,
    messages: Vec<Message>,
    temperature: Option<f32>,
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<ResponseFormat>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Message {
    role: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct ChatCompletionResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    message: Message,
}

#[derive(Clone, Debug, Serialize)]
struct ResponseFormat {
    #[serde(rename = "type")]
    response_type: String,
}

impl ResponseFormat {
    fn json_object() -> Self {
        Self {
            response_type: "json_object".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ResponseFormat, parse_llm_json, should_retry_without_response_format};

    #[test]
    fn parses_embedded_json_after_reasoning_preamble() {
        let content = r#"Here is the judgment.

{
  "factual_novelty": 0.2,
  "entity_novelty": 0.1,
  "relational_novelty": 0.3,
  "temporal_novelty": 0.4,
  "expected_crawl_value": 0.5,
  "analysis": "This page mostly repeats known archive navigation.",
  "concise_reason": "Low novelty archive page.",
  "likely_novel_aspects": ["specific date grouping"],
  "likely_redundant_aspects": ["directory structure"],
  "recommended_action": "ignore",
  "recommended_link_hints": []
}"#;

        let novelty = parse_llm_json(content).expect("should parse embedded JSON");
        assert_eq!(novelty.recommended_action, "ignore");
        assert_eq!(
            novelty.analysis,
            "This page mostly repeats known archive navigation."
        );
    }

    #[test]
    fn missing_analysis_defaults_cleanly_for_old_rows() {
        let content = r#"{
  "factual_novelty": 0.2,
  "entity_novelty": 0.1,
  "relational_novelty": 0.3,
  "temporal_novelty": 0.4,
  "expected_crawl_value": 0.5,
  "concise_reason": "Low novelty archive page.",
  "likely_novel_aspects": [],
  "likely_redundant_aspects": ["directory structure"],
  "recommended_action": "ignore",
  "recommended_link_hints": []
}"#;

        let novelty = parse_llm_json(content).expect("should parse legacy JSON");
        assert!(novelty.analysis.is_empty());
        assert_eq!(novelty.recommended_action, "ignore");
    }

    #[test]
    fn retries_when_response_format_is_rejected() {
        assert!(should_retry_without_response_format(
            400,
            r#"{"error":"'response_format.type' must be 'json_schema' or 'text'"}"#,
            Some(&ResponseFormat::json_object())
        ));
    }

    #[test]
    fn does_not_retry_plain_400s_without_response_format_issue() {
        assert!(!should_retry_without_response_format(
            400,
            r#"{"error":"unknown model"}"#,
            Some(&ResponseFormat::json_object())
        ));
    }
}
