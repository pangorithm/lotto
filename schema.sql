PRAGMA foreign_keys = ON;
PRAGMA user_version = 1;

CREATE TABLE series (
    code TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    first_draw_date TEXT NOT NULL,
    last_draw_date TEXT
) STRICT, WITHOUT ROWID;

CREATE TABLE source (
    id TEXT PRIMARY KEY,
    url TEXT NOT NULL,
    publisher TEXT NOT NULL,
    title TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('official', 'news', 'third_party')),
    accessed_on TEXT NOT NULL,
    artifact TEXT,
    sha256 TEXT CHECK (sha256 IS NULL OR length(sha256) = 64)
) STRICT, WITHOUT ROWID;

CREATE TABLE draw (
    series_code TEXT NOT NULL REFERENCES series(code),
    round_no INTEGER NOT NULL CHECK (round_no > 0),
    draw_date TEXT NOT NULL CHECK (date(draw_date, '+0 days') IS draw_date),
    source_id TEXT NOT NULL REFERENCES source(id),
    source_locator TEXT NOT NULL,
    total_sales_krw INTEGER CHECK (total_sales_krw >= 0),
    PRIMARY KEY (series_code, round_no),
    UNIQUE (series_code, draw_date)
) STRICT, WITHOUT ROWID;

CREATE TABLE lotto_result (
    series_code TEXT NOT NULL DEFAULT 'lotto645' CHECK (series_code = 'lotto645'),
    round_no INTEGER PRIMARY KEY,
    n1 INTEGER NOT NULL,
    n2 INTEGER NOT NULL,
    n3 INTEGER NOT NULL,
    n4 INTEGER NOT NULL,
    n5 INTEGER NOT NULL,
    n6 INTEGER NOT NULL,
    bonus INTEGER NOT NULL CHECK (bonus BETWEEN 1 AND 45 AND bonus NOT IN (n1,n2,n3,n4,n5,n6)),
    CHECK (1 <= n1 AND n1 < n2 AND n2 < n3 AND n3 < n4 AND n4 < n5 AND n5 < n6 AND n6 <= 45),
    FOREIGN KEY (series_code, round_no) REFERENCES draw(series_code, round_no)
) STRICT;

CREATE TABLE pension_number (
    series_code TEXT NOT NULL CHECK (series_code IN ('pension520', 'pension720')),
    round_no INTEGER NOT NULL,
    tier TEXT NOT NULL,
    slot INTEGER NOT NULL CHECK (slot > 0),
    group_no INTEGER CHECK (group_no BETWEEN 1 AND 7),
    digits TEXT NOT NULL CHECK (length(digits) BETWEEN 1 AND 6 AND digits NOT GLOB '*[^0-9]*'),
    CHECK (series_code <> 'pension720' OR (slot = 1 AND (
        (tier = '1' AND length(digits) = 6 AND group_no IS NOT NULL AND group_no <= 5) OR
        (tier IN ('2','3','4','5','6','7','bonus') AND group_no IS NULL AND
            length(digits) = CASE WHEN tier = 'bonus' THEN 6 ELSE 8 - CAST(tier AS INTEGER) END)
    ))),
    PRIMARY KEY (series_code, round_no, tier, slot),
    FOREIGN KEY (series_code, round_no) REFERENCES draw(series_code, round_no)
) STRICT, WITHOUT ROWID;

CREATE TABLE prize (
    series_code TEXT NOT NULL,
    round_no INTEGER NOT NULL,
    tier TEXT NOT NULL,
    winning_games INTEGER CHECK (winning_games >= 0),
    unit_krw INTEGER CHECK (unit_krw >= 0),
    payment_period TEXT CHECK (payment_period IN ('once', 'monthly')),
    payment_months INTEGER CHECK (payment_months > 0),
    total_krw INTEGER CHECK (total_krw >= 0),
    CHECK ((payment_period = 'monthly' AND payment_months IS NOT NULL) OR
           (payment_period IS NOT 'monthly' AND payment_months IS NULL)),
    PRIMARY KEY (series_code, round_no, tier),
    FOREIGN KEY (series_code, round_no) REFERENCES draw(series_code, round_no)
) STRICT, WITHOUT ROWID;

CREATE TABLE event (
    id TEXT PRIMARY KEY,
    category TEXT NOT NULL CHECK (category IN ('equipment','broadcast','operator','rules','oversight','publication')),
    effect TEXT NOT NULL CHECK (effect IN ('mechanism','operations','observation','announcement')),
    status TEXT NOT NULL CHECK (status IN ('confirmed','reported','unresolved')),
    title TEXT NOT NULL,
    description TEXT NOT NULL,
    announced_on TEXT CHECK (date(announced_on, '+0 days') IS announced_on),
    date_from TEXT CHECK (date(date_from, '+0 days') IS date_from),
    date_to TEXT CHECK (date(date_to, '+0 days') IS date_to),
    date_precision TEXT NOT NULL CHECK (date_precision IN ('day','month','year','range','unknown')),
    CHECK ((date_precision = 'unknown' AND date_from IS NULL AND date_to IS NULL) OR
           (date_precision = 'day' AND date_from IS NOT NULL AND date_to IS date_from) OR
           (date_precision IN ('month','year','range') AND date_from IS NOT NULL AND date_to IS NOT NULL AND date_from < date_to))
) STRICT, WITHOUT ROWID;

CREATE TABLE event_scope (
    event_id TEXT NOT NULL REFERENCES event(id),
    series_code TEXT NOT NULL REFERENCES series(code),
    first_round INTEGER,
    round_role TEXT CHECK (round_role IN ('first_affected', 'observed_in')),
    CHECK ((first_round IS NULL AND round_role IS NULL) OR
           (first_round IS NOT NULL AND first_round > 0 AND round_role IS NOT NULL)),
    PRIMARY KEY (event_id, series_code),
    FOREIGN KEY (series_code, first_round) REFERENCES draw(series_code, round_no)
) STRICT, WITHOUT ROWID;

CREATE TABLE event_evidence (
    event_id TEXT NOT NULL REFERENCES event(id),
    source_id TEXT NOT NULL REFERENCES source(id),
    anchor TEXT NOT NULL,
    PRIMARY KEY (event_id, source_id)
) STRICT, WITHOUT ROWID;

CREATE VIEW v_lotto_draws AS
SELECT d.round_no, d.draw_date, l.n1, l.n2, l.n3, l.n4, l.n5, l.n6, l.bonus,
       d.total_sales_krw, d.source_id, d.source_locator
FROM draw AS d JOIN lotto_result AS l USING (series_code, round_no);

CREATE VIEW v_pension_draws AS
SELECT d.series_code, d.round_no, d.draw_date, n.tier, n.slot, n.group_no, n.digits,
       d.source_id, d.source_locator
FROM draw AS d JOIN pension_number AS n USING (series_code, round_no);

CREATE VIEW v_event_timeline AS
SELECT s.series_code, s.first_round, s.round_role, e.*
FROM event AS e JOIN event_scope AS s ON s.event_id = e.id;
