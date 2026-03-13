# llm.rs

## Purpose
Wraps an OpenAI-compatible HTTP surface for novelty judgments and optional embeddings. It is the adapter that lets the crawler target LM Studio first while remaining portable to hosted OpenAI-compatible endpoints later.

## Components

### `OpenAiCompatibleClient`
- **Does**: Manages HTTP calls to `/chat/completions` and `/embeddings` using the dedicated model timeout.
- **Interacts with**: `Config` in `config.rs`, `Crawler` in `crawler.rs`

### `OpenAiCompatibleClient::score_novelty`
- **Does**: Builds a compact novelty prompt, prefers structured-output hints when the server accepts them, calls the chat model, and parses the structured judgment.
- **Interacts with**: `PageDigest` and `PageRecord` in `models.rs`
- **Rationale**: Keeps model-specific prompting isolated so the core automaton logic can stay model-agnostic while remaining fast enough for local inference.

### `OpenAiCompatibleClient::repair_novelty_response`
- **Does**: Performs a second, shorter repair pass when a local model emits prose, markdown, or truncated reasoning instead of directly returning valid JSON.
- **Interacts with**: `score_novelty`, `parse_llm_json`
- **Rationale**: OpenAI-compatible local servers are often close to compliant but not strict enough for single-pass JSON decoding, so a repair path is cheaper than discarding the judgment entirely.

### `OpenAiCompatibleClient::send_chat_completion`
- **Does**: Sends chat requests and automatically retries without `response_format` if an OpenAI-compatible server rejects the structured-output hint.
- **Interacts with**: `score_novelty`, `repair_novelty_response`
- **Rationale**: LM Studio and similar servers often implement only part of the OpenAI surface, so feature negotiation has to happen client-side.

### `OpenAiCompatibleClient::embed_text`
- **Does**: Requests an embedding when an embedding model is configured.
- **Interacts with**: `compute_cheap_signals` in `novelty.rs`

## Contracts

| Dependent | Expects | Breaking changes |
|-----------|---------|------------------|
| `crawler.rs` | Errors are returned instead of panics when LM Studio is unavailable or non-compliant | Swallowing failures or changing fallback semantics |
| `config.rs` | Base URL and model names map directly onto OpenAI-compatible endpoints | Changing URL construction or auth expectations |

## Notes
- The parser accepts pure JSON, JSON embedded in surrounding prose, or fenced JSON blocks before falling back to a repair request.
- Reasoning is now expected to live inside the structured payload so it can be stored and rendered without breaking the crawler loop.
- If the server rejects `response_format`, the client retries the same request as plain text and relies on parsing/repair instead of failing the crawl step outright.
