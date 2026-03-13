# config.rs

## Purpose
Defines runtime configuration for the crawler, LLM gateway, storage, and thresholds. It centralizes environment parsing and the editable settings shape so the rest of the system can work with typed settings.

## Components

### `Config`
- **Does**: Holds bind address, SQLite path, settings file path, crawl thresholds, fetch timeout, OpenAI-compatible model settings including a dedicated LLM timeout, and toggles for optional semantic-map embedding passes.
- **Interacts with**: `run` in `app.rs`, `Crawler` in `crawler.rs`, `OpenAiCompatibleClient` in `llm.rs`, `SettingsManager` in `settings.rs`

### `Config::from_env`
- **Does**: Parses environment variables into a validated `Config`.
- **Interacts with**: `main` in `main.rs`
- **Rationale**: Keeps operational knobs out of the crawl loop, makes LM Studio vs hosted OpenAI selection a deployment concern, and resolves default storage paths relative to the executable directory for portability.

### `EditableSettings`, `Config::to_editable`, `Config::apply_editable`
- **Does**: Defines the dashboard-editable settings payload and converts it to runtime config.
- **Interacts with**: `SettingsManager` in `settings.rs`, handlers in `ui.rs`
- **Rationale**: Normalizes operator-entered LM Studio URLs so a bare host like `192.168.0.203:1234/v1` still becomes a valid HTTP base URL, lets model timeout differ from fetch timeout, rebases relative storage paths onto the executable directory, and makes the slower semantic-clustering paths explicit operator choices.
- **Notes**: `EditableSettings` supports equality checks so the native UI can distinguish unsaved edits from the last persisted snapshot.

## Contracts

| Dependent | Expects | Breaking changes |
|-----------|---------|------------------|
| `main.rs` | `from_env()` returns a ready-to-run config or an error | Renaming env vars, changing parsing defaults |
| `crawler.rs` | Thresholds and limits are already normalized | Changing field semantics |
| `settings.rs` | Editable settings can round-trip through JSON without losing meaning | Renaming editable fields or changing normalization semantics |
