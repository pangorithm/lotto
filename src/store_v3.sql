CREATE TABLE notice_outbox (
    series_code TEXT NOT NULL CHECK (series_code IN ('lotto645', 'pension720')),
    round_no INTEGER NOT NULL,
    sent_at_utc TEXT,
    PRIMARY KEY (series_code, round_no),
    FOREIGN KEY (series_code, round_no) REFERENCES draw(series_code, round_no)
) STRICT, WITHOUT ROWID;

CREATE INDEX notice_outbox_pending ON notice_outbox(series_code, round_no)
    WHERE sent_at_utc IS NULL;

PRAGMA user_version = 3;
