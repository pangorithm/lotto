CREATE TABLE model_prediction_v6 (
    series_code TEXT NOT NULL CHECK (series_code IN ('lotto645', 'pension720')),
    target_round INTEGER NOT NULL CHECK (target_round = based_on_round + 1),
    target_date TEXT NOT NULL CHECK (date(target_date, '+0 days') IS target_date),
    based_on_round INTEGER NOT NULL,
    created_at_utc TEXT NOT NULL,
    algorithm TEXT NOT NULL CHECK (algorithm IN ('recent-bias-v1', 'jackpot-joint-v2')),
    ticket_json TEXT NOT NULL CHECK (json_type(ticket_json) IS 'object'),
    alternatives_json TEXT CHECK (alternatives_json IS NULL OR (
        json_type(alternatives_json) IS 'array' AND json_array_length(alternatives_json) = 4
    )),
    PRIMARY KEY (series_code, target_round),
    FOREIGN KEY (series_code, based_on_round) REFERENCES draw(series_code, round_no)
) STRICT, WITHOUT ROWID;

INSERT INTO model_prediction_v6 (
    series_code, target_round, target_date, based_on_round, created_at_utc, algorithm,
    ticket_json, alternatives_json
)
SELECT series_code, target_round, target_date, based_on_round, created_at_utc, algorithm,
       ticket_json, alternatives_json
FROM model_prediction;

DROP TABLE model_prediction;
ALTER TABLE model_prediction_v6 RENAME TO model_prediction;

PRAGMA user_version = 6;
