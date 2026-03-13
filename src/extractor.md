# extractor.rs

## Purpose
Converts raw HTML into the crawler's compact page digest. It performs lightweight readability extraction, link normalization, and simple claim/entity harvesting so the scorer can work on structured input instead of raw markup.

## Components

### `extract_page`
- **Does**: Parses HTML, extracts content and outbound links, computes a content hash, and returns a `PageDigest`.
- **Interacts with**: `Crawler::process_frontier_item` in `crawler.rs`, `PageDigest` in `models.rs`

### Extraction helpers
- **Does**: Normalize whitespace, split sentences, infer entities, and estimate boilerplate.
- **Interacts with**: Internal to `extract_page`
- **Rationale**: Keeps the first prototype deterministic and local, even before introducing heavier extraction services.

## Contracts

| Dependent | Expects | Breaking changes |
|-----------|---------|------------------|
| `crawler.rs` | `extract_page` returns normalized HTTP(S) links and non-panicking digests for malformed HTML | Changing link normalization or returning errors |
| `novelty.rs` | Summaries, claims, and entities are concise and bounded | Returning unbounded lists or empty text for valid pages |

## Notes
- Entity extraction is heuristic and intentionally shallow for v1.
