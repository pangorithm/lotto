use anyhow::{Context, Result};
use chrono::{SecondsFormat, Utc};
use rusqlite::OptionalExtension;

use super::{Store, forecast::decode_model_tickets};
use crate::domain::{Game, Head, Notice, NoticeResult};

impl Store {
    pub(crate) fn pending_notice(&self, game: Game) -> Result<Option<Notice>> {
        // Keep the selected outbox row, result, and tickets in one read snapshot.
        let tx = self.db.unchecked_transaction()?;
        let code = game.code();
        let pending = tx
            .query_row(
                "SELECT d.round_no, d.draw_date, model_prior.ticket_json, model_prior.alternatives_json,
                        model_next.ticket_json, model_next.alternatives_json, model_next.algorithm
             FROM notice_outbox AS o
             JOIN draw AS d USING (series_code, round_no)
             LEFT JOIN model_prediction AS model_prior ON model_prior.series_code = o.series_code
                 AND model_prior.target_round = o.round_no
             LEFT JOIN model_prediction AS model_next ON model_next.series_code = o.series_code
                 AND model_next.target_round = o.round_no + 1
             WHERE o.series_code = ?1 AND o.sent_at_utc IS NULL
             ORDER BY o.round_no LIMIT 1",
                [code],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get::<_, String>(1)?,
                        [
                            (row.get(2)?, row.get(3)?),
                            (row.get(4)?, row.get(5)?),
                        ],
                        row.get(6)?,
                    ))
                },
            )
            .optional()?;
        let Some((round, date, [model_prior, model_next], model_next_algorithm)) = pending else {
            return Ok(None);
        };
        let head = Head {
            round,
            date: date.parse().context("invalid stored draw date")?,
        };
        let result = match game {
            Game::Lotto => tx.query_row(
                "SELECT n1, n2, n3, n4, n5, n6, bonus FROM lotto_result
                 WHERE series_code = ?1 AND round_no = ?2",
                (code, round),
                |r| Ok(NoticeResult::Lotto {
                    numbers: [r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?],
                    bonus: r.get(6)?,
                }),
            ),
            Game::Pension => tx.query_row(
                "SELECT first.group_no, first.digits, bonus.digits
                 FROM pension_number AS first JOIN pension_number AS bonus USING (series_code, round_no)
                 WHERE first.series_code = ?1 AND first.round_no = ?2
                   AND first.tier = '1' AND first.slot = 1 AND bonus.tier = 'bonus' AND bonus.slot = 1",
                (code, round),
                |r| Ok(NoticeResult::Pension {
                    group_no: r.get(0)?,
                    digits: r.get(1)?,
                    bonus: r.get(2)?,
                }),
            ),
        }.with_context(|| format!("read notice result for {code} round {round}"))?;
        let tickets = |(first, alternatives): (Option<String>, Option<String>)| {
            decode_model_tickets(first.as_deref(), alternatives.as_deref())
        };
        let notice = Notice {
            game,
            head,
            result,
            model_prior: tickets(model_prior)?,
            model_next: tickets(model_next)?,
            model_next_algorithm,
        };
        tx.commit()?;
        Ok(Some(notice))
    }

    pub(crate) fn mark_notice_sent(&mut self, game: Game, round: u32) -> Result<()> {
        let sent = Utc::now().to_rfc3339_opts(SecondsFormat::AutoSi, true);
        let code = game.code();
        self.db.execute(
            "UPDATE notice_outbox SET sent_at_utc = ?3
             WHERE series_code = ?1 AND round_no = ?2 AND sent_at_utc IS NULL",
            (code, round, sent),
        )?;
        Ok(())
    }
}

#[cfg(test)]
pub(super) mod tests {
    use chrono::{DateTime, Days};
    use rusqlite::Connection;
    use tempfile::NamedTempFile;

    use super::{Context, Game, Head, NoticeResult, Result, Store};
    use crate::domain::{Draw, LottoDraw, LottoPrize, Record};

    pub(in crate::store) fn archive() -> Result<NamedTempFile> {
        let file = NamedTempFile::new()?;
        let db = Connection::open(file.path())?;
        db.execute_batch(include_str!("../schema.sql"))?;
        db.execute_batch(
            "INSERT INTO series VALUES ('lotto645','Lotto','2002-12-07',NULL),
                                      ('pension720','Pension','2020-05-07',NULL);
             INSERT INTO source (id,url,publisher,title,kind,accessed_on)
             VALUES ('seed','https://example.org/','fixture','fixture','third_party','2020-01-01');
             INSERT INTO draw (series_code,round_no,draw_date,source_id,source_locator)
             VALUES ('lotto645',1,'2026-11-21','seed','seed'),
                    ('pension720',1,'2026-11-19','seed','seed');",
        )?;
        Ok(file)
    }

    pub(in crate::store) fn lotto_after(head: Head) -> Result<Record> {
        Ok(Record {
            head: Head {
                round: head.round + 1,
                date: head.date + Days::new(7),
            },
            draw: Draw::Lotto(LottoDraw {
                numbers: [1, 2, 3, 4, 5, 6],
                bonus: 7,
                sales_krw: 100,
                prizes: std::array::from_fn(|_| LottoPrize {
                    winners: 1,
                    unit_krw: 1,
                    total_krw: 1,
                }),
            }),
            source_url: "https://example.org/".parse()?,
        })
    }

    #[test]
    fn notices_follow_draw_rounds_and_keep_the_first_sent_marker() -> Result<()> {
        let file = archive()?;
        let path = file.path();
        let seed = path.with_extension("seed");
        let mut store = Store::open(path, &seed)?;
        let game = Game::Lotto;
        assert!(store.pending_notice(game)?.is_none());
        let head = store.head(game)?;
        let first = lotto_after(head)?;
        let second = lotto_after(first.head)?;
        let (first_head, second_head) = (first.head, second.head);
        store.apply(game, &[first, second])?;

        let mut store = Store::open(path, &seed)?;
        let notice = store.pending_notice(game)?.context("first notice")?;
        assert_eq!((notice.game, notice.head), (game, first_head));
        let NoticeResult::Lotto { numbers, bonus } = notice.result else {
            anyhow::bail!("expected a Lotto result");
        };
        assert_eq!((numbers, bonus), ([1, 2, 3, 4, 5, 6], 7));
        assert!(notice.model_prior.is_empty());
        assert!(notice.model_next.is_empty());
        assert!(notice.model_next_algorithm.is_none());
        let sent = |db: &Connection| {
            db.query_row(
                "SELECT sent_at_utc FROM notice_outbox WHERE series_code = ?1 AND round_no = ?2",
                (game.code(), first_head.round),
                |row| row.get::<_, String>(0),
            )
        };
        store.mark_notice_sent(game, first_head.round)?;
        let first_sent = sent(&store.db)?;
        let parsed = DateTime::parse_from_rfc3339(&first_sent)?;
        assert_eq!(parsed.offset().local_minus_utc(), 0);
        store.mark_notice_sent(game, first_head.round)?;
        assert_eq!(sent(&store.db)?, first_sent);
        let notice = store.pending_notice(game)?.context("second notice")?;
        assert_eq!(notice.head, second_head);
        store.mark_notice_sent(game, notice.head.round)?;
        assert!(store.pending_notice(game)?.is_none());
        Ok(())
    }
}
