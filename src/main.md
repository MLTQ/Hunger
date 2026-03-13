# main.rs

## Purpose
Bootstraps the Hunger desktop app by loading environment configuration, initializing tracing, and handing control to the application runtime. It exists to keep process setup separate from crawler and UI behavior.

## Components

### `main`
- **Does**: Configures logging, loads `Config`, and starts the synchronous desktop runtime entrypoint.
- **Interacts with**: `Config::from_env` in `config.rs`, `run` in `app.rs`

## Contracts

| Dependent | Expects | Breaking changes |
|-----------|---------|------------------|
| Cargo binary target | `main` returns an `anyhow::Result<()>` and starts the desktop app | Changing startup flow or panic behavior |
