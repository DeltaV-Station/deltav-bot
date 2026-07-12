CREATE TABLE IF NOT EXISTS cr_raised_issues (
    pr_id INTEGER NOT NULL,
    user_id INTEGER NOT NULL,
    message_id INTEGER NOT NULL,

    FOREIGN KEY(pr_id) REFERENCES cr_discussions(pr_id),
    PRIMARY KEY(pr_id, user_id)
);

CREATE TABLE IF NOT EXISTS cr_raised_issues_overrides (
    pr_id INTEGER NOT NULL,
    issue_user_id INTEGER NOT NULL,
    override_user_id INTEGER NOT NULL,
    override_message_id INTEGER NOT NULL,

    FOREIGN KEY(pr_id, issue_user_id) REFERENCES cr_raised_issues(pr_id, user_id),
    PRIMARY KEY(pr_id, issue_user_id, override_user_id)
);

CREATE INDEX idx_cr_discussions_thread ON cr_discussions(thread_id);
