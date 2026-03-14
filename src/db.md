# db.rs

## Purpose
Owns SQLite schema creation and the queries that persist frontier state, crawled pages, and graph edges. It keeps the rest of the runtime from knowing SQL details.

## Components

### `Database`
- **Does**: Wraps a `SqlitePool` and exposes high-level methods for enqueueing, claiming work, storing crawl results, and reading dashboard snapshots.
- **Interacts with**: `Crawler` in `crawler.rs`, `ui.rs`

### `Database::store_crawl_result`
- **Does**: Upserts the page record, persists the parsed LLM novelty response and optional semantic embeddings when present, marks the frontier item complete, stores graph edges, and enqueues selected children.
- **Interacts with**: `NoveltyScore` in `models.rs`, `select_links` in `novelty.rs`

### `Database::snapshot`
- **Does**: Produces the aggregate view used by the dashboard polling endpoint.
- **Interacts with**: `DashboardSnapshot` in `models.rs`
- **Rationale**: Centralizes row decoding so SQLite-specific timestamp quirks are handled once instead of leaking into the UI.

### Backfill Queries / Updates
- **Does**: Counts missing semantic embeddings, exposes paged reads of stored novelty judgments for axis rescoring, and writes regenerated vectors or updated `llm_novelty_json` back onto existing pages.
- **Interacts with**: `BackfillController` in `backfill.rs`
- **Rationale**: Lets semantic visualization modes and operator-defined axis projections be retrofitted onto a preexisting crawl without re-fetching the web.

## Contracts

| Dependent | Expects | Breaking changes |
|-----------|---------|------------------|
| `crawler.rs` | Claim/store methods are idempotent enough for a single-worker loop | Changing enqueue upsert semantics |
| `ui.rs` | `snapshot()` remains cheap enough to poll every few seconds | Expanding snapshot cost without caching |

## Notes
- The first prototype keeps schema management in-process instead of adding a migration tool.
- Lightweight in-process migrations add new optional columns so portable databases can evolve without a separate migration binary.
