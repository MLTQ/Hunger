# ui.rs

## Purpose
Provides the native egui desktop application for inspecting the live crawl graph, recent scoring decisions, frontier state, and persisted settings. It replaces the temporary browser UI with a sharper operator console.

## Components

### `UiContext`
- **Does**: Shares the Tokio runtime, database, settings manager, runtime controls, and semantic backfill controller across the desktop app.
- **Interacts with**: `run` in `app.rs`

### `HungerApp`
- **Does**: Owns the native dashboard state, settings editor, pause button, and live snapshot refresh loop.
- **Interacts with**: `Database::snapshot` in `db.rs`, `SettingsManager` in `settings.rs`, `RuntimeControl` in `control.rs`, `GraphViewport` in `graph_view.rs`
- **Rationale**: Keeps in-progress settings edits separate from the polled persisted snapshot so the refresh loop does not overwrite typed changes, and now polls snapshots asynchronously so long SQLite reads/writes do not freeze the egui thread.

### `HungerApp::draw_top_bar`, `draw_side_panel`, `draw_center`
- **Does**: Splits the operator console into transport controls, graph analysis mode selection, semantic backfill controls, settings, graph viewport, and live lists.
- **Interacts with**: egui panel/layout APIs
- **Rationale**: Keeps the native UI structured so the graph work and crawler controls can evolve independently, including giving operators direct control over the slower LLM timeout and semantic-analysis cost toggles.

### `draw_metrics`, `draw_status_bar`
- **Does**: Renders the crawler status as a fixed-width stacked bar with per-segment counts centered above the done, queued, and failed regions.
- **Interacts with**: `DashboardSnapshot` in `models.rs`, egui painter APIs

### `draw_command_panel`
- **Does**: Renders a dedicated telemetry/radar panel from the graph viewport’s active-mode summary.
- **Interacts with**: `FieldTelemetry` in `graph_view.rs`, `BackfillStatus` in `backfill.rs`

### `draw_page_card`
- **Does**: Renders recent pages and exposes the stored parsed LLM novelty response in a collapsible section when one exists, including the model's structured reasoning text.
- **Interacts with**: `PageRecord` in `models.rs`

## Notes
- The graph mode selector only changes the visual analysis layer; the underlying crawl graph and 3D force layout stay the same.
- The semantic backfill button is intentionally separate from crawling so operators can retrofit old datasets without wiping the database or waiting for revisits.

## Contracts

| Dependent | Expects | Breaking changes |
|-----------|---------|------------------|
| Operators | The desktop app reflects live crawler state, supports pause/resume, and exposes settings and seed controls | Regressing to a static or browser-only interface |
| `app.rs` | `HungerApp::new` constructs from shared runtime services and can be handed to `eframe::run_native` | Requiring external web services to render the UI |
