CREATE TABLE IF NOT EXISTS douban_items (
    id TEXT NOT NULL,
    type TEXT NOT NULL,
    title TEXT NOT NULL,
    poster TEXT,
    rating SMALLINT,
    tags TEXT[] NOT NULL DEFAULT '{}',
    comment TEXT,
    date DATE,
    PRIMARY KEY (type, id),
    CHECK (rating IS NULL OR (rating >= 1 AND rating <= 5))
);
