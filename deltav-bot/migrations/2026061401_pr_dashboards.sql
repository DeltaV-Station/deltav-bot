ALTER TABLE prfeeds RENAME TO pr_dashboards;

CREATE TABLE IF NOT EXISTS pr_dashboard_messages (
    message_id INTEGER PRIMARY KEY,
    pr_id INTEGER NOT NULL,
    dashboard_id INTEGER NOT NULL,

    FOREIGN KEY(dashboard_id) REFERENCES pr_dashboards(id)
);

CREATE INDEX idx_pr_dashboard_messages_pr_id ON pr_dashboard_messages(pr_id);

CREATE TABLE IF NOT EXISTS pr_dashboard_pending_drafts (
    id INTEGER PRIMARY KEY,
    pr_id INTEGER NOT NULL,
    dashboard_id INTEGER NOT NULL,

    FOREIGN KEY(dashboard_id) REFERENCES pr_dashboards(id)
);

CREATE INDEX idx_pr_dashboard_pending_drafts_pr_id ON pr_dashboard_pending_drafts(pr_id);
