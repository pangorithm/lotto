CREATE TABLE model_prediction (
    series_code TEXT NOT NULL CHECK (series_code IN ('lotto645', 'pension720')),
    target_round INTEGER NOT NULL CHECK (target_round = based_on_round + 1),
    target_date TEXT NOT NULL CHECK (date(target_date, '+0 days') IS target_date),
    based_on_round INTEGER NOT NULL,
    created_at_utc TEXT NOT NULL,
    algorithm TEXT NOT NULL CHECK (algorithm = 'recent-bias-v1'),
    ticket_json TEXT NOT NULL CHECK (json_type(ticket_json) IS 'object'),
    PRIMARY KEY (series_code, target_round),
    FOREIGN KEY (series_code, based_on_round) REFERENCES draw(series_code, round_no)
) STRICT, WITHOUT ROWID;

PRAGMA user_version = 4;
