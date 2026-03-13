# novelty.rs

## Purpose
Implements the crawler's "food" logic by blending cheap local signals with the LLM judgment into a single energy score. It also decides which links deserve reproduction pressure.

## Components

### `compute_cheap_signals`
- **Does**: Measures lexical or embedding similarity, token novelty, domain redundancy, and graph diversity.
- **Interacts with**: `PageDigest` and `PageRecord` in `models.rs`

### `combine_scores`
- **Does**: Produces the final `NoveltyScore` using weighted heuristics plus the model judgment when available.
- **Interacts with**: `Config` in `config.rs`, `LlmNovelty` in `models.rs`

### `fallback_novelty`
- **Does**: Synthesizes a conservative `LlmNovelty`-shaped result when model scoring fails, leaving structured reasoning empty so the UI can distinguish heuristics from a real model judgment.
- **Interacts with**: `Config` in `config.rs`, `CheapSignals` in `models.rs`

### `select_links`
- **Does**: Converts page energy into a bounded set of outgoing links to enqueue next.
- **Interacts with**: `Crawler` in `crawler.rs`
- **Rationale**: Keeps the frontier-expansion policy explicit and easy to retune from one place.

## Contracts

| Dependent | Expects | Breaking changes |
|-----------|---------|------------------|
| `crawler.rs` | Scores are clamped to `[0,1]` and `select_links` stays deterministic for the same input | Removing clamping or making link selection stateful |
| `ui.rs` | `NoveltyScore` fields remain interpretable for operators | Renaming or removing score fields |
