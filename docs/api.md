# Inkstone API

Base URL: `http://127.0.0.1:8080`

## Health

`GET /health`

Response:

```text
ok
```

## Search

`GET /search`

### Query parameters

- `q` (required): search query string (max 256 chars, cannot be empty)
- `limit` (optional): number of results to return (default: 20, max: `INKSTONE_MAX_SEARCH_LIMIT`)
- `offset` (optional): pagination offset (default: 0)

### Search query syntax

Tokens are separated by whitespace and may be combined:

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

### Input limits

- Query string length (entire URL query): max 512 chars
- `q` length: max 256 chars, cannot be empty
- Max 10 keywords
- Control characters are rejected; whitespace is normalized

### Response

`200 OK`

```json
{
  "total": 1,
  "hits": [
    {
      "id": "urn:uuid:...",
      "title": "<b>Example</b> title snippet",
      "content": "<b>Example</b> content snippet",
      "url": "https://blog.example.com/posts/example",
      "tags": ["Rust", "Search"],
      "category": "share",
      "published_at": "2025-01-01T00:00:00Z",
      "updated_at": "2025-01-02T00:00:00Z"
    }
  ]
}
```

Notes:
- `title` and `content` contain highlighted snippets for keyword queries; `content` may be null when正文为空。

### Error responses

- `400 Bad Request`: invalid query syntax (e.g. invalid range), empty query, control characters, too many keywords, or `q` exceeds 256 chars
- `414 URI Too Long`: query string exceeds 512 chars
- `500 Internal Server Error`: search backend failure

Error body:

```json
{
  "error": "message"
}
```

## Kudos

`GET /kudos`

Query parameters:

- `path` (required): blog path, e.g. `/posts/hello/`

Response:

```json
{
  "count": 12,
  "interacted": true
}
```

`PUT /kudos`

Query parameters:

- `path` (required): blog path, e.g. `/posts/hello/`

Notes:

- The API sets/uses the `bid` cookie for idempotent kudos.
- `PUT /kudos` requires a valid `bid` cookie; missing/invalid cookies return `401`.
- `path` must exist in `valid_paths.txt`, otherwise `404` is returned.
- Kudos counts are served from in-memory cache; the worker flushes pending kudos to the database.

Error responses:

- `400 Bad Request`: missing/invalid path
- `404 Not Found`: path not in valid list
- `401 Unauthorized`: missing/invalid `bid` cookie
- `503 Service Unavailable`: valid paths not loaded, cookie secrets missing, or DB not configured
- `500 Internal Server Error`: database error

Error body:

```json
{
  "error": "message"
}
```

## Pulse (analytics)

`POST /pulse/pv`

Records a page view (without duration). The server sets a `bid` cookie if missing.

Request body:

```json
{
  "page_instance_id": "uuid",
  "path": "/posts/hello/"
}
```

Notes:

- `path` must exist in `valid_paths.txt`, otherwise `404` is returned.
- `ua_family`, `device`, `source_type`, `ref_host`, and `country` are derived from request headers.
- `country` uses `CF-IPCountry` if present; otherwise the first `X-Forwarded-For` IP.

`POST /pulse/engage`

Upserts engagement duration for the page instance.

Request body:

```json
{
  "page_instance_id": "uuid",
  "duration_ms": 120000
}
```

Responses:

- `204 No Content`: success
- `400 Bad Request`: missing/invalid fields
- `404 Not Found`: path not in valid list (pv only)
- `503 Service Unavailable`: valid paths not loaded, cookie secrets missing, or DB not configured
- `500 Internal Server Error`: database error

Error body:

```json
{
  "error": "message"
}
```

## GitHub webhook

`POST /webhook/github`

Handles GitHub `check_run` events and refreshes `atom.xml` + `valid_paths.txt` on successful
completed checks. `ping` events return `204`. Requires `INKSTONE_GITHUB_WEBHOOK_SECRET`.
Refresh is queued asynchronously; failures enter a 60-second per-task backoff.

Headers:

- `X-GitHub-Event` (required)
- `X-Hub-Signature-256` (required, `sha256=<hex>`)

Responses:

- `204 No Content`: `ping` handled
- `202 Accepted`: event ignored or refresh queued
- `400 Bad Request`: missing headers or invalid payload
- `401 Unauthorized`: invalid signature
- `503 Service Unavailable`: webhook secret not configured

## Douban marks (current year)

`GET /douban/marks`

Returns this year's Douban marks ordered by date ascending.

### Response

`200 OK`

```json
{
  "total": 2,
  "items": [
    {
      "title": "Example Title",
      "poster": "https://img.example.com/poster.jpg",
      "type": "movie",
      "date": "2025-12-04",
      "url": "https://movie.douban.com/subject/1234567/"
    }
  ]
}
```

### Error responses

- `503 Service Unavailable`: database not configured
- `500 Internal Server Error`: database or data error

Error body:

```json
{
  "error": "message"
}
```
