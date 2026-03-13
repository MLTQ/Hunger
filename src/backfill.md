# backfill.rs

## Purpose
Runs an operator-triggered background pass that backfills semantic embeddings for pages already stored in SQLite. It keeps that maintenance work out of the main crawler loop so semantic modes can be enabled after data already exists.

## Components

### `BackfillController`
- **Does**: Starts at most one background backfill job at a time, tracks progress, and exposes a snapshot for the UI.
- **Interacts with**: `UiContext` in `ui.rs`, `Database` in `db.rs`, `SettingsManager` in `settings.rs`

### `BackfillController::run`
- **Does**: Reads current settings, counts missing page / LLM embeddings, requests embeddings in batches, and writes them back to SQLite.
- **Interacts with**: `OpenAiCompatibleClient` in `llm.rs`
- **Rationale**: Lets operators retrofit semantic clustering onto an old crawl without waiting for pages to be revisited organically.

## Contracts

| Dependent | Expects | Breaking changes |
|-----------|---------|------------------|
| `ui.rs` | Backfill status is cheap to poll and only one backfill job can be active at a time | Removing progress tracking or allowing overlapping writes |
| Operators | Existing crawled pages can gain semantic embeddings without clearing the database | Making backfill destructive or crawl-blocking |
