# graph_view.rs

## Purpose
Owns the native 3D force-directed network viewport. It keeps graph layout, camera controls, projection, and rendering separate from the rest of the desktop UI so the crawler panel can stay focused on operations.

## Components

### `GraphViewport`
- **Does**: Synchronizes node physics with the latest crawl snapshot, runs a 3D force simulation, and renders the projected graph with switchable color/overlay modes for novelty, page semantics, LLM-judgment semantics, and operator-defined axis projection.
- **Interacts with**: `DashboardSnapshot` in `models.rs`, `HungerApp` in `ui.rs`
- **Rationale**: Semantic modes now affect layout as well as color by adding attraction between resonance pairs and cluster-level anchoring forces.

### `GraphViewport::draw`
- **Does**: Handles orbit/zoom input, advances the layout, paints the graph with the cold neon styling, overlays semantic resonance lines when enabled, and shows hover tooltips with page results.
- **Interacts with**: egui painter APIs

### `GraphColorMode`
- **Does**: Encodes the operator-selected visual analysis mode so the viewport can swap palettes and semantic clustering behavior, including an `AXES` mode that projects nodes from stored LLM axis scores.
- **Interacts with**: `HungerApp` in `ui.rs`

### `FieldTelemetry`
- **Does**: Summarizes coverage, cluster sizes, resonance links, and domain concentration for the currently active graph mode.
- **Interacts with**: `HungerApp` in `ui.rs`

## Contracts

| Dependent | Expects | Breaking changes |
|-----------|---------|------------------|
| `ui.rs` | `draw()` is self-contained and does not mutate crawl state | Requiring external layout bookkeeping |
| Operators | Graph layout is 3D and not a fixed circular placement, while semantic modes add clustering cues without destabilizing the force field | Reverting to static projection or 2D-only layout |
