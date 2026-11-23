CREATE TEMP TABLE assert_snapshot (
    label TEXT PRIMARY KEY,
    valid INTEGER NOT NULL CHECK (valid = 1)
) STRICT;

INSERT INTO assert_snapshot VALUES
('integrity', (SELECT integrity_check = 'ok' FROM pragma_integrity_check)),
('foreign keys', NOT EXISTS (SELECT 1 FROM pragma_foreign_key_check)),
('lotto coverage', (SELECT count(*) = 1251 AND min(round_no) = 1 AND max(round_no) = 1251
 AND min(draw_date) = '2002-12-07' AND max(draw_date) = '2026-11-21' FROM v_lotto_draws)),
('lotto tiers', (SELECT count(*) = 6255 AND min(tier) = '1' AND max(tier) = '5'
 FROM prize WHERE series_code = 'lotto645')),
('pension520 coverage', (SELECT count(*) = 461 AND min(round_no) = 1 AND max(round_no) = 461
 AND min(draw_date) = '2011-07-06' AND max(draw_date) = '2020-04-29' FROM draw WHERE series_code = 'pension520')),
('pension520 tier shape', NOT EXISTS (
 SELECT round_no FROM pension_number WHERE series_code = 'pension520' GROUP BY round_no
 HAVING count(*) <> 13 OR count(DISTINCT tier) <> 7 OR sum(tier = '1') <> 2 OR sum(tier = '2') <> 4
 OR sum(tier = '6') <> 2 OR sum(tier = '7') <> 2
)),
('pension520 patterns', (SELECT count(*) = 5993 FROM pension_number WHERE series_code = 'pension520')),
('pension520 fifth prize boundary', NOT EXISTS (
 SELECT 1 FROM pension_number n JOIN prize p USING (series_code, round_no, tier)
 WHERE n.series_code = 'pension520' AND n.tier = '5'
 AND (length(n.digits) <> CASE WHEN n.round_no < 66 THEN 4 ELSE 3 END
      OR p.unit_krw <> CASE WHEN n.round_no < 66 THEN 200000 ELSE 20000 END)
)),
('pension720 coverage', (SELECT count(*) = 342 AND min(round_no) = 1 AND max(round_no) = 342
 AND min(draw_date) = '2020-05-07' AND max(draw_date) = '2026-11-19' FROM draw WHERE series_code = 'pension720')),
('pension720 patterns', (SELECT count(*) = 2736 FROM pension_number WHERE series_code = 'pension720')),
('pension720 suffixes', NOT EXISTS (
 SELECT 1 FROM pension_number n JOIN pension_number first USING (series_code, round_no)
 WHERE n.series_code = 'pension720' AND first.tier = '1' AND n.tier NOT IN ('1','bonus')
 AND substr(first.digits, -length(n.digits)) <> n.digits
)),
('prize on each pension tier', NOT EXISTS (
 SELECT 1 FROM pension_number n LEFT JOIN prize p USING (series_code, round_no, tier) WHERE p.tier IS NULL
)),
('events with scope and evidence', NOT EXISTS (
 SELECT 1 FROM event e WHERE NOT EXISTS (SELECT 1 FROM event_scope WHERE event_id = e.id)
 OR NOT EXISTS (SELECT 1 FROM event_evidence WHERE event_id = e.id)
));
