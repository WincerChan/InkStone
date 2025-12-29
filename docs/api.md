# Inkstone API

Base URL: `http://127.0.0.1:8080`

API prefix: `/v2` (except `/health` and `/webhook/github/content`)


## Search

`GET /v2/search`

### Query parameters

- `q` (required): search query string (max 256 chars, cannot be empty)
- `limit` (optional): number of results to return (default: 8, max: `INKSTONE_MAX_SEARCH_LIMIT`)
- `offset` (optional): pagination offset (default: 0)
- `sort` (optional): `relevance` (default) or `latest` (order by updated date desc)

### Search query syntax

Tokens are separated by whitespace and may be combined:

- Keywords: `Python Linux` (match title, content, tags, category)
- Date range:
  - `range:2020-01-01~`
  - `range:~2020-01-01`
  - `range:2018-01-01~2020-01-01`
- Tags: `tags:Python,Linux`
- Category: `category:share`

Example:

```bash
curl "http://127.0.0.1:8080/v2/search?q=Python%20range:2020-01-01~%20tags:Rust"
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
  "elapsed_ms": 12,
  "hits": [
    {
      "id": "urn:uuid:...",
      "title": "<b>Example</b> title snippet",
      "subtitle": "<b>Example</b> subtitle snippet",
      "content": "<b>Example</b> content snippet",
      "url": "https://blog.example.com/posts/example",
      "tags": ["Rust", "Search"],
      "category": "share",
      "published_at": "2025-01-01T00:00:00Z",
      "updated_at": "2025-01-02T00:00:00Z",
      "matched": {
        "title": true,
        "subtitle": false,
        "content": true,
        "tags": ["Rust"],
        "category": false
      }
    }
  ]
}
```

Notes:
- `title`, `subtitle`, and `content` contain highlighted snippets for keyword queries; `content` may be null when正文为空。
- `matched.tags` lists exact tag matches from keywords or `tags:` filters.
- `matched.subtitle` indicates matches inside subtitle text.
- `matched` indicates which fields matched (snippet highlight + exact category match).

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

## Comments

`GET /v2/comments`

Query parameters:

- `post_id` (required): blog path, e.g. `/posts/hello-world/` or `/life/`

Response:

```json
{
  "post_id": "hello-world",
  "discussion_url": "https://github.com/owner/repo/discussions/12",
  "total": 2,
  "comments": [
    {
      "id": "DIC_kwDO...",
      "url": "https://github.com/owner/repo/discussions/12#discussioncomment-1",
      "author_login": "octocat",
      "author_url": "https://github.com/octocat",
      "author_avatar_url": "https://avatars.githubusercontent.com/u/583231?v=4",
      "body_html": "<p>First comment</p>",
      "created_at": "2025-01-01T00:00:00Z",
      "updated_at": "2025-01-01T00:00:00Z",
      "replies": [
        {
          "id": "DIC_kwDO...",
          "url": "https://github.com/owner/repo/discussions/12#discussioncomment-2",
          "author_login": "octocat",
          "author_url": "https://github.com/octocat",
          "author_avatar_url": "https://avatars.githubusercontent.com/u/583231?v=4",
          "body_html": "<p>Reply</p>",
          "created_at": "2025-01-01T00:01:00Z",
          "updated_at": "2025-01-01T00:01:00Z",
          "replies": []
        }
      ]
    }
  ]
}
```

Notes:
- `discussion_url` is null when the post has no discussion yet.
- `post_id` must start with `/` and cannot contain whitespace.
- Comments are ordered by `created_at` descending (newest first), including replies.
- Discussion titles are mapped to paths: `hello-world` -> `/posts/hello-world/`. For special pages, set the discussion title to the full path (e.g. `/life/`). Legacy titles like `posts/hello-world` are supported.
- TODO: comment reactions may be added later (pending evaluation to avoid overlap with kudos).

Error responses:

- `400 Bad Request`: missing/invalid `post_id`
- `503 Service Unavailable`: database not configured
- `500 Internal Server Error`: database error

Error body:

```json
{
  "error": "message"
}
```

## Kudos

`GET /v2/kudos`

Query parameters:

- `path` (required): blog path, e.g. `/posts/hello/`

Response:

```json
{
  "count": 12,
  "interacted": true
}
```

`PUT /v2/kudos`

Query parameters:

- `path` (required): blog path, e.g. `/posts/hello/`

Notes:

- The API sets/uses the `bid` cookie for idempotent kudos.
- `PUT /v2/kudos` requires a valid `bid` cookie; missing/invalid cookies return `401`.
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


## Douban marks (current year)

`GET /v2/douban/marks`

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
