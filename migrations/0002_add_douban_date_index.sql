-- Reserved for future analytics tables.
CREATE INDEX IF NOT EXISTS idx_douban_items_date
    ON douban_items (date, id)
    WHERE date IS NOT NULL;
