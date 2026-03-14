use std::{path::PathBuf, sync::Arc};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tokio::{fs, sync::RwLock};

use crate::config::{Config, EditableSettings};

#[derive(Clone)]
pub struct SettingsManager {
    active: Arc<RwLock<Config>>,
    startup_bind_addr: String,
    startup_database_url: String,
    settings_path: Arc<PathBuf>,
}

impl SettingsManager {
    pub async fn new(initial: Config) -> Result<Self> {
        let settings_path = PathBuf::from(initial.settings_path.clone());
        let effective = if fs::try_exists(&settings_path)
            .await
            .context("failed to check settings file")?
        {
            let contents = fs::read_to_string(&settings_path)
                .await
                .with_context(|| format!("failed to read {}", settings_path.display()))?;
            let editable: PartialEditableSettings =
                serde_json::from_str(&contents).context("failed to parse settings JSON")?;
            let merged = editable.merge(initial.to_editable());
            initial.apply_editable(&merged)?
        } else {
            initial
        };

        Ok(Self {
            startup_bind_addr: effective.bind_addr.to_string(),
            startup_database_url: effective.database_url.clone(),
            active: Arc::new(RwLock::new(effective)),
            settings_path: Arc::new(settings_path),
        })
    }

    pub async fn current(&self) -> Config {
        self.active.read().await.clone()
    }

    pub async fn snapshot(&self) -> SettingsSnapshot {
        let config = self.active.read().await.clone();
        SettingsSnapshot {
            settings: config.to_editable(),
            pending_restart: self.pending_restart(&config),
            settings_path: self.settings_path.display().to_string(),
        }
    }

    pub async fn update(&self, editable: EditableSettings) -> Result<SettingsSnapshot> {
        let next = {
            let current = self.active.read().await.clone();
            current.apply_editable(&editable)?
        };
        self.persist(&next.to_editable()).await?;
        *self.active.write().await = next;
        Ok(self.snapshot().await)
    }

    fn pending_restart(&self, config: &Config) -> Vec<String> {
        let mut pending = Vec::new();
        if config.bind_addr.to_string() != self.startup_bind_addr {
            pending.push("bind_addr".to_string());
        }
        if config.database_url != self.startup_database_url {
            pending.push("database_url".to_string());
        }
        pending
    }

    async fn persist(&self, editable: &EditableSettings) -> Result<()> {
        if let Some(parent) = self.settings_path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)
                    .await
                    .with_context(|| format!("failed to create {}", parent.display()))?;
            }
        }

        let body =
            serde_json::to_string_pretty(editable).context("failed to serialize settings")?;
        fs::write(&*self.settings_path, body)
            .await
            .with_context(|| format!("failed to write {}", self.settings_path.display()))?;
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct SettingsSnapshot {
    pub settings: EditableSettings,
    pub pending_restart: Vec<String>,
    pub settings_path: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
struct PartialEditableSettings {
    bind_addr: Option<String>,
    database_url: Option<String>,
    seed_urls: Option<Vec<String>>,
    llm_base_url: Option<String>,
    llm_api_key: Option<String>,
    llm_model: Option<String>,
    embedding_model: Option<String>,
    page_semantic_map_enabled: Option<bool>,
    llm_semantic_map_enabled: Option<bool>,
    semantic_axes: Option<Vec<crate::models::SemanticAxis>>,
    request_timeout_secs: Option<u64>,
    llm_timeout_secs: Option<u64>,
    crawl_idle_ms: Option<u64>,
    max_links_per_page: Option<usize>,
    expand_threshold: Option<f32>,
    sample_threshold: Option<f32>,
}

impl PartialEditableSettings {
    fn merge(self, mut base: EditableSettings) -> EditableSettings {
        if let Some(value) = self.bind_addr {
            base.bind_addr = value;
        }
        if let Some(value) = self.database_url {
            base.database_url = value;
        }
        if let Some(value) = self.seed_urls {
            base.seed_urls = value;
        }
        if let Some(value) = self.llm_base_url {
            base.llm_base_url = value;
        }
        if let Some(value) = self.llm_api_key {
            base.llm_api_key = value;
        }
        if let Some(value) = self.llm_model {
            base.llm_model = value;
        }
        if self.embedding_model.is_some() {
            base.embedding_model = self.embedding_model;
        }
        if let Some(value) = self.page_semantic_map_enabled {
            base.page_semantic_map_enabled = value;
        }
        if let Some(value) = self.llm_semantic_map_enabled {
            base.llm_semantic_map_enabled = value;
        }
        if let Some(value) = self.semantic_axes {
            base.semantic_axes = value;
        }
        if let Some(value) = self.request_timeout_secs {
            base.request_timeout_secs = value;
        }
        if let Some(value) = self.llm_timeout_secs {
            base.llm_timeout_secs = value;
        }
        if let Some(value) = self.crawl_idle_ms {
            base.crawl_idle_ms = value;
        }
        if let Some(value) = self.max_links_per_page {
            base.max_links_per_page = value;
        }
        if let Some(value) = self.expand_threshold {
            base.expand_threshold = value;
        }
        if let Some(value) = self.sample_threshold {
            base.sample_threshold = value;
        }
        base
    }
}

#[cfg(test)]
mod tests {
    use super::PartialEditableSettings;
    use crate::config::EditableSettings;

    #[test]
    fn partial_settings_merge_preserves_new_defaults() {
        let partial: PartialEditableSettings =
            serde_json::from_str(r#"{"llm_model":"local-a","request_timeout_secs":20}"#)
                .expect("parse partial settings");

        let merged = partial.merge(EditableSettings {
            bind_addr: "127.0.0.1:3000".to_string(),
            database_url: "sqlite://hunger.db?mode=rwc".to_string(),
            seed_urls: vec!["https://example.com".to_string()],
            llm_base_url: "http://localhost:1234/v1".to_string(),
            llm_api_key: "lm-studio".to_string(),
            llm_model: "fallback".to_string(),
            embedding_model: None,
            page_semantic_map_enabled: true,
            llm_semantic_map_enabled: false,
            semantic_axes: vec![
                crate::models::SemanticAxis::default(),
                crate::models::SemanticAxis::default(),
                crate::models::SemanticAxis::default(),
            ],
            request_timeout_secs: 20,
            llm_timeout_secs: 75,
            crawl_idle_ms: 1500,
            max_links_per_page: 6,
            expand_threshold: 0.55,
            sample_threshold: 0.30,
        });

        assert_eq!(merged.llm_model, "local-a");
        assert_eq!(merged.llm_timeout_secs, 75);
    }
}
