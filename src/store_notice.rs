use anyhow::{Context, Result, ensure};
use chrono::{SecondsFormat, Utc};
use rusqlite::{OptionalExtension, TransactionBehavior};

use super::{Store, read_head};
use crate::domain::{Candidate, Game, Head, Notice, NoticeResult, Ticket};

const ALGORITHM: &str = "uniform-v1";

impl Store {
    pub(crate) fn pending_notice(&self, game: Game) -> Result<Option<Notice>> {
        // Keep the selected outbox row, result, and tickets in one read snapshot.
        let tx = self.db.unchecked_transaction()?;
        let code = game.code();
        let pending = tx
            .query_row(
                "SELECT d.round_no, d.draw_date, prior.ticket_json, next.ticket_json
             FROM notice_outbox AS o
             JOIN draw AS d USING (series_code, round_no)
             LEFT JOIN prediction AS prior ON prior.series_code = o.series_code
                 AND prior.target_round = o.round_no
             LEFT JOIN prediction AS next ON next.series_code = o.series_code
                 AND next.target_round = o.round_no + 1
             WHERE o.series_code = ?1 AND o.sent_at_utc IS NULL
             ORDER BY o.round_no LIMIT 1",
                [code],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get::<_, String>(1)?,
                        row.get(2)?,
                        row.get(3)?,
                    ))
                },
            )
            .optional()?;
        let Some((round, date, prior, next)) = pending else {
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
        let ticket = |json: Option<String>| -> Result<Option<Ticket>> {
            json.as_deref()
                .map(serde_json::from_str)
                .transpose()
                .context("invalid stored notice ticket")
        };
        let prior = ticket(prior)?;
        let next = ticket(next)?;
        tx.commit()?;
        Ok(Some(Notice {
            game,
            head,
            result,
            prior,
            next,
        }))
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

    pub(crate) fn save_candidate(&mut self, candidate: &Candidate) -> Result<bool> {
        let db = &mut self.db;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let latest = read_head(&tx, candidate.game)?;
        let target = candidate.target;
        // The head is the greatest stored round, so its successor has no draw in this transaction.
        if latest != candidate.based_on
            || latest.round.checked_add(1) != Some(target.round)
            || target.date <= latest.date
        {
            return Ok(false);
        }
        let ticket_game = match &candidate.ticket {
            Ticket::Lotto { .. } => Game::Lotto,
            Ticket::Pension { .. } => Game::Pension,
        };
        ensure!(ticket_game == candidate.game, "ticket game mismatch");
        let created_at = candidate.created_at;
        let inserted = tx.execute(
            "INSERT INTO prediction (series_code, target_round, target_date, based_on_round,
                                     created_at_utc, algorithm, ticket_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT (series_code, target_round) DO NOTHING",
            (
                candidate.game.code(),
                target.round,
                target.date.to_string(),
                latest.round,
                created_at.to_rfc3339_opts(SecondsFormat::AutoSi, true),
                ALGORITHM,
                serde_json::to_string(&candidate.ticket)?,
            ),
        )?;
        tx.commit().context("commit candidate")?;
        Ok(inserted == 1)
    }
}

#[cfg(test)]
pub(super) mod tests {
    use chrono::{DateTime, Days};
    use rusqlite::Connection;
    use tempfile::NamedTempFile;

    use super::{Candidate, Context, Game, Head, NoticeResult, Result, Store, Ticket, Utc};
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
    fn status_and_migration_preserve_archive_and_v2_predictions() -> Result<()> {
        use crate::store::schema_version;
        let file = archive()?;
        let path = file.path();
        let db = Connection::open(path)?;
        assert!(Store::status(path)?["lotto645"]["candidate"].is_null());
        assert_eq!(schema_version(&db)?, 1);
        db.execute_batch(include_str!("store_v2.sql"))?;
        db.execute_batch(
            r#"INSERT INTO prediction VALUES ('lotto645',2,'2099-11-28',1,'2026-11-22T00:00:00Z',
             'uniform-v1','{"numbers":[8,9,10,11,12,13]}');"#,
        )?;
        let status = Store::status(path)?;
        assert_eq!(status["lotto645"]["candidate"]["target_round"], 2);
        assert_eq!(schema_version(&db)?, 2);

        let store = Store::open(path, &path.with_extension("seed"))?;
        assert_eq!(schema_version(&db)?, 3);
        assert_eq!(Store::status(path)?, status);
        for game in Game::ALL {
            assert!(store.pending_notice(game)?.is_none());
        }
        Ok(())
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
        let candidate = |based_on, target, numbers| Candidate {
            game,
            based_on,
            target,
            ticket: Ticket::Lotto { numbers },
            created_at: Utc::now(),
        };
        let prior = candidate(head, first.head, [8, 9, 10, 11, 12, 13]);
        assert!(store.save_candidate(&prior)?);
        store.apply(game, std::slice::from_ref(&first))?;
        let next = candidate(first.head, second.head, [14, 15, 16, 17, 18, 19]);
        assert!(store.save_candidate(&next)?);
        let first_head = first.head;
        store.apply(game, &[first, second])?;
        assert!(!store.save_candidate(&next)?);

        let mut store = Store::open(path, &seed)?;
        let notice = store.pending_notice(game)?.context("first notice")?;
        assert_eq!((notice.game, notice.head), (game, first_head));
        let NoticeResult::Lotto { numbers, bonus } = notice.result else {
            anyhow::bail!("expected a Lotto result");
        };
        assert_eq!((numbers, bonus), ([1, 2, 3, 4, 5, 6], 7));
        let numbers = |ticket| match ticket {
            Some(Ticket::Lotto { numbers }) => Some(numbers),
            Some(Ticket::Pension { .. }) | None => None,
        };
        assert_eq!(numbers(notice.prior), Some([8, 9, 10, 11, 12, 13]));
        assert_eq!(numbers(notice.next), Some([14, 15, 16, 17, 18, 19]));
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
        assert_eq!(notice.head, next.target);
        assert!(notice.next.is_none());
        store.mark_notice_sent(game, notice.head.round)?;
        assert!(store.pending_notice(game)?.is_none());
        Ok(())
    }
}
