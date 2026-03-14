use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SemanticAxis {
    pub negative_label: String,
    pub positive_label: String,
}

impl SemanticAxis {
    pub fn is_configured(&self) -> bool {
        !self.negative_label.trim().is_empty() && !self.positive_label.trim().is_empty()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OutboundLink {
    pub url: String,
    pub anchor: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PageDigest {
    pub url: String,
    pub domain: String,
    pub title: String,
    pub clean_text: String,
    pub short_summary: String,
    pub key_points: Vec<String>,
    pub entities: Vec<String>,
    pub claims: Vec<String>,
    pub outbound_links: Vec<OutboundLink>,
    pub boilerplate_ratio: f32,
    pub content_hash: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CheapSignals {
    pub semantic_distance: f32,
    pub new_token_ratio: f32,
    pub domain_redundancy: f32,
    pub local_graph_diversity: f32,
    pub content_delta_from_last_snapshot: f32,
    pub boilerplate_penalty: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct LlmNovelty {
    pub factual_novelty: f32,
    pub entity_novelty: f32,
    pub relational_novelty: f32,
    pub temporal_novelty: f32,
    pub expected_crawl_value: f32,
    pub analysis: String,
    pub concise_reason: String,
    pub likely_novel_aspects: Vec<String>,
    pub likely_redundant_aspects: Vec<String>,
    pub recommended_action: String,
    pub recommended_link_hints: Vec<LinkHint>,
    pub axis_scores: Vec<AxisScore>,
}

impl LlmNovelty {
    pub fn fallback() -> Self {
        Self {
            factual_novelty: 0.0,
            entity_novelty: 0.0,
            relational_novelty: 0.0,
            temporal_novelty: 0.0,
            expected_crawl_value: 0.0,
            analysis: String::new(),
            concise_reason: "Model judgment unavailable; using heuristic fallback.".to_string(),
            likely_novel_aspects: Vec::new(),
            likely_redundant_aspects: Vec::new(),
            recommended_action: "sample".to_string(),
            recommended_link_hints: Vec::new(),
            axis_scores: Vec::new(),
        }
    }
}

impl Default for LlmNovelty {
    fn default() -> Self {
        Self::fallback()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LinkHint {
    pub link_url: String,
    pub reason: String,
    pub priority: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct AxisScore {
    pub negative_label: String,
    pub positive_label: String,
    pub score: f32,
    pub explanation: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NoveltyScore {
    pub food_energy: f32,
    pub factual_novelty: f32,
    pub entity_novelty: f32,
    pub relational_novelty: f32,
    pub temporal_novelty: f32,
    pub expected_crawl_value: f32,
    pub concise_reason: String,
    pub likely_novel_aspects: Vec<String>,
    pub likely_redundant_aspects: Vec<String>,
    pub semantic_distance: f32,
    pub new_token_ratio: f32,
    pub local_graph_diversity: f32,
    pub domain_redundancy: f32,
    pub boilerplate_penalty: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FrontierEntry {
    pub url: String,
    pub discovered_from: Option<String>,
    pub anchor: Option<String>,
    pub depth: i64,
    pub energy: f32,
    pub status: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PageRecord {
    pub url: String,
    pub domain: String,
    pub title: String,
    pub summary: String,
    pub reason: String,
    pub food_energy: f32,
    pub depth: i64,
    pub content_hash: String,
    pub embedding: Option<Vec<f32>>,
    pub llm_embedding: Option<Vec<f32>>,
    pub llm_novelty: Option<LlmNovelty>,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug)]
pub struct PageEmbeddingBackfillRecord {
    pub url: String,
    pub title: String,
    pub summary: String,
    pub clean_text: String,
}

#[derive(Clone, Debug)]
pub struct LlmEmbeddingBackfillRecord {
    pub url: String,
    pub llm_novelty: LlmNovelty,
}

#[derive(Clone, Debug)]
pub struct AxisBackfillRecord {
    pub url: String,
    pub title: String,
    pub summary: String,
    pub llm_novelty: LlmNovelty,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GraphNode {
    pub url: String,
    pub title: String,
    pub domain: String,
    pub food_energy: f32,
    pub reason: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GraphEdge {
    pub source_url: String,
    pub target_url: String,
    pub anchor: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DashboardSnapshot {
    pub total_pages: i64,
    pub queued_pages: i64,
    pub failed_pages: i64,
    pub pages: Vec<PageRecord>,
    pub frontier: Vec<FrontierEntry>,
    pub graph_nodes: Vec<GraphNode>,
    pub graph_edges: Vec<GraphEdge>,
}
