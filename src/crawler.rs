use anyhow::{Context, Result};
use std::time::Duration;
use tracing::{debug, warn};

use crate::{
    control::RuntimeControl,
    db::Database,
    extractor::extract_page,
    llm::OpenAiCompatibleClient,
    models::{FrontierEntry, LlmNovelty, PageDigest},
    novelty::{combine_scores, compute_cheap_signals, select_links},
    settings::SettingsManager,
};

#[derive(Clone)]
pub struct Crawler {
    settings: SettingsManager,
    database: Database,
    control: RuntimeControl,
    http: reqwest::Client,
}

impl Crawler {
    pub fn new(
        settings: SettingsManager,
        database: Database,
        control: RuntimeControl,
    ) -> Result<Self> {
        let http = reqwest::Client::builder()
            .user_agent("Hunger/0.1 (+https://github.com/openai/codex)")
            .build()
            .context("failed to build crawler http client")?;

        Ok(Self {
            settings,
            database,
            control,
            http,
        })
    }

    pub async fn run_forever(self) {
        loop {
            if self.control.is_paused() {
                tokio::time::sleep(Duration::from_millis(180)).await;
                continue;
            }

            match self.tick().await {
                Ok(true) => {}
                Ok(false) => {
                    let config = self.settings.current().await;
                    tokio::time::sleep(config.crawl_idle_delay).await;
                }
                Err(error) => {
                    warn!(%error, "crawler tick failed");
                    let config = self.settings.current().await;
                    tokio::time::sleep(config.crawl_idle_delay).await;
                }
            }
        }
    }

    async fn tick(&self) -> Result<bool> {
        let Some(entry) = self.database.claim_frontier_item().await? else {
            return Ok(false);
        };

        if let Err(error) = self.process_frontier_item(&entry).await {
            warn!(url = %entry.url, %error, "frontier item failed");
            self.database
                .mark_frontier_failed(&entry.url, &error.to_string())
                .await?;
        }

        Ok(true)
    }

    async fn process_frontier_item(&self, entry: &FrontierEntry) -> Result<()> {
        let config = self.settings.current().await;
        let response = self
            .http
            .get(&entry.url)
            .timeout(config.request_timeout)
            .send()
            .await
            .with_context(|| format!("failed to fetch {}", entry.url))?;

        let status = response.status();
        if !status.is_success() {
            anyhow::bail!("fetch returned status {status}");
        }

        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_string();
        if !content_type.contains("text/html") {
            anyhow::bail!("unsupported content type {content_type}");
        }

        let html = response
            .text()
            .await
            .context("failed to read response body")?;
        let digest = extract_page(&entry.url, &html);
        if digest.clean_text.is_empty() {
            anyhow::bail!("extracted page had no usable text");
        }

        let llm = OpenAiCompatibleClient::new(&config)?;
        let embedding = if config.page_semantic_map_enabled {
            let embedding_input = build_embedding_input(&digest);
            match llm.embed_text(&embedding_input).await {
                Ok(value) => value,
                Err(error) => {
                    warn!(
                        url = %entry.url,
                        error = %format!("{error:#}"),
                        "page embedding request failed; falling back to lexical similarity"
                    );
                    None
                }
            }
        } else {
            None
        };

        let recent_pages = self.database.recent_pages(24).await?;
        let same_domain_pages = self.database.same_domain_pages(&digest.domain, 12).await?;
        let cheap_signals = compute_cheap_signals(
            &digest,
            embedding.as_deref(),
            &recent_pages,
            &same_domain_pages,
        );

        let llm_judgment = if should_query_model(&cheap_signals) {
            match llm
                .score_novelty(&digest, &recent_pages, &same_domain_pages, &cheap_signals)
                .await
            {
                Ok(judgment) => Some(judgment),
                Err(error) => {
                    warn!(
                        url = %entry.url,
                        error = %format!("{error:#}"),
                        "novelty scoring request failed; falling back to heuristics"
                    );
                    None
                }
            }
        } else {
            None
        };

        let llm_embedding = if config.llm_semantic_map_enabled {
            if let Some(judgment) = llm_judgment.as_ref() {
                let llm_embedding_input = build_llm_embedding_input(judgment);
                if llm_embedding_input.trim().is_empty() {
                    None
                } else {
                    match llm.embed_text(&llm_embedding_input).await {
                        Ok(value) => value,
                        Err(error) => {
                            warn!(
                                url = %entry.url,
                                error = %format!("{error:#}"),
                                "llm-response embedding request failed; skipping semantic judgment clustering"
                            );
                            None
                        }
                    }
                }
            } else {
                None
            }
        } else {
            None
        };

        let score = combine_scores(&config, &cheap_signals, llm_judgment.clone());
        let selected_links = select_links(
            &digest,
            &score,
            llm_judgment.as_ref(),
            config.max_links_per_page,
        );

        debug!(
            url = %entry.url,
            energy = score.food_energy,
            selected_links = selected_links.len(),
            reason = %score.concise_reason,
            "stored page"
        );

        self.database
            .store_crawl_result(
                entry,
                &digest,
                embedding.as_deref(),
                llm_embedding.as_deref(),
                llm_judgment.as_ref(),
                &score,
                &selected_links,
            )
            .await?;

        Ok(())
    }
}

fn build_embedding_input(digest: &PageDigest) -> String {
    let mut sections = vec![digest.title.clone(), digest.short_summary.clone()];
    sections.extend(digest.key_points.iter().take(4).cloned());
    sections.extend(digest.claims.iter().take(4).cloned());
    sections.join("\n")
}

fn build_llm_embedding_input(judgment: &LlmNovelty) -> String {
    let mut sections = Vec::new();
    if !judgment.analysis.trim().is_empty() {
        sections.push(judgment.analysis.clone());
    }
    if !judgment.concise_reason.trim().is_empty() {
        sections.push(judgment.concise_reason.clone());
    }
    if !judgment.likely_novel_aspects.is_empty() {
        sections.push(format!(
            "novel: {}",
            judgment.likely_novel_aspects.join(" | ")
        ));
    }
    if !judgment.likely_redundant_aspects.is_empty() {
        sections.push(format!(
            "redundant: {}",
            judgment.likely_redundant_aspects.join(" | ")
        ));
    }
    sections.push(format!("action: {}", judgment.recommended_action));
    sections.join("\n")
}

fn should_query_model(signals: &crate::models::CheapSignals) -> bool {
    signals.semantic_distance > 0.10
        || signals.new_token_ratio > 0.12
        || signals.local_graph_diversity > 0.25
        || signals.content_delta_from_last_snapshot > 0.5
}
