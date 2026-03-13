# settings.rs

## Purpose
Persists dashboard-editable settings to a local JSON file and exposes the active configuration to the crawler and UI. It bridges environment defaults, saved settings, and the running service.

## Components

### `SettingsManager`
- **Does**: Loads saved settings, merges older JSON files onto the current defaults, exposes the current config snapshot, and persists updates from the dashboard.
- **Interacts with**: `Config` in `config.rs`, `run` in `app.rs`, `Crawler` in `crawler.rs`, `ui.rs`

### `SettingsManager::update`
- **Does**: Validates an editable settings payload, writes it to disk, and swaps the active config.
- **Interacts with**: `EditableSettings` in `config.rs`
- **Rationale**: Keeps persistence and validation in one place so the UI does not drift from runtime behavior.

### `PartialEditableSettings`
- **Does**: Accepts older settings files with missing fields and merges them onto the current defaults during startup, including newer semantic-analysis toggles.
- **Interacts with**: `SettingsManager::new`

### `SettingsSnapshot`
- **Does**: Shapes the settings response returned to the dashboard, including which fields require a restart.
- **Interacts with**: `/api/settings` in `ui.rs`

## Contracts

| Dependent | Expects | Breaking changes |
|-----------|---------|------------------|
| `app.rs` | Saved settings are loaded before DB and server startup | Delaying overlay until after startup resources exist |
| `ui.rs` | `snapshot()` and `update()` are safe to call concurrently from handlers | Removing JSON persistence or pending-restart reporting |
