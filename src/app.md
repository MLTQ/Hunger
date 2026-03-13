# app.rs

## Purpose
Composes the configured runtime into a running desktop app: persisted settings, database, crawler task, Tokio runtime, and native egui shell. It is the boundary between static configuration and live execution.

## Components

### `run`
- **Does**: Loads saved settings, opens SQLite, seeds the frontier, starts the crawler background task, creates the semantic backfill controller, builds the Tokio runtime, and launches the native egui app.
- **Interacts with**: `SettingsManager` in `settings.rs`, `Database` in `db.rs`, `Crawler` in `crawler.rs`, `BackfillController` in `backfill.rs`, `HungerApp` in `ui.rs`

## Notes
- Startup logs include the semantic-map toggles so operators can confirm whether the expensive embedding passes are live before a crawl begins.

## Contracts

| Dependent | Expects | Breaking changes |
|-----------|---------|------------------|
| `main.rs` | `run(config)` blocks until the desktop window exits or returns an error | Changing it to background-only behavior |
| Operators | Saved settings are applied before startup and seed URLs are enqueued before the UI starts | Delaying settings overlay or frontier initialization |
