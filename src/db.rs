use anyhow::{Context, Result};
use chrono::{DateTime, NaiveDateTime, Utc};
use sqlx::{Row, SqlitePool, sqlite::SqlitePoolOptions};

use crate::models::{
    DashboardSnapshot, FrontierEntry, GraphEdge, GraphNode, LlmEmbeddingBackfillRecord, LlmNovelty,
    NoveltyScore, OutboundLink, PageDigest, PageEmbeddingBackfillRecord, PageRecord,
};

#[derive(Clone)]
pub struct Database {
    pool: SqlitePool,
}

impl Database {
    pub async fn connect(database_url: &str) -> Result<Self> {
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect(database_url)
            .await
            .with_context(|| format!("failed to connect to database at {database_url}"))?;
        let database = Self { pool };
        database.init().await?;
        Ok(database)
    }

    pub async fn enqueue_seed_urls(&self, urls: &[String]) -> Result<()> {
        for url in urls {
            self.enqueue_link(url, None, None, 0, 0.5).await?;
        }
        Ok(())
    }

    pub async fn enqueue_link(
        &self,
        url: &str,
        discovered_from: Option<&str>,
        anchor: Option<&str>,
        depth: i64,
        energy: f32,
    ) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO frontier (url, discovered_from, anchor, depth, energy, status, updated_at)
            VALUES (?1, ?2, ?3, ?4, ?5, 'queued', CURRENT_TIMESTAMP)
            ON CONFLICT(url) DO UPDATE SET
                energy = MAX(frontier.energy, excluded.energy),
                discovered_from = COALESCE(frontier.discovered_from, excluded.discovered_from),
                anchor = COALESCE(frontier.anchor, excluded.anchor),
                depth = MIN(frontier.depth, excluded.depth),
                updated_at = CURRENT_TIMESTAMP
            "#,
        )
        .bind(url)
        .bind(discovered_from)
        .bind(anchor)
        .bind(depth)
        .bind(energy)
        .execute(&self.pool)
        .await
        .context("failed to enqueue frontier url")?;
        Ok(())
    }

    pub async fn claim_frontier_item(&self) -> Result<Option<FrontierEntry>> {
        let row = sqlx::query(
            r#"
            SELECT url, discovered_from, anchor, depth, energy, status
            FROM frontier
            WHERE status = 'queued'
            ORDER BY energy DESC, updated_at ASC
            LIMIT 1
            "#,
        )
        .fetch_optional(&self.pool)
        .await
        .context("failed to read frontier")?;

        let Some(row) = row else {
            return Ok(None);
        };

        let entry = FrontierEntry {
            url: row.get("url"),
            discovered_from: row.get("discovered_from"),
            anchor: row.get("anchor"),
            depth: row.get("depth"),
            energy: row.get::<f64, _>("energy") as f32,
            status: row.get("status"),
        };

        sqlx::query("UPDATE frontier SET status = 'processing', updated_at = CURRENT_TIMESTAMP WHERE url = ?1")
            .bind(&entry.url)
            .execute(&self.pool)
            .await
            .context("failed to mark frontier row as processing")?;

        Ok(Some(entry))
    }

    pub async fn mark_frontier_failed(&self, url: &str, error: &str) -> Result<()> {
        sqlx::query(
            "UPDATE frontier SET status = 'failed', updated_at = CURRENT_TIMESTAMP, last_error = ?2 WHERE url = ?1",
        )
        .bind(url)
        .bind(error)
        .execute(&self.pool)
        .await
        .context("failed to mark frontier item as failed")?;
        Ok(())
    }

    pub async fn recent_pages(&self, limit: i64) -> Result<Vec<PageRecord>> {
        self.fetch_pages(
            r#"
            SELECT url, domain, title, summary, reason, food_energy, depth, content_hash, embedding_json, created_at
            , llm_embedding_json, llm_novelty_json
            FROM pages
            ORDER BY created_at DESC
            LIMIT ?1
            "#,
            limit,
        )
        .await
    }

    pub async fn same_domain_pages(&self, domain: &str, limit: i64) -> Result<Vec<PageRecord>> {
        let rows = sqlx::query(
            r#"
            SELECT url, domain, title, summary, reason, food_energy, depth, content_hash, embedding_json, created_at
            , llm_embedding_json, llm_novelty_json
            FROM pages
            WHERE domain = ?1
            ORDER BY created_at DESC
            LIMIT ?2
            "#,
        )
        .bind(domain)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .context("failed to fetch same-domain pages")?;

        rows.into_iter().map(row_to_page_record).collect()
    }

    pub async fn store_crawl_result(
        &self,
        entry: &FrontierEntry,
        digest: &PageDigest,
        embedding: Option<&[f32]>,
        llm_embedding: Option<&[f32]>,
        llm_novelty: Option<&LlmNovelty>,
        score: &NoveltyScore,
        selected_links: &[OutboundLink],
    ) -> Result<()> {
        let embedding_json = embedding.map(serde_json::to_string).transpose()?;
        let llm_embedding_json = llm_embedding.map(serde_json::to_string).transpose()?;
        let llm_novelty_json = llm_novelty.map(serde_json::to_string).transpose()?;
        sqlx::query(
            r#"
            INSERT INTO pages (
                url, domain, title, summary, clean_text, reason, food_energy, depth, content_hash,
                embedding_json, llm_embedding_json, llm_novelty_json, factual_novelty, entity_novelty, relational_novelty, temporal_novelty,
                expected_crawl_value, semantic_distance, new_token_ratio, local_graph_diversity,
                domain_redundancy, boilerplate_penalty, created_at
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9,
                ?10, ?11, ?12, ?13, ?14, ?15, ?16,
                ?17, ?18, ?19, ?20,
                ?21, ?22, CURRENT_TIMESTAMP
            )
            ON CONFLICT(url) DO UPDATE SET
                domain = excluded.domain,
                title = excluded.title,
                summary = excluded.summary,
                clean_text = excluded.clean_text,
                reason = excluded.reason,
                food_energy = excluded.food_energy,
                depth = excluded.depth,
                content_hash = excluded.content_hash,
                embedding_json = excluded.embedding_json,
                llm_embedding_json = excluded.llm_embedding_json,
                llm_novelty_json = excluded.llm_novelty_json,
                factual_novelty = excluded.factual_novelty,
                entity_novelty = excluded.entity_novelty,
                relational_novelty = excluded.relational_novelty,
                temporal_novelty = excluded.temporal_novelty,
                expected_crawl_value = excluded.expected_crawl_value,
                semantic_distance = excluded.semantic_distance,
                new_token_ratio = excluded.new_token_ratio,
                local_graph_diversity = excluded.local_graph_diversity,
                domain_redundancy = excluded.domain_redundancy,
                boilerplate_penalty = excluded.boilerplate_penalty
            "#,
        )
        .bind(&digest.url)
        .bind(&digest.domain)
        .bind(&digest.title)
        .bind(&digest.short_summary)
        .bind(&digest.clean_text)
        .bind(&score.concise_reason)
        .bind(score.food_energy)
        .bind(entry.depth)
        .bind(&digest.content_hash)
        .bind(embedding_json)
        .bind(llm_embedding_json)
        .bind(llm_novelty_json)
        .bind(score.factual_novelty)
        .bind(score.entity_novelty)
        .bind(score.relational_novelty)
        .bind(score.temporal_novelty)
        .bind(score.expected_crawl_value)
        .bind(score.semantic_distance)
        .bind(score.new_token_ratio)
        .bind(score.local_graph_diversity)
        .bind(score.domain_redundancy)
        .bind(score.boilerplate_penalty)
        .execute(&self.pool)
        .await
        .context("failed to store crawled page")?;

        sqlx::query(
            "UPDATE frontier SET status = 'done', updated_at = CURRENT_TIMESTAMP WHERE url = ?1",
        )
        .bind(&entry.url)
        .execute(&self.pool)
        .await
        .context("failed to mark frontier item done")?;

        sqlx::query("DELETE FROM links WHERE source_url = ?1")
            .bind(&digest.url)
            .execute(&self.pool)
            .await
            .context("failed to clear prior links")?;

        for link in selected_links {
            sqlx::query("INSERT INTO links (source_url, target_url, anchor) VALUES (?1, ?2, ?3)")
                .bind(&digest.url)
                .bind(&link.url)
                .bind(&link.anchor)
                .execute(&self.pool)
                .await
                .context("failed to insert link edge")?;

            self.enqueue_link(
                &link.url,
                Some(&digest.url),
                Some(&link.anchor),
                entry.depth + 1,
                score.food_energy,
            )
            .await?;
        }

        Ok(())
    }

    pub async fn snapshot(&self) -> Result<DashboardSnapshot> {
        let totals = sqlx::query(
            r#"
            SELECT
                (SELECT COUNT(*) FROM pages) AS total_pages,
                (SELECT COUNT(*) FROM frontier WHERE status = 'queued') AS queued_pages,
                (SELECT COUNT(*) FROM frontier WHERE status = 'failed') AS failed_pages
            "#,
        )
        .fetch_one(&self.pool)
        .await
        .context("failed to fetch totals")?;

        let pages = self
            .fetch_pages(
                r#"
                SELECT url, domain, title, summary, reason, food_energy, depth, content_hash, embedding_json, created_at
                , llm_embedding_json, llm_novelty_json
                FROM pages
                ORDER BY created_at DESC
                LIMIT ?1
                "#,
                48,
            )
            .await?;

        let frontier_rows = sqlx::query(
            r#"
            SELECT url, discovered_from, anchor, depth, energy, status
            FROM frontier
            ORDER BY
                CASE status
                    WHEN 'processing' THEN 0
                    WHEN 'queued' THEN 1
                    WHEN 'failed' THEN 2
                    ELSE 3
                END,
                energy DESC,
                updated_at DESC
            LIMIT 48
            "#,
        )
        .fetch_all(&self.pool)
        .await
        .context("failed to fetch frontier snapshot")?;

        let frontier = frontier_rows
            .into_iter()
            .map(|row| FrontierEntry {
                url: row.get("url"),
                discovered_from: row.get("discovered_from"),
                anchor: row.get("anchor"),
                depth: row.get("depth"),
                energy: row.get::<f64, _>("energy") as f32,
                status: row.get("status"),
            })
            .collect::<Vec<_>>();

        let graph_nodes = pages
            .iter()
            .map(|page| GraphNode {
                url: page.url.clone(),
                title: page.title.clone(),
                domain: page.domain.clone(),
                food_energy: page.food_energy,
                reason: page.reason.clone(),
            })
            .collect::<Vec<_>>();

        let graph_edges = sqlx::query(
            r#"
            SELECT source_url, target_url, anchor
            FROM links
            ORDER BY rowid DESC
            LIMIT 128
            "#,
        )
        .fetch_all(&self.pool)
        .await
        .context("failed to fetch graph edges")?
        .into_iter()
        .map(|row| GraphEdge {
            source_url: row.get("source_url"),
            target_url: row.get("target_url"),
            anchor: row.get("anchor"),
        })
        .collect();

        Ok(DashboardSnapshot {
            total_pages: totals.get("total_pages"),
            queued_pages: totals.get("queued_pages"),
            failed_pages: totals.get("failed_pages"),
            pages,
            frontier,
            graph_nodes,
            graph_edges,
        })
    }

    pub async fn count_missing_page_embeddings(&self) -> Result<i64> {
        let row = sqlx::query(
            r#"
            SELECT COUNT(*) AS count
            FROM pages
            WHERE embedding_json IS NULL
            "#,
        )
        .fetch_one(&self.pool)
        .await
        .context("failed to count pages missing page embeddings")?;

        Ok(row.get("count"))
    }

    pub async fn count_missing_llm_embeddings(&self) -> Result<i64> {
        let row = sqlx::query(
            r#"
            SELECT COUNT(*) AS count
            FROM pages
            WHERE llm_embedding_json IS NULL
              AND llm_novelty_json IS NOT NULL
            "#,
        )
        .fetch_one(&self.pool)
        .await
        .context("failed to count pages missing llm embeddings")?;

        Ok(row.get("count"))
    }

    pub async fn page_embedding_backfill_batch(
        &self,
        limit: i64,
    ) -> Result<Vec<PageEmbeddingBackfillRecord>> {
        let rows = sqlx::query(
            r#"
            SELECT url, title, summary, clean_text
            FROM pages
            WHERE embedding_json IS NULL
            ORDER BY created_at ASC
            LIMIT ?1
            "#,
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .context("failed to fetch page embedding backfill batch")?;

        Ok(rows
            .into_iter()
            .map(|row| PageEmbeddingBackfillRecord {
                url: row.get("url"),
                title: row.get("title"),
                summary: row.get("summary"),
                clean_text: row.get("clean_text"),
            })
            .collect())
    }

    pub async fn llm_embedding_backfill_batch(
        &self,
        limit: i64,
    ) -> Result<Vec<LlmEmbeddingBackfillRecord>> {
        let rows = sqlx::query(
            r#"
            SELECT url, llm_novelty_json
            FROM pages
            WHERE llm_embedding_json IS NULL
              AND llm_novelty_json IS NOT NULL
            ORDER BY created_at ASC
            LIMIT ?1
            "#,
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .context("failed to fetch llm embedding backfill batch")?;

        rows.into_iter()
            .map(|row| {
                let json: String = row.get("llm_novelty_json");
                let llm_novelty = serde_json::from_str(&json)
                    .context("failed to parse llm_novelty_json for backfill")?;
                Ok(LlmEmbeddingBackfillRecord {
                    url: row.get("url"),
                    llm_novelty,
                })
            })
            .collect()
    }

    pub async fn update_page_embedding(&self, url: &str, embedding: &[f32]) -> Result<()> {
        let embedding_json = serde_json::to_string(embedding)
            .context("failed to serialize page embedding update")?;
        sqlx::query("UPDATE pages SET embedding_json = ?2 WHERE url = ?1")
            .bind(url)
            .bind(embedding_json)
            .execute(&self.pool)
            .await
            .with_context(|| format!("failed to update page embedding for {url}"))?;
        Ok(())
    }

    pub async fn update_llm_embedding(&self, url: &str, embedding: &[f32]) -> Result<()> {
        let embedding_json =
            serde_json::to_string(embedding).context("failed to serialize llm embedding update")?;
        sqlx::query("UPDATE pages SET llm_embedding_json = ?2 WHERE url = ?1")
            .bind(url)
            .bind(embedding_json)
            .execute(&self.pool)
            .await
            .with_context(|| format!("failed to update llm embedding for {url}"))?;
        Ok(())
    }

    async fn fetch_pages(&self, query: &str, limit: i64) -> Result<Vec<PageRecord>> {
        let rows = sqlx::query(query)
            .bind(limit)
            .fetch_all(&self.pool)
            .await
            .context("failed to fetch pages")?;

        rows.into_iter().map(row_to_page_record).collect()
    }

    async fn init(&self) -> Result<()> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS pages (
                url TEXT PRIMARY KEY,
                domain TEXT NOT NULL,
                title TEXT NOT NULL,
                summary TEXT NOT NULL,
                clean_text TEXT NOT NULL,
                reason TEXT NOT NULL,
                food_energy REAL NOT NULL,
                depth INTEGER NOT NULL,
                content_hash TEXT NOT NULL,
                embedding_json TEXT,
                llm_embedding_json TEXT,
                llm_novelty_json TEXT,
                factual_novelty REAL NOT NULL,
                entity_novelty REAL NOT NULL,
                relational_novelty REAL NOT NULL,
                temporal_novelty REAL NOT NULL,
                expected_crawl_value REAL NOT NULL,
                semantic_distance REAL NOT NULL,
                new_token_ratio REAL NOT NULL,
                local_graph_diversity REAL NOT NULL,
                domain_redundancy REAL NOT NULL,
                boilerplate_penalty REAL NOT NULL,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );
            "#,
        )
        .execute(&self.pool)
        .await
        .context("failed to create pages table")?;

        match sqlx::query("ALTER TABLE pages ADD COLUMN llm_novelty_json TEXT")
            .execute(&self.pool)
            .await
        {
            Ok(_) => {}
            Err(error) if error.to_string().contains("duplicate column name") => {}
            Err(error) => {
                return Err(error).context("failed to add llm_novelty_json column");
            }
        }

        match sqlx::query("ALTER TABLE pages ADD COLUMN llm_embedding_json TEXT")
            .execute(&self.pool)
            .await
        {
            Ok(_) => {}
            Err(error) if error.to_string().contains("duplicate column name") => {}
            Err(error) => {
                return Err(error).context("failed to add llm_embedding_json column");
            }
        }

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS frontier (
                url TEXT PRIMARY KEY,
                discovered_from TEXT,
                anchor TEXT,
                depth INTEGER NOT NULL DEFAULT 0,
                energy REAL NOT NULL DEFAULT 0.0,
                status TEXT NOT NULL DEFAULT 'queued',
                last_error TEXT,
                updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );
            "#,
        )
        .execute(&self.pool)
        .await
        .context("failed to create frontier table")?;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS links (
                source_url TEXT NOT NULL,
                target_url TEXT NOT NULL,
                anchor TEXT NOT NULL
            );
            "#,
        )
        .execute(&self.pool)
        .await
        .context("failed to create links table")?;

        Ok(())
    }
}

fn row_to_page_record(row: sqlx::sqlite::SqliteRow) -> Result<PageRecord> {
    let created_at: String = row.get("created_at");
    let created_at = DateTime::parse_from_rfc3339(&created_at)
        .map(|value| value.with_timezone(&Utc))
        .or_else(|_| {
            NaiveDateTime::parse_from_str(&created_at, "%Y-%m-%d %H:%M:%S")
                .map(|value| DateTime::<Utc>::from_naive_utc_and_offset(value, Utc))
        })
        .with_context(|| format!("failed to parse stored timestamp {created_at}"))?;

    let embedding = row
        .get::<Option<String>, _>("embedding_json")
        .map(|json| serde_json::from_str(&json))
        .transpose()
        .context("failed to parse embedding_json")?;
    let llm_novelty = row
        .get::<Option<String>, _>("llm_novelty_json")
        .map(|json| serde_json::from_str(&json))
        .transpose()
        .context("failed to parse llm_novelty_json")?;
    let llm_embedding = row
        .get::<Option<String>, _>("llm_embedding_json")
        .map(|json| serde_json::from_str(&json))
        .transpose()
        .context("failed to parse llm_embedding_json")?;

    Ok(PageRecord {
        url: row.get("url"),
        domain: row.get("domain"),
        title: row.get("title"),
        summary: row.get("summary"),
        reason: row.get("reason"),
        food_energy: row.get::<f64, _>("food_energy") as f32,
        depth: row.get("depth"),
        content_hash: row.get("content_hash"),
        embedding,
        llm_embedding,
        llm_novelty,
        created_at,
    })
}

#[cfg(test)]
mod tests {
    use super::Database;

    #[tokio::test]
    async fn snapshot_handles_sqlite_timestamps() {
        let path = format!(
            "sqlite://{}/db-snapshot-test-{}.sqlite?mode=rwc",
            std::env::temp_dir().display(),
            std::process::id()
        );
        let database = Database::connect(&path).await.expect("connect database");

        database
            .enqueue_link("https://example.com", None, None, 0, 0.5)
            .await
            .expect("enqueue seed");

        let entry = database
            .claim_frontier_item()
            .await
            .expect("claim frontier item")
            .expect("frontier item exists");

        let digest = crate::models::PageDigest {
            url: "https://example.com".to_string(),
            domain: "example.com".to_string(),
            title: "Example".to_string(),
            clean_text: "Example page text".to_string(),
            short_summary: "Example page text".to_string(),
            key_points: Vec::new(),
            entities: Vec::new(),
            claims: Vec::new(),
            outbound_links: Vec::new(),
            boilerplate_ratio: 0.2,
            content_hash: "hash".to_string(),
        };
        let score = crate::models::NoveltyScore {
            food_energy: 0.5,
            factual_novelty: 0.5,
            entity_novelty: 0.4,
            relational_novelty: 0.3,
            temporal_novelty: 1.0,
            expected_crawl_value: 0.5,
            concise_reason: "test".to_string(),
            likely_novel_aspects: Vec::new(),
            likely_redundant_aspects: Vec::new(),
            semantic_distance: 0.6,
            new_token_ratio: 0.7,
            local_graph_diversity: 0.1,
            domain_redundancy: 0.0,
            boilerplate_penalty: 0.2,
        };

        database
            .store_crawl_result(&entry, &digest, None, None, None, &score, &[])
            .await
            .expect("store crawl result");

        let snapshot = database.snapshot().await.expect("build snapshot");
        assert_eq!(snapshot.total_pages, 1);

        let _ = std::fs::remove_file(format!(
            "{}/db-snapshot-test-{}.sqlite",
            std::env::temp_dir().display(),
            std::process::id()
        ));
    }
}
