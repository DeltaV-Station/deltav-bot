CREATE TABLE IF NOT EXISTS prfeeds (
    id         INTEGER PRIMARY KEY,
    gh_label   TEXT NOT NULL,
    channel_id INTEGER NOT NULL,
    ping_role  INTEGER
);
