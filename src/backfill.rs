use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use anyhow::{Context, Result, anyhow};
use tokio::{sync::RwLock, task::JoinSet};

use crate::{
    control::RuntimeControl,
    db::Database,
    llm::OpenAiCompatibleClient,
    models::{LlmEmbeddingBackfillRecord, LlmNovelty, PageEmbeddingBackfillRecord, SemanticAxis},
    settings::SettingsManager,
};

const BACKFILL_THROTTLE_MS: u64 = 120;
const BACKFILL_PARALLELISM: usize = 4;
const AXIS_SCAN_BATCH: i64 = 96;

#[derive(Clone, Debug, Default)]
pub struct BackfillStatus {
    pub running: bool,
    pub phase: String,
    pub current_url: Option<String>,
    pub processed: usize,
    pub total: usize,
    pub page_processed: usize,
    pub llm_processed: usize,
    pub axis_processed: usize,
    pub last_error: Option<String>,
}

#[derive(Clone, Default)]
pub struct BackfillController {
    active: Arc<AtomicBool>,
    status: Arc<RwLock<BackfillStatus>>,
}

impl BackfillController {
    pub fn start(
        &self,
        runtime: &Arc<tokio::runtime::Runtime>,
        database: Database,
        settings: SettingsManager,
        control: RuntimeControl,
    ) -> bool {
        if self.active.swap(true, Ordering::SeqCst) {
            return false;
        }

        let resume_after = !control.is_paused();
        if resume_after {
            control.pause();
        }

        let controller = self.clone();
        runtime.spawn(async move {
            controller.reset_running_status().await;
            let result = controller.run(database, settings).await;
            controller.finish(result, control, resume_after).await;
        });
        true
    }

    pub async fn snapshot(&self) -> BackfillStatus {
        self.status.read().await.clone()
    }

    async fn reset_running_status(&self) {
        *self.status.write().await = BackfillStatus {
            running: true,
            phase: "initializing".to_string(),
            ..Default::default()
        };
    }

    async fn finish(&self, result: Result<()>, control: RuntimeControl, resume_after: bool) {
        let mut status = self.status.write().await;
        status.running = false;
        match result {
            Ok(()) => {
                if status.total == 0 {
                    status.phase = "nothing to backfill".to_string();
                } else {
                    status.phase = "backfill complete".to_string();
                }
                status.last_error = None;
            }
            Err(error) => {
                status.phase = "backfill failed".to_string();
                status.last_error = Some(format!("{error:#}"));
            }
        }
        if resume_after {
            control.resume();
        }
        self.active.store(false, Ordering::SeqCst);
    }

    async fn run(&self, database: Database, settings: SettingsManager) -> Result<()> {
        let config = settings.current().await;
        let semantic_axes = config
            .semantic_axes
            .iter()
            .filter(|axis| axis.is_configured())
            .take(3)
            .cloned()
            .collect::<Vec<_>>();
        let semantic_backfill_enabled =
            config.page_semantic_map_enabled || config.llm_semantic_map_enabled;
        let axis_backfill_enabled = !semantic_axes.is_empty();

        if !semantic_backfill_enabled && !axis_backfill_enabled {
            return Err(anyhow!(
                "enable page embeddings, llm-response embeddings, or configure semantic axes first"
            ));
        }

        if semantic_backfill_enabled && config.embedding_model.is_none() {
            return Err(anyhow!("embedding model is not configured"));
        }

        let llm = OpenAiCompatibleClient::new(&config)?;
        let page_total = if config.page_semantic_map_enabled {
            database.count_missing_page_embeddings().await? as usize
        } else {
            0
        };
        let llm_total = if config.llm_semantic_map_enabled {
            database.count_missing_llm_embeddings().await? as usize
        } else {
            0
        };
        {
            let mut status = self.status.write().await;
            status.total = page_total + llm_total;
            status.phase = "counted backlog".to_string();
        }

        if config.page_semantic_map_enabled {
            loop {
                let batch = database
                    .page_embedding_backfill_batch(BACKFILL_PARALLELISM as i64)
                    .await?;
                if batch.is_empty() {
                    break;
                }

                self.update_cursor("page embeddings", &batch[0].url).await;
                let mut jobs = JoinSet::new();
                for page in batch {
                    let database = database.clone();
                    let llm = llm.clone();
                    jobs.spawn(async move {
                        let input = build_page_embedding_input(&page);
                        if let Some(embedding) = llm.embed_text(&input).await? {
                            database
                                .update_page_embedding(&page.url, &embedding)
                                .await?;
                        }
                        Ok::<String, anyhow::Error>(page.url)
                    });
                }

                while let Some(result) = jobs.join_next().await {
                    let url = result.context("page embedding task join failed")??;
                    self.update_cursor("page embeddings", &url).await;
                    self.increment_page().await;
                    tokio::time::sleep(Duration::from_millis(BACKFILL_THROTTLE_MS)).await;
                }
            }
        }

        if config.llm_semantic_map_enabled {
            loop {
                let batch = database
                    .llm_embedding_backfill_batch(BACKFILL_PARALLELISM as i64)
                    .await?;
                if batch.is_empty() {
                    break;
                }

                self.update_cursor("llm-response embeddings", &batch[0].url)
                    .await;
                let mut jobs = JoinSet::new();
                for page in batch {
                    let database = database.clone();
                    let llm = llm.clone();
                    jobs.spawn(async move {
                        let input = build_llm_embedding_input(&page);
                        if !input.trim().is_empty() {
                            if let Some(embedding) = llm.embed_text(&input).await? {
                                database.update_llm_embedding(&page.url, &embedding).await?;
                            }
                        }
                        Ok::<String, anyhow::Error>(page.url)
                    });
                }

                while let Some(result) = jobs.join_next().await {
                    let url = result.context("llm embedding task join failed")??;
                    self.update_cursor("llm-response embeddings", &url).await;
                    self.increment_llm().await;
                    tokio::time::sleep(Duration::from_millis(BACKFILL_THROTTLE_MS)).await;
                }
            }
        }

        if axis_backfill_enabled {
            let mut offset = 0i64;
            loop {
                let batch = database
                    .axis_backfill_batch(AXIS_SCAN_BATCH, offset)
                    .await?;
                if batch.is_empty() {
                    break;
                }
                offset += batch.len() as i64;

                let pending = batch
                    .into_iter()
                    .filter(|record| needs_axis_rescore(&record.llm_novelty, &semantic_axes))
                    .collect::<Vec<_>>();
                if pending.is_empty() {
                    tokio::task::yield_now().await;
                    continue;
                }

                {
                    let mut status = self.status.write().await;
                    status.total += pending.len();
                }

                let mut pending_iter = pending.into_iter();
                loop {
                    let mut jobs = JoinSet::new();
                    for _ in 0..BACKFILL_PARALLELISM {
                        let Some(page) = pending_iter.next() else {
                            break;
                        };
                        let database = database.clone();
                        let llm = llm.clone();
                        jobs.spawn(async move {
                            let axis_scores = llm
                                .score_axes_for_page(
                                    &page.url,
                                    &page.title,
                                    &page.summary,
                                    &page.llm_novelty,
                                )
                                .await?;
                            let updated_novelty = LlmNovelty {
                                axis_scores,
                                ..page.llm_novelty
                            };
                            database
                                .update_llm_novelty(&page.url, &updated_novelty)
                                .await?;
                            Ok::<String, anyhow::Error>(page.url)
                        });
                    }

                    if jobs.is_empty() {
                        break;
                    }

                    while let Some(result) = jobs.join_next().await {
                        let url = result.context("axis scoring task join failed")??;
                        self.update_cursor("axis scoring", &url).await;
                        self.increment_axis().await;
                        tokio::time::sleep(Duration::from_millis(BACKFILL_THROTTLE_MS)).await;
                    }
                }
            }
        }

        Ok(())
    }

    async fn update_cursor(&self, phase: &str, url: &str) {
        let mut status = self.status.write().await;
        status.phase = phase.to_string();
        status.current_url = Some(url.to_string());
    }

    async fn increment_page(&self) {
        let mut status = self.status.write().await;
        status.processed += 1;
        status.page_processed += 1;
    }

    async fn increment_llm(&self) {
        let mut status = self.status.write().await;
        status.processed += 1;
        status.llm_processed += 1;
    }

    async fn increment_axis(&self) {
        let mut status = self.status.write().await;
        status.processed += 1;
        status.axis_processed += 1;
    }
}

fn build_page_embedding_input(page: &PageEmbeddingBackfillRecord) -> String {
    let clean_excerpt = page
        .clean_text
        .split_whitespace()
        .take(160)
        .collect::<Vec<_>>()
        .join(" ");
    [page.title.clone(), page.summary.clone(), clean_excerpt]
        .into_iter()
        .filter(|value| !value.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn build_llm_embedding_input(page: &LlmEmbeddingBackfillRecord) -> String {
    build_llm_embedding_from_novelty(&page.llm_novelty)
}

fn build_llm_embedding_from_novelty(judgment: &LlmNovelty) -> String {
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

fn needs_axis_rescore(judgment: &LlmNovelty, semantic_axes: &[SemanticAxis]) -> bool {
    if semantic_axes.is_empty() {
        return false;
    }

    if judgment.axis_scores.len() != semantic_axes.len() {
        return true;
    }

    semantic_axes.iter().any(|axis| {
        !judgment.axis_scores.iter().any(|score| {
            score.negative_label.trim() == axis.negative_label.trim()
                && score.positive_label.trim() == axis.positive_label.trim()
        })
    })
}

#[cfg(test)]
mod tests {
    use super::{build_llm_embedding_from_novelty, build_page_embedding_input, needs_axis_rescore};
    use crate::models::{AxisScore, LlmNovelty, PageEmbeddingBackfillRecord, SemanticAxis};

    #[test]
    fn llm_embedding_input_captures_reasoning() {
        let novelty = LlmNovelty {
            analysis: "Detailed reasoning".to_string(),
            concise_reason: "Short reason".to_string(),
            likely_novel_aspects: vec!["A".to_string()],
            likely_redundant_aspects: vec!["B".to_string()],
            recommended_action: "expand".to_string(),
            ..LlmNovelty::fallback()
        };
        let input = build_llm_embedding_from_novelty(&novelty);
        assert!(input.contains("Detailed reasoning"));
        assert!(input.contains("action: expand"));
    }

    #[test]
    fn page_embedding_input_uses_clean_excerpt() {
        let page = PageEmbeddingBackfillRecord {
            url: "https://example.com".to_string(),
            title: "Title".to_string(),
            summary: "Summary".to_string(),
            clean_text: "alpha beta gamma".to_string(),
        };
        let input = build_page_embedding_input(&page);
        assert!(input.contains("Title"));
        assert!(input.contains("alpha beta gamma"));
    }

    #[test]
    fn detects_missing_axis_scores_for_backfill() {
        let axes = vec![SemanticAxis {
            negative_label: "Isolationist".to_string(),
            positive_label: "Expansionist".to_string(),
        }];
        assert!(needs_axis_rescore(&LlmNovelty::fallback(), &axes));
    }

    #[test]
    fn skips_axis_backfill_when_scores_match_configured_axes() {
        let axes = vec![SemanticAxis {
            negative_label: "Isolationist".to_string(),
            positive_label: "Expansionist".to_string(),
        }];
        let novelty = LlmNovelty {
            axis_scores: vec![AxisScore {
                negative_label: "Isolationist".to_string(),
                positive_label: "Expansionist".to_string(),
                score: 0.2,
                explanation: "test".to_string(),
            }],
            ..LlmNovelty::fallback()
        };
        assert!(!needs_axis_rescore(&novelty, &axes));
    }
}
