# backfill.rs

## Purpose
Runs an operator-triggered background pass that backfills semantic embeddings and operator-axis placements for pages already stored in SQLite. It keeps that maintenance work out of the main crawler loop so semantic modes or the AXES projection can be enabled after data already exists.

## Components

### `BackfillController`
- **Does**: Starts at most one background backfill job at a time, temporarily pauses the live crawler while the pass is active, tracks progress, and exposes a snapshot for the UI.
- **Interacts with**: `UiContext` in `ui.rs`, `Database` in `db.rs`, `SettingsManager` in `settings.rs`, `RuntimeControl` in `control.rs`

### `BackfillController::run`
- **Does**: Reads current settings, counts missing page / LLM embeddings, scans stored novelty rows in pages instead of loading them all at once, runs up to four concurrent model requests per batch, throttles between completions, and writes the updates back to SQLite.
- **Interacts with**: `OpenAiCompatibleClient` in `llm.rs`
- **Rationale**: Lets operators retrofit semantic clustering or new semantic-axis projections onto an old crawl without waiting for pages to be revisited organically.

## Contracts

| Dependent | Expects | Breaking changes |
|-----------|---------|------------------|
| `ui.rs` | Backfill status is cheap to poll and only one backfill job can be active at a time | Removing progress tracking or allowing overlapping writes |
| Operators | Existing crawled pages can gain semantic embeddings and/or axis placements without clearing the database | Making backfill destructive or crawl-blocking |
