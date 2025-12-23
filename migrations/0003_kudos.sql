CREATE TABLE IF NOT EXISTS kudos (
    path TEXT NOT NULL,
    interaction_id BYTEA NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (path, interaction_id)
);
