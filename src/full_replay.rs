use std::{fs, path::Path};

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Days, NaiveDate, Utc};
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde_json::json;

use crate::collector::Collector;
use crate::domain::{Game, Head, Ticket, seoul_date};
use crate::replay_feed::ReplayFeed;
use crate::service::Service;
use crate::slack::Slack;
use crate::store::Store;

const WARMUP_END: u32 = 26;
const READ_ONLY: OpenFlags = OpenFlags::SQLITE_OPEN_READ_ONLY;

#[tokio::test(flavor = "multi_thread")]
#[ignore = "full archive evaluation"]
async fn all_active_rounds() -> Result<()> {
    let archive_uri = concat!(
        "file:",
        env!("CARGO_MANIFEST_DIR"),
        "/lottery.sqlite3?mode=ro&immutable=1"
    );
    let archive = Connection::open_with_flags(archive_uri, READ_ONLY | OpenFlags::SQLITE_OPEN_URI)?;
    let feed = ReplayFeed::start().await?;
    feed.reveal(0, 0);
    let dir = tempfile::tempdir()?;
    let webhook = dir.path().join("webhook");
    fs::write(
        &webhook,
        feed.url().join("/services/replay/test/token")?.as_str(),
    )?;
    let mut expected_notices = Vec::new();

    for (game, last, slack_name) in [
        (Game::Lotto, 1251, "로또 6/45"),
        (Game::Pension, 342, "연금복권 720+"),
    ] {
        let series = game.code();
        let runtime = dir.path().join(format!("{series}.sqlite3"));
        seed_first_round(&runtime, archive_uri, game)?;
        let mut service = Service {
            strategy: match std::env::var("LOTTERY_REPLAY_STRATEGY") {
                Ok(name) => crate::jackpot::Strategy::from_name(&name)?,
                Err(std::env::VarError::NotPresent) => crate::jackpot::Strategy::Recent,
                Err(error) => return Err(error.into()),
            },
            store: Store::open(&runtime, &dir.path().join("unused-seed.sqlite3"))?,
            collector: Collector::new(feed.url().clone())?,
            slack: Some(Slack::new(&webhook)?),
        };
        let db = Connection::open_with_flags(&runtime, READ_ONLY)?;
        let draw_count = |db: &Connection| -> rusqlite::Result<u32> {
            db.query_row("SELECT count(*) FROM draw", [], |r| r.get(0))
        };
        assert_eq!(
            draw_count(&db)?,
            1,
            "{series}: seed must contain only round 1"
        );
        assert!(Forecast::read(&db, game, 1)?.is_none());
        emit(game, 1, &[]);

        for round in 2..=last {
            let case = format!("{series} round {round}");
            assert_eq!(
                service.store.head(game)?.round,
                round - 1,
                "{case}: before release"
            );
            let prior = Forecast::read(&db, game, round)?;
            let date: String = archive.query_row(
                "SELECT draw_date FROM draw WHERE series_code=?1 AND round_no=?2",
                (series, round),
                |r| r.get(0),
            )?;
            let date: NaiveDate = date.parse()?;
            if let Some(prior) = &prior {
                assert!(
                    seoul_date(prior.created)? < date,
                    "{case}: forecast on or after draw day"
                );
            }

            let now = date
                .and_hms_opt(13, 0, 0)
                .context("22:00 KST draw time")?
                .and_utc();
            match game {
                Game::Lotto => feed.reveal(round, 0),
                Game::Pension => feed.reveal(0, round),
            }
            service
                .poll_game(game, &|| now)
                .await
                .with_context(|| format!("{case}: poll"))?;
            service
                .deliver_notices(game)
                .await
                .with_context(|| format!("{case}: Slack delivery"))?;
            expected_notices.push(format!("{slack_name} {round}회 ({date})"));

            assert_eq!(
                service.store.head(game)?,
                Head { round, date },
                "{case}: imported draw"
            );
            let official = Numbers::read(&archive, game, round)?;
            assert_eq!(
                Numbers::read(&db, game, round)?,
                official,
                "{case}: imported numbers"
            );
            assert_eq!(
                Forecast::read(&db, game, round)?,
                prior,
                "{case}: forecast changed after release"
            );
            if let Some(next) = Forecast::read(&db, game, round + 1)? {
                assert_eq!(next.created, now, "{case}: next forecast creation");
                assert_eq!(
                    next.target,
                    date + Days::new(7),
                    "{case}: next forecast date"
                );
            }

            let mut ranks = Vec::new();
            if let Some(prior) = &prior {
                for ticket in &prior.tickets {
                    ranks.push(official.rank(ticket).with_context(|| case.clone())?);
                }
            }
            emit(game, round, &ranks);
        }

        assert_eq!(
            draw_count(&db)?,
            last,
            "{series}: full coverage in isolation"
        );
        assert!(
            service.store.pending_notice(game)?.is_none(),
            "{series}: undelivered notice"
        );
        feed.reveal(0, 0);
    }
    let received = feed.notices()?;
    assert!(
        received
            .iter()
            .map(|notice| notice.lines().next())
            .eq(expected_notices.iter().map(|line| Some(line.as_str()))),
        "local Slack must receive each released draw exactly once, in replay order"
    );
    Ok(())
}

fn seed_first_round(path: &Path, archive: &str, game: Game) -> Result<()> {
    let db = Connection::open(path)?;
    db.execute_batch(include_str!("../schema.sql"))?;
    db.execute("ATTACH DATABASE ?1 AS archive", [archive])?;
    let key = [game.code()];
    db.execute(
        "INSERT INTO series SELECT code,name,first_draw_date,NULL FROM archive.series WHERE code=?1",
        key,
    )?;
    db.execute(
        "INSERT INTO source SELECT s.* FROM archive.source s JOIN archive.draw d ON s.id=d.source_id
         WHERE d.series_code=?1 AND d.round_no=1",
        key,
    )?;
    db.execute(
        "INSERT INTO draw SELECT * FROM archive.draw WHERE series_code=?1 AND round_no=1",
        key,
    )?;
    for table in ["lotto_result", "pension_number", "prize"] {
        db.execute_batch(&format!(
            "INSERT INTO {table} SELECT a.* FROM archive.{table} a JOIN main.draw USING (series_code,round_no);"
        ))?;
    }
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
struct Forecast {
    based_on: u32,
    target: NaiveDate,
    created: DateTime<Utc>,
    tickets: [Ticket; 5],
}

impl Forecast {
    fn read(db: &Connection, game: Game, round: u32) -> Result<Option<Self>> {
        let series = game.code();
        let row = db
            .query_row(
                "SELECT based_on_round,target_date,created_at_utc,ticket_json,alternatives_json
             FROM model_prediction WHERE series_code=?1 AND target_round=?2",
                (series, round),
                |r| {
                    Ok((
                        r.get(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, String>(4)?,
                    ))
                },
            )
            .optional()
            .with_context(|| format!("{series} round {round}: read five stored tickets"))?;
        assert_eq!(
            row.is_some(),
            round > WARMUP_END,
            "{series} round {round}: forecast availability"
        );
        let Some((based_on, target, created, first, rest)) = row else {
            return Ok(None);
        };
        let [a, b, c, d] = serde_json::from_str(&rest)?;
        let tickets = [serde_json::from_str(&first)?, a, b, c, d];
        for (i, ticket) in tickets.iter().enumerate() {
            assert!(
                !tickets[..i].contains(ticket),
                "{series} round {round}: duplicate ticket"
            );
        }
        let (target, created) = (target.parse()?, created.parse()?);
        assert_eq!(
            based_on,
            round - 1,
            "{series} round {round}: forecast basis"
        );
        assert!(
            seoul_date(created)? < target,
            "{series} round {round}: forecast deadline"
        );
        Ok(Some(Self {
            based_on,
            target,
            created,
            tickets,
        }))
    }
}

#[derive(Debug, PartialEq, Eq)]
struct PensionTier {
    rank: String,
    group: Option<u8>,
    digits: String,
}

#[derive(Debug, PartialEq, Eq)]
enum Numbers {
    Lotto { main: [u8; 6], bonus: u8 },
    Pension(Vec<PensionTier>),
}

impl Numbers {
    fn read(db: &Connection, game: Game, round: u32) -> Result<Self> {
        let key = (game.code(), round);
        let numbers = match game {
            Game::Lotto => db.query_row(
                "SELECT n1,n2,n3,n4,n5,n6,bonus FROM lotto_result WHERE series_code=?1 AND round_no=?2",
                key,
                |r| Ok(Self::Lotto {
                    main: [r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?],
                    bonus: r.get(6)?,
                }),
            )?,
            Game::Pension => {
                // Count a ticket once if either regular or bonus matches. Keep the best payout:
                // ranks 1 and 2 outrank or tie bonus; bonus beats ranks 3 through 7.
                let mut sql = db.prepare(
                    "SELECT tier,group_no,digits FROM pension_number WHERE series_code=?1 AND round_no=?2
                     ORDER BY CASE WHEN tier='bonus' THEN '2b' ELSE tier END, slot",
                )?;
                let tiers = sql.query_map(key, |r| Ok(PensionTier { rank: r.get(0)?, group: r.get(1)?, digits: r.get(2)? }))?;
                Self::Pension(tiers.collect::<rusqlite::Result<_>>()?)
            }
        };
        Ok(numbers)
    }

    fn rank(&self, ticket: &Ticket) -> Result<&str> {
        let rank = match (self, ticket) {
            (Self::Lotto { main, bonus }, Ticket::Lotto { numbers }) => {
                match (
                    main.iter().filter(|n| numbers.contains(n)).count(),
                    numbers.contains(bonus),
                ) {
                    (6, _) => "1",
                    (5, true) => "2",
                    (5, false) => "3",
                    (4, _) => "4",
                    (3, _) => "5",
                    _ => "miss",
                }
            }
            (Self::Pension(tiers), Ticket::Pension { group_no, digits }) => tiers
                .iter()
                .find(|tier| {
                    tier.group.is_none_or(|group| group == *group_no)
                        && digits.ends_with(tier.digits.as_str())
                })
                .map_or("miss", |tier| tier.rank.as_str()),
            (Self::Lotto { .. }, Ticket::Pension { .. })
            | (Self::Pension(_), Ticket::Lotto { .. }) => {
                bail!("ticket game differs from draw");
            }
        };
        Ok(rank)
    }
}

fn emit(game: Game, round: u32, ranks: &[&str]) {
    println!(
        "PERIOD_REPLAY {}",
        json!({"series": game.code(), "round": round, "ranks": ranks})
    );
}
