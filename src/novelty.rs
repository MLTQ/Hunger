use std::collections::HashSet;

use crate::{
    config::Config,
    models::{CheapSignals, LlmNovelty, NoveltyScore, OutboundLink, PageDigest, PageRecord},
};

pub fn compute_cheap_signals(
    candidate: &PageDigest,
    candidate_embedding: Option<&[f32]>,
    recent_pages: &[PageRecord],
    same_domain_pages: &[PageRecord],
) -> CheapSignals {
    let candidate_tokens = token_set(&candidate.clean_text);
    let max_similarity = recent_pages
        .iter()
        .map(|page| compare_similarity(&candidate_tokens, candidate_embedding, page))
        .fold(0.0_f32, f32::max);

    let seen_tokens = recent_pages
        .iter()
        .flat_map(|page| token_set(&page.summary))
        .collect::<HashSet<_>>();
    let unseen_tokens = candidate_tokens
        .iter()
        .filter(|token| !seen_tokens.contains(*token))
        .count();
    let new_token_ratio = if candidate_tokens.is_empty() {
        0.0
    } else {
        unseen_tokens as f32 / candidate_tokens.len() as f32
    };

    let domain_redundancy = if same_domain_pages.is_empty() {
        0.0
    } else {
        same_domain_pages
            .iter()
            .map(|page| compare_similarity(&candidate_tokens, candidate_embedding, page))
            .sum::<f32>()
            / same_domain_pages.len() as f32
    };

    let local_graph_diversity = graph_diversity(candidate, recent_pages);
    let content_delta_from_last_snapshot = recent_pages
        .iter()
        .find(|page| page.url == candidate.url)
        .map(|page| {
            if page.content_hash == candidate.content_hash {
                0.0
            } else {
                1.0
            }
        })
        .unwrap_or(1.0);

    CheapSignals {
        semantic_distance: (1.0 - max_similarity).clamp(0.0, 1.0),
        new_token_ratio,
        domain_redundancy: domain_redundancy.clamp(0.0, 1.0),
        local_graph_diversity,
        content_delta_from_last_snapshot,
        boilerplate_penalty: candidate.boilerplate_ratio,
    }
}

pub fn combine_scores(
    config: &Config,
    cheap: &CheapSignals,
    llm: Option<LlmNovelty>,
) -> NoveltyScore {
    let llm = llm.unwrap_or_else(|| fallback_novelty(cheap, config));
    let food_energy = (0.30 * llm.factual_novelty)
        + (0.15 * llm.entity_novelty)
        + (0.20 * llm.relational_novelty)
        + (0.10 * llm.temporal_novelty)
        + (0.25 * llm.expected_crawl_value)
        + (0.10 * cheap.semantic_distance)
        + (0.10 * cheap.new_token_ratio)
        + (0.10 * cheap.local_graph_diversity)
        - (0.15 * cheap.boilerplate_penalty)
        - (0.10 * cheap.domain_redundancy);

    NoveltyScore {
        food_energy: food_energy.clamp(0.0, 1.0),
        factual_novelty: llm.factual_novelty,
        entity_novelty: llm.entity_novelty,
        relational_novelty: llm.relational_novelty,
        temporal_novelty: llm.temporal_novelty,
        expected_crawl_value: llm.expected_crawl_value,
        concise_reason: llm.concise_reason,
        likely_novel_aspects: llm.likely_novel_aspects,
        likely_redundant_aspects: llm.likely_redundant_aspects,
        semantic_distance: cheap.semantic_distance,
        new_token_ratio: cheap.new_token_ratio,
        local_graph_diversity: cheap.local_graph_diversity,
        domain_redundancy: cheap.domain_redundancy,
        boilerplate_penalty: cheap.boilerplate_penalty,
    }
}

pub fn select_links(
    candidate: &PageDigest,
    score: &NoveltyScore,
    llm: Option<&LlmNovelty>,
    max_links_per_page: usize,
) -> Vec<OutboundLink> {
    if candidate.outbound_links.is_empty() {
        return Vec::new();
    }

    let expansion_budget = if score.food_energy >= 0.75 {
        max_links_per_page
    } else if score.food_energy >= 0.45 {
        max_links_per_page.min(2)
    } else if score.food_energy >= 0.25 {
        1
    } else {
        0
    };

    if expansion_budget == 0 {
        return Vec::new();
    }

    let mut links = candidate.outbound_links.clone();
    links.sort_by(|left, right| {
        let right_score = link_score(candidate, right, llm);
        let left_score = link_score(candidate, left, llm);
        right_score.total_cmp(&left_score)
    });
    links.truncate(expansion_budget);
    links
}

fn fallback_novelty(cheap: &CheapSignals, config: &Config) -> LlmNovelty {
    let expected_crawl_value =
        (cheap.semantic_distance + cheap.new_token_ratio + cheap.local_graph_diversity) / 3.0;
    let recommended_action = if expected_crawl_value >= config.expand_threshold {
        "expand"
    } else if expected_crawl_value >= config.sample_threshold {
        "sample"
    } else {
        "ignore"
    };

    LlmNovelty {
        factual_novelty: cheap.semantic_distance,
        entity_novelty: cheap.new_token_ratio,
        relational_novelty: cheap.local_graph_diversity,
        temporal_novelty: cheap.content_delta_from_last_snapshot,
        expected_crawl_value,
        analysis: String::new(),
        concise_reason:
            "Heuristic fallback based on semantic distance, token novelty, and graph diversity."
                .to_string(),
        likely_novel_aspects: Vec::new(),
        likely_redundant_aspects: Vec::new(),
        recommended_action: recommended_action.to_string(),
        recommended_link_hints: Vec::new(),
        axis_scores: Vec::new(),
    }
}

fn graph_diversity(candidate: &PageDigest, recent_pages: &[PageRecord]) -> f32 {
    let recent_domains = recent_pages
        .iter()
        .map(|page| page.domain.as_str())
        .collect::<HashSet<_>>();

    let candidate_domains = candidate
        .outbound_links
        .iter()
        .filter_map(|link| url::Url::parse(&link.url).ok())
        .filter_map(|url| url.domain().map(ToOwned::to_owned))
        .collect::<HashSet<_>>();

    if candidate_domains.is_empty() {
        return 0.0;
    }

    let new_domains = candidate_domains
        .iter()
        .filter(|domain| !recent_domains.contains(domain.as_str()))
        .count();
    new_domains as f32 / candidate_domains.len() as f32
}

fn compare_similarity(
    candidate_tokens: &HashSet<String>,
    candidate_embedding: Option<&[f32]>,
    page: &PageRecord,
) -> f32 {
    if let (Some(left), Some(right)) = (candidate_embedding, page.embedding.as_deref()) {
        return cosine_similarity(left, right);
    }

    let page_tokens = token_set(&page.summary);
    jaccard_similarity(candidate_tokens, &page_tokens)
}

fn token_set(text: &str) -> HashSet<String> {
    text.split(|character: char| !character.is_alphanumeric())
        .map(str::trim)
        .filter(|token| token.len() >= 4)
        .map(|token| token.to_ascii_lowercase())
        .collect()
}

fn jaccard_similarity(left: &HashSet<String>, right: &HashSet<String>) -> f32 {
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }

    let intersection = left.intersection(right).count() as f32;
    let union = left.union(right).count() as f32;
    if union == 0.0 {
        0.0
    } else {
        intersection / union
    }
}

fn cosine_similarity(left: &[f32], right: &[f32]) -> f32 {
    if left.is_empty() || right.is_empty() || left.len() != right.len() {
        return 0.0;
    }

    let mut dot = 0.0;
    let mut left_norm = 0.0;
    let mut right_norm = 0.0;
    for (l, r) in left.iter().zip(right.iter()) {
        dot += l * r;
        left_norm += l * l;
        right_norm += r * r;
    }

    if left_norm == 0.0 || right_norm == 0.0 {
        0.0
    } else {
        (dot / (left_norm.sqrt() * right_norm.sqrt()))
            .clamp(-1.0, 1.0)
            .max(0.0)
    }
}

fn link_score(candidate: &PageDigest, link: &OutboundLink, llm: Option<&LlmNovelty>) -> f32 {
    let llm_hint_priority = llm
        .and_then(|judgment| {
            judgment
                .recommended_link_hints
                .iter()
                .find(|hint| hint.link_url == link.url)
                .map(|hint| hint.priority)
        })
        .unwrap_or(0.0);

    let anchor_semantic_promise = if link.anchor.split_whitespace().count() >= 2 {
        0.7
    } else if !link.anchor.is_empty() {
        0.4
    } else {
        0.1
    };

    let destination_domain_bonus = url::Url::parse(&link.url)
        .ok()
        .and_then(|url| url.domain().map(ToOwned::to_owned))
        .map(|domain| if domain == candidate.domain { 0.9 } else { 0.5 })
        .unwrap_or(0.1);

    let spam_pattern_penalty = if link.url.contains("login") || link.url.contains("signup") {
        0.5
    } else {
        0.0
    };

    (0.45 * llm_hint_priority)
        + (0.25 * anchor_semantic_promise)
        + (0.20 * destination_domain_bonus)
        - (0.10 * spam_pattern_penalty)
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::compute_cheap_signals;
    use crate::models::{OutboundLink, PageDigest, PageRecord};

    #[test]
    fn cheap_signals_detect_new_tokens() {
        let digest = PageDigest {
            url: "https://example.com/new".to_string(),
            domain: "example.com".to_string(),
            title: "Novel".to_string(),
            clean_text: "alpha beta gamma delta epsilon zeta".to_string(),
            short_summary: "alpha beta gamma delta epsilon zeta".to_string(),
            key_points: Vec::new(),
            entities: Vec::new(),
            claims: Vec::new(),
            outbound_links: vec![OutboundLink {
                url: "https://example.com/next".to_string(),
                anchor: "next research note".to_string(),
            }],
            boilerplate_ratio: 0.2,
            content_hash: "hash".to_string(),
        };
        let recent = vec![PageRecord {
            url: "https://example.com/old".to_string(),
            domain: "example.com".to_string(),
            title: "Old".to_string(),
            summary: "alpha beta gamma".to_string(),
            reason: String::new(),
            food_energy: 0.4,
            depth: 0,
            content_hash: "older".to_string(),
            embedding: None,
            llm_embedding: None,
            llm_novelty: None,
            created_at: Utc::now(),
        }];

        let signals = compute_cheap_signals(&digest, None, &recent, &recent);
        assert!(signals.new_token_ratio > 0.0);
        assert!(signals.semantic_distance > 0.0);
    }
}
