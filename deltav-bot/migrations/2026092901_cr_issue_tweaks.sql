CREATE TABLE IF NOT EXISTS cr_raised_issues_new (
    message_id INTEGER PRIMARY KEY NOT NULL,
    pr_id INTEGER NOT NULL,
    user_id INTEGER NOT NULL,

    FOREIGN KEY(pr_id) REFERENCES cr_discussions(pr_id)
);
CREATE INDEX idx_cr_raised_issues_pr_id ON cr_raised_issues_new(pr_id);

INSERT INTO cr_raised_issues_new(message_id, pr_id, user_id) SELECT message_id, pr_id, user_id FROM cr_raised_issues;
DROP TABLE cr_raised_issues;
ALTER TABLE cr_raised_issues_new RENAME TO cr_raised_issues;

CREATE TABLE cr_raised_issue_overrides_new (
    pr_id INTEGER NOT NULL,
    user_id INTEGER NOT NULL,
    message_id INTEGER NOT NULL,
    related_issue INTEGER,

    FOREIGN KEY(related_issue) REFERENCES cr_raised_issues(message_id),
    FOREIGN KEY(pr_id) REFERENCES cr_discussions(pr_id),
    PRIMARY KEY(message_id)
);
CREATE INDEX idx_cr_raised_issue_overrides_pr_id ON cr_raised_issue_overrides_new(pr_id);

INSERT INTO cr_raised_issue_overrides_new(message_id, pr_id, user_id) SELECT message_id, pr_id, user_id FROM cr_raised_issue_overrides;
DROP TABLE cr_raised_issue_overrides;
ALTER TABLE cr_raised_issue_overrides_new RENAME TO cr_raised_issue_overrides;
