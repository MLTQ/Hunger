# Hunger

Novelty-driven crawler prototype in Rust. It treats web discovery as an energy system: each fetched page is digested, compared to crawler memory, scored for novelty, and allowed to reproduce into more links when it looks nutritionally rich.

## Features

- Rust crawler core with SQLite state
- OpenAI-compatible chat + optional embeddings, tuned for LM Studio first
- Hybrid novelty scoring: cheap local signals plus model judgment
- Native egui desktop app with a 3D force-directed graph viewport
- Operator seeding via config or UI

## Run

1. Start LM Studio's local server and load a chat model.
2. Optionally load an embedding model and set `HUNGER_EMBEDDING_MODEL`.
3. Export configuration:

```bash
export HUNGER_LLM_BASE_URL="http://localhost:1234/v1"
export HUNGER_LLM_API_KEY="lm-studio"
export HUNGER_LLM_MODEL="your-chat-model"
export HUNGER_EMBEDDING_MODEL="your-embedding-model"   # optional
export HUNGER_LLM_TIMEOUT_SECS="75"
export HUNGER_SEEDS="https://example.com,https://example.org"
export HUNGER_DATABASE_URL="sqlite:///absolute/path/to/hunger.db?mode=rwc"
export HUNGER_SETTINGS_PATH="/absolute/path/to/hunger.settings.json"     # optional
```

4. Launch the desktop app:

```bash
cargo run
```

5. Use the native settings pane to change models, thresholds, pacing, seeds, and database URL.

## Notes

- The first cut stores embeddings as JSON in SQLite for simplicity.
- By default, both `hunger.db` and `hunger.settings.json` live next to the executable, not the shell's current working directory.
- UI-edited settings are saved to the executable directory by default.
- `database_url` is persisted immediately but applies on restart.
- Screenshot capture and robots-aware crawling are still TODOs.
