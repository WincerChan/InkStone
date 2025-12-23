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
