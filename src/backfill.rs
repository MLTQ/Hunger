use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use anyhow::{Result, anyhow};
use tokio::sync::RwLock;

use crate::{
    db::Database,
    llm::OpenAiCompatibleClient,
    models::{LlmEmbeddingBackfillRecord, LlmNovelty, PageEmbeddingBackfillRecord},
    settings::SettingsManager,
};

#[derive(Clone, Debug, Default)]
pub struct BackfillStatus {
    pub running: bool,
    pub phase: String,
    pub current_url: Option<String>,
    pub processed: usize,
    pub total: usize,
    pub page_processed: usize,
    pub llm_processed: usize,
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
    ) -> bool {
        if self.active.swap(true, Ordering::SeqCst) {
            return false;
        }

        let controller = self.clone();
        runtime.spawn(async move {
            controller.reset_running_status().await;
            let result = controller.run(database, settings).await;
            controller.finish(result).await;
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

    async fn finish(&self, result: Result<()>) {
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
        self.active.store(false, Ordering::SeqCst);
    }

    async fn run(&self, database: Database, settings: SettingsManager) -> Result<()> {
        let config = settings.current().await;
        if config.embedding_model.is_none() {
            return Err(anyhow!("embedding model is not configured"));
        }
        if !config.page_semantic_map_enabled && !config.llm_semantic_map_enabled {
            return Err(anyhow!(
                "enable page semantic embeddings and/or llm-response embeddings first"
            ));
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
                let batch = database.page_embedding_backfill_batch(12).await?;
                if batch.is_empty() {
                    break;
                }

                for page in batch {
                    self.update_cursor("page embeddings", &page.url).await;
                    let input = build_page_embedding_input(&page);
                    if let Some(embedding) = llm.embed_text(&input).await? {
                        database
                            .update_page_embedding(&page.url, &embedding)
                            .await?;
                    }
                    self.increment_page().await;
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            }
        }

        if config.llm_semantic_map_enabled {
            loop {
                let batch = database.llm_embedding_backfill_batch(12).await?;
                if batch.is_empty() {
                    break;
                }

                for page in batch {
                    self.update_cursor("llm-response embeddings", &page.url)
                        .await;
                    let input = build_llm_embedding_input(&page);
                    if input.trim().is_empty() {
                        self.increment_llm().await;
                        continue;
                    }
                    if let Some(embedding) = llm.embed_text(&input).await? {
                        database.update_llm_embedding(&page.url, &embedding).await?;
                    }
                    self.increment_llm().await;
                    tokio::time::sleep(Duration::from_millis(20)).await;
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

#[cfg(test)]
mod tests {
    use super::{build_llm_embedding_from_novelty, build_page_embedding_input};
    use crate::models::{LlmNovelty, PageEmbeddingBackfillRecord};

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
}
