# models.rs

## Purpose
Holds the shared data structures that move between the extractor, novelty scorer, storage layer, crawler runtime, and UI. It keeps the system vocabulary consistent across modules.

## Components

### `PageDigest`
- **Does**: Represents the cleaned content, extracted claims/entities, and outbound links for a fetched page.
- **Interacts with**: `extract_page` in `extractor.rs`, `Crawler` in `crawler.rs`

### `SemanticAxis`, `AxisScore`
- **Does**: Define operator-configured bipolar semantic axes and the LLM’s per-page placements on those axes.
- **Interacts with**: `Config` in `config.rs`, `OpenAiCompatibleClient` in `llm.rs`, `GraphViewport` in `graph_view.rs`

### `CheapSignals`, `LlmNovelty`, `NoveltyScore`
- **Does**: Separate heuristic scoring, model judgment, and final energy calculation into explicit stages.
- **Interacts with**: `compute_cheap_signals` and `combine_scores` in `novelty.rs`, `OpenAiCompatibleClient::score_novelty` in `llm.rs`
- **Rationale**: `LlmNovelty` keeps both the machine-readable scores and the model's structured reasoning so local-model output can be inspected without sacrificing automation.

### `FrontierEntry`, `PageRecord`, `DashboardSnapshot`
- **Does**: Shape persisted crawl state and the data served to the dashboard, including the stored parsed LLM novelty response and optional page / LLM semantic embeddings for inspected pages.
- **Interacts with**: `Database` in `db.rs`, handlers in `ui.rs`

### `PageEmbeddingBackfillRecord`, `LlmEmbeddingBackfillRecord`, `AxisBackfillRecord`
- **Does**: Provide the minimal payloads the background backfill pass needs to regenerate page embeddings, LLM-response embeddings, and operator-axis placements for already-stored pages.
- **Interacts with**: `Database` in `db.rs`, `BackfillController` in `backfill.rs`

## Contracts

| Dependent | Expects | Breaking changes |
|-----------|---------|------------------|
| `db.rs` | Field names serialize cleanly to JSON and SQL columns, with defaults for backward-compatible decoding | Renaming fields or changing types without defaults |
| `ui.rs` | Dashboard structs are serializable and stable enough for the polling UI | Removing fields used by the UI |
