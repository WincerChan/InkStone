# Inkstone

Inkstone is a Rust workspace that combines an HTTP search API with a scheduled worker. The worker pulls an Atom feed, builds a Tantivy index, and keeps it up to date via incremental updates.

## Workspace layout

- `crates/app`: binary for HTTP API + worker scheduler
- `crates/core`: domain types and errors
- `crates/infra`: infrastructure adapters (Tantivy, etc.)

## Prerequisites

- Rust 1.78+ (edition 2024)

## Quick start

Build the workspace:

```bash
cargo build
```

Run API only:

```bash
cargo run -p inkstone-app -- --mode api
```

Run worker only:

```bash
cargo run -p inkstone-app -- --mode worker
```

Run API + worker (default):

```bash
cargo run -p inkstone-app
```

Rebuild the search index on startup:

```bash
cargo run -p inkstone-app -- --rebuild
```

## Configuration

All configuration is driven by environment variables:

- `INKSTONE_HTTP_ADDR` (default: `127.0.0.1:8080`)
- `INKSTONE_INDEX_DIR` (default: `./data/index`)
- `INKSTONE_FEED_URL` (default: `https://velite-refactor.blog-8fo.pages.dev/atom.xml`)
- `INKSTONE_POLL_INTERVAL_SECS` (default: `300`)
- `INKSTONE_REQUEST_TIMEOUT_SECS` (default: `15`)
- `INKSTONE_MAX_SEARCH_LIMIT` (default: `50`)

Example:

```bash
INKSTONE_HTTP_ADDR=0.0.0.0:8080 \
INKSTONE_INDEX_DIR=./data/index \
INKSTONE_POLL_INTERVAL_SECS=120 \
INKSTONE_MAX_SEARCH_LIMIT=100 \
cargo run -p inkstone-app
```

## Search query format

Queries are space-separated tokens and can be combined in any order:

- Keywords: `Python Linux`
- Date range:
  - `range:2020-01-01~`
  - `range:~2020-01-01`
  - `range:2018-01-01~2020-01-01`
- Tags: `tags:Python,Linux`
- Category: `category:share`

Example:

```bash
curl "http://127.0.0.1:8080/search?q=Python%20range:2020-01-01~%20tags:Rust"
```

## API documentation

See `docs/api.md`.
