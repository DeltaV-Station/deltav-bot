DROP TABLE cr_raised_issues_overrides;

CREATE TABLE IF NOT EXISTS cr_raised_issue_overrides (
    pr_id INTEGER NOT NULL,
    user_id INTEGER NOT NULL,
    message_id INTEGER NOT NULL,

    FOREIGN KEY(pr_id) REFERENCES cr_discussions(pr_id),
    PRIMARY KEY(pr_id, user_id)
);
