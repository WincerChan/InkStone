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

- `q` (optional): search query string
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

- `400 Bad Request`: invalid query syntax (e.g. invalid range)
- `500 Internal Server Error`: search backend failure

Error body:

```json
{
  "error": "message"
}
```
