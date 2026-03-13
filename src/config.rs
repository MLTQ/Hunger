use std::{
    env,
    net::SocketAddr,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug)]
pub struct Config {
    pub bind_addr: SocketAddr,
    pub database_url: String,
    pub settings_path: String,
    pub seed_urls: Vec<String>,
    pub llm_base_url: String,
    pub llm_api_key: String,
    pub llm_model: String,
    pub embedding_model: Option<String>,
    pub page_semantic_map_enabled: bool,
    pub llm_semantic_map_enabled: bool,
    pub request_timeout: Duration,
    pub llm_timeout: Duration,
    pub crawl_idle_delay: Duration,
    pub max_links_per_page: usize,
    pub expand_threshold: f32,
    pub sample_threshold: f32,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let executable_dir = executable_dir()?;
        let bind_addr = env::var("HUNGER_BIND_ADDR")
            .unwrap_or_else(|_| "127.0.0.1:3000".to_string())
            .parse()
            .context("failed to parse HUNGER_BIND_ADDR")?;

        let database_url = env::var("HUNGER_DATABASE_URL")
            .map(|value| resolve_database_url(&value, &executable_dir))
            .unwrap_or_else(|_| default_database_url(&executable_dir));
        let settings_path = env::var("HUNGER_SETTINGS_PATH")
            .map(|value| resolve_settings_path(&value, &executable_dir))
            .unwrap_or_else(|_| default_settings_path(&executable_dir));

        let seed_urls = env::var("HUNGER_SEEDS")
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .collect();

        let llm_base_url = env::var("HUNGER_LLM_BASE_URL")
            .map(|value| normalize_llm_base_url(&value))
            .unwrap_or_else(|_| "http://localhost:1234/v1".to_string());
        let llm_api_key =
            env::var("HUNGER_LLM_API_KEY").unwrap_or_else(|_| "lm-studio".to_string());
        let llm_model = env::var("HUNGER_LLM_MODEL").unwrap_or_else(|_| "local-model".to_string());
        let embedding_model = env::var("HUNGER_EMBEDDING_MODEL")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        let page_semantic_map_enabled = read_bool("HUNGER_PAGE_SEMANTIC_MAP", true);
        let llm_semantic_map_enabled = read_bool("HUNGER_LLM_SEMANTIC_MAP", false);

        let request_timeout = Duration::from_secs(read_u64("HUNGER_REQUEST_TIMEOUT_SECS", 20));
        let llm_timeout = Duration::from_secs(read_u64("HUNGER_LLM_TIMEOUT_SECS", 75));
        let crawl_idle_delay = Duration::from_millis(read_u64("HUNGER_CRAWL_IDLE_MS", 1500));
        let max_links_per_page = read_usize("HUNGER_MAX_LINKS_PER_PAGE", 6);
        let expand_threshold = read_f32("HUNGER_EXPAND_THRESHOLD", 0.55);
        let sample_threshold = read_f32("HUNGER_SAMPLE_THRESHOLD", 0.30);

        Ok(Self {
            bind_addr,
            database_url,
            settings_path,
            seed_urls,
            llm_base_url,
            llm_api_key,
            llm_model,
            embedding_model,
            page_semantic_map_enabled,
            llm_semantic_map_enabled,
            request_timeout,
            llm_timeout,
            crawl_idle_delay,
            max_links_per_page,
            expand_threshold,
            sample_threshold,
        })
    }

    pub fn to_editable(&self) -> EditableSettings {
        EditableSettings {
            bind_addr: self.bind_addr.to_string(),
            database_url: self.database_url.clone(),
            seed_urls: self.seed_urls.clone(),
            llm_base_url: self.llm_base_url.clone(),
            llm_api_key: self.llm_api_key.clone(),
            llm_model: self.llm_model.clone(),
            embedding_model: self.embedding_model.clone(),
            page_semantic_map_enabled: self.page_semantic_map_enabled,
            llm_semantic_map_enabled: self.llm_semantic_map_enabled,
            request_timeout_secs: self.request_timeout.as_secs(),
            llm_timeout_secs: self.llm_timeout.as_secs(),
            crawl_idle_ms: self.crawl_idle_delay.as_millis() as u64,
            max_links_per_page: self.max_links_per_page,
            expand_threshold: self.expand_threshold,
            sample_threshold: self.sample_threshold,
        }
    }

    pub fn apply_editable(&self, editable: &EditableSettings) -> Result<Self> {
        let bind_addr = editable
            .bind_addr
            .trim()
            .parse()
            .context("failed to parse bind_addr")?;

        Ok(Self {
            bind_addr,
            database_url: resolve_database_url(&editable.database_url, &executable_dir()?),
            settings_path: self.settings_path.clone(),
            seed_urls: editable
                .seed_urls
                .iter()
                .map(|value| value.trim())
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
                .collect(),
            llm_base_url: normalize_llm_base_url(&editable.llm_base_url),
            llm_api_key: editable.llm_api_key.clone(),
            llm_model: editable.llm_model.trim().to_string(),
            embedding_model: editable
                .embedding_model
                .as_ref()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty()),
            page_semantic_map_enabled: editable.page_semantic_map_enabled,
            llm_semantic_map_enabled: editable.llm_semantic_map_enabled,
            request_timeout: Duration::from_secs(editable.request_timeout_secs.max(1)),
            llm_timeout: Duration::from_secs(editable.llm_timeout_secs.max(1)),
            crawl_idle_delay: Duration::from_millis(editable.crawl_idle_ms.max(100)),
            max_links_per_page: editable.max_links_per_page.max(1),
            expand_threshold: editable.expand_threshold.clamp(0.0, 1.0),
            sample_threshold: editable.sample_threshold.clamp(0.0, 1.0),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EditableSettings {
    pub bind_addr: String,
    pub database_url: String,
    pub seed_urls: Vec<String>,
    pub llm_base_url: String,
    pub llm_api_key: String,
    pub llm_model: String,
    pub embedding_model: Option<String>,
    pub page_semantic_map_enabled: bool,
    pub llm_semantic_map_enabled: bool,
    pub request_timeout_secs: u64,
    pub llm_timeout_secs: u64,
    pub crawl_idle_ms: u64,
    pub max_links_per_page: usize,
    pub expand_threshold: f32,
    pub sample_threshold: f32,
}

fn read_u64(name: &str, default: u64) -> u64 {
    env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

fn read_usize(name: &str, default: usize) -> usize {
    env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

fn read_f32(name: &str, default: f32) -> f32 {
    env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

fn read_bool(name: &str, default: bool) -> bool {
    env::var(name)
        .ok()
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(default)
}

fn normalize_llm_base_url(value: &str) -> String {
    let trimmed = value.trim().trim_end_matches('/');
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        trimmed.to_string()
    } else {
        format!("http://{trimmed}")
    }
}

fn executable_dir() -> Result<PathBuf> {
    let executable = env::current_exe().context("failed to resolve current executable path")?;
    executable
        .parent()
        .map(Path::to_path_buf)
        .context("current executable had no parent directory")
}

fn default_database_url(executable_dir: &Path) -> String {
    sqlite_url_for_path(&executable_dir.join("hunger.db"), Some("mode=rwc"))
}

fn default_settings_path(executable_dir: &Path) -> String {
    executable_dir
        .join("hunger.settings.json")
        .to_string_lossy()
        .into_owned()
}

fn resolve_settings_path(value: &str, executable_dir: &Path) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return default_settings_path(executable_dir);
    }

    let path = Path::new(trimmed);
    if path.is_absolute() {
        path.to_string_lossy().into_owned()
    } else {
        executable_dir.join(path).to_string_lossy().into_owned()
    }
}

fn resolve_database_url(value: &str, executable_dir: &Path) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return default_database_url(executable_dir);
    }

    if let Some(body) = trimmed.strip_prefix("sqlite://") {
        return resolve_sqlite_body(body, executable_dir);
    }

    if trimmed.starts_with("postgres://")
        || trimmed.starts_with("postgresql://")
        || trimmed.starts_with("mysql://")
    {
        return trimmed.to_string();
    }

    let path = Path::new(trimmed);
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        executable_dir.join(path)
    };
    sqlite_url_for_path(&absolute, Some("mode=rwc"))
}

fn resolve_sqlite_body(body: &str, executable_dir: &Path) -> String {
    let (path_part, query_part) = match body.split_once('?') {
        Some((path, query)) => (path, Some(query)),
        None => (body, None),
    };

    let path = Path::new(path_part);
    if path.is_absolute() {
        return sqlite_url_for_path(path, query_part);
    }

    sqlite_url_for_path(&executable_dir.join(path), query_part)
}

fn sqlite_url_for_path(path: &Path, query: Option<&str>) -> String {
    let mut url = format!("sqlite://{}", path.to_string_lossy());
    if let Some(query) = query.filter(|value| !value.is_empty()) {
        url.push('?');
        url.push_str(query);
    }
    url
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{
        default_database_url, normalize_llm_base_url, resolve_database_url, resolve_settings_path,
    };

    #[test]
    fn prefixes_http_when_scheme_is_missing() {
        assert_eq!(
            normalize_llm_base_url("192.168.0.203:1234/v1"),
            "http://192.168.0.203:1234/v1"
        );
    }

    #[test]
    fn sqlite_defaults_use_executable_directory() {
        let dir = Path::new("/tmp/hunger-bin");
        assert_eq!(
            default_database_url(dir),
            "sqlite:///tmp/hunger-bin/hunger.db?mode=rwc"
        );
        assert_eq!(
            resolve_settings_path("hunger.settings.json", dir),
            "/tmp/hunger-bin/hunger.settings.json"
        );
    }

    #[test]
    fn relative_sqlite_urls_are_rebased_to_executable_directory() {
        let dir = Path::new("/opt/hunger");
        assert_eq!(
            resolve_database_url("sqlite://hunger.db?mode=rwc", dir),
            "sqlite:///opt/hunger/hunger.db?mode=rwc"
        );
        assert_eq!(
            resolve_database_url("crawl-data.db", dir),
            "sqlite:///opt/hunger/crawl-data.db?mode=rwc"
        );
    }
}
