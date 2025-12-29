CREATE TABLE IF NOT EXISTS comment_discussions (
    post_id TEXT PRIMARY KEY,
    discussion_id TEXT NOT NULL UNIQUE,
    number INTEGER NOT NULL,
    title TEXT NOT NULL,
    url TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_comment_discussions_updated_at
    ON comment_discussions (updated_at);

CREATE TABLE IF NOT EXISTS comment_items (
    discussion_id TEXT NOT NULL,
    comment_id TEXT NOT NULL,
    parent_id TEXT,
    author_login TEXT,
    author_url TEXT,
    body_html TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (discussion_id, comment_id)
);

CREATE INDEX IF NOT EXISTS idx_comment_items_discussion
    ON comment_items (discussion_id, created_at);

CREATE INDEX IF NOT EXISTS idx_comment_items_parent
    ON comment_items (discussion_id, parent_id);
