ALTER TABLE model_prediction ADD COLUMN alternatives_json TEXT
    CHECK (alternatives_json IS NULL OR (
        json_type(alternatives_json) IS 'array' AND json_array_length(alternatives_json) = 4
    ));

PRAGMA user_version = 5;
