# control.rs

## Purpose
Provides lightweight runtime controls shared between the desktop UI and the crawler loop. It currently owns pause and resume state without forcing the crawler to know about UI details.

## Components

### `RuntimeControl`
- **Does**: Stores the shared paused flag and exposes pause/resume/toggle helpers.
- **Interacts with**: `Crawler` in `crawler.rs`, `HungerApp` in `ui.rs`

## Contracts

| Dependent | Expects | Breaking changes |
|-----------|---------|------------------|
| `crawler.rs` | `is_paused()` is cheap and lock-free inside the main crawl loop | Replacing atomics with blocking synchronization |
| `ui.rs` | `toggle()` immediately affects crawler behavior | Delaying state propagation |
