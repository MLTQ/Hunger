# crawler.rs

## Purpose
Runs the long-lived crawl loop that pulls work from the frontier, fetches pages, extracts digests, scores novelty, and grows the graph. It is the orchestrator for the first prototype and reads live settings before each decision.

## Components

### `Crawler`
- **Does**: Owns the runtime dependencies for fetching, scoring, persistence, and live settings reads.
- **Interacts with**: `Database` in `db.rs`, `SettingsManager` in `settings.rs`, `RuntimeControl` in `control.rs`, `OpenAiCompatibleClient` in `llm.rs`

### `Crawler::run_forever`
- **Does**: Repeats the frontier claim/process cycle, respecting pause/resume control and idling using the current saved delay setting.
- **Interacts with**: `RuntimeControl` in `control.rs`, `SettingsManager::current` in `settings.rs`

### `Crawler::process_frontier_item`
- **Does**: Fetches a page, builds a digest, requests embeddings and model judgment when appropriate, optionally records extra semantic-map embeddings, computes energy, and stores the result using the current active settings.
- **Interacts with**: `extract_page` in `extractor.rs`, scoring functions in `novelty.rs`
- **Rationale**: Logs full error chains for model/embedding failures so endpoint and connectivity problems are visible without attaching a debugger.

## Contracts

| Dependent | Expects | Breaking changes |
|-----------|---------|------------------|
| `app.rs` | `run_forever` never returns under normal operation | Changing loop ownership or shutdown semantics |
| `db.rs` | `process_frontier_item` stores a fully-scored page or marks the item failed | Partial writes without status updates |

## Notes
- Robots-aware fetching is still a TODO; the first cut enforces HTML-only crawling and bounded expansion.
- Page-content and LLM-judgment embeddings are independently switchable so operators can trade semantic richness against local-model latency.
