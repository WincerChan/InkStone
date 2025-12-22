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

All configuration is driven by environment variables. You can also place them in a
`.env` file; existing environment variables take precedence.

- `INKSTONE_HTTP_ADDR` (default: `127.0.0.1:8080`)
- `INKSTONE_INDEX_DIR` (default: `./data/index`)
- `INKSTONE_FEED_URL` (default: `https://velite-refactor.blog-8fo.pages.dev/atom.xml`)
- `INKSTONE_POLL_INTERVAL_SECS` (default: `300`)
- `INKSTONE_DOUBAN_POLL_INTERVAL_SECS` (default: `INKSTONE_POLL_INTERVAL_SECS`)
- `INKSTONE_REQUEST_TIMEOUT_SECS` (default: `15`)
- `INKSTONE_MAX_SEARCH_LIMIT` (default: `50`)
- `INKSTONE_DOUBAN_MAX_PAGES` (default: `1`, set `0` to disable limit)
- `INKSTONE_DOUBAN_UID` (default: `93562087`)
- `INKSTONE_DOUBAN_COOKIE` (default: `bid=3EHqn8aRvcI`)
- `INKSTONE_DOUBAN_USER_AGENT` (default: `Mozilla/5.0 ...`)

Example:

```bash
INKSTONE_HTTP_ADDR=0.0.0.0:8080 \
INKSTONE_INDEX_DIR=./data/index \
INKSTONE_POLL_INTERVAL_SECS=120 \
INKSTONE_DOUBAN_POLL_INTERVAL_SECS=3600 \
INKSTONE_MAX_SEARCH_LIMIT=100 \
INKSTONE_DOUBAN_MAX_PAGES=1 \
INKSTONE_DOUBAN_UID=93562087 \
INKSTONE_DOUBAN_COOKIE=bid=3EHqn8aRvcI \
INKSTONE_DOUBAN_USER_AGENT="Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0 Safari/537.36" \
cargo run -p inkstone-app
```

Example `.env`:

```bash
INKSTONE_HTTP_ADDR=127.0.0.1:8080
INKSTONE_INDEX_DIR=./data/index
INKSTONE_POLL_INTERVAL_SECS=120
INKSTONE_DOUBAN_POLL_INTERVAL_SECS=3600
INKSTONE_MAX_SEARCH_LIMIT=100
INKSTONE_DOUBAN_MAX_PAGES=1
INKSTONE_DOUBAN_UID=93562087
INKSTONE_DOUBAN_COOKIE=bid=3EHqn8aRvcI
INKSTONE_DOUBAN_USER_AGENT="Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0 Safari/537.36"
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

Query limits:

- Query string length (entire URL query): max 512 chars
- `q` length: max 256 chars, cannot be empty
- Max 10 keywords
- Control characters are rejected; whitespace is normalized

## API documentation

See `docs/api.md`.
