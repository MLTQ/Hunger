# lib.rs

## Purpose
Exposes the crate modules that make up the crawler runtime, storage layer, scoring pipeline, persisted settings layer, native desktop UI, and graph viewport. It keeps `main.rs` thin and makes the system easier to test as components.

## Components

### Module exports
- **Does**: Re-exports the application modules used by the binary entrypoint.
- **Interacts with**: `main` in `main.rs`

## Contracts

| Dependent | Expects | Breaking changes |
|-----------|---------|------------------|
| `main.rs` | `app::run` and the supporting modules are publicly reachable | Renaming or hiding modules |
