ALTER TABLE comment_items
    ADD COLUMN IF NOT EXISTS author_avatar_url TEXT;
