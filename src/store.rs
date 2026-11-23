use std::fs::{self, File};
use std::io::{self, ErrorKind};
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use chrono::{NaiveDate, Utc};
use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior};
use serde_json::{Map, Value, json};
use tempfile::NamedTempFile;

use crate::domain::{Draw, Game, Head, Record, seoul_date};

#[path = "store_notice.rs"]
mod notice;

const ARCHIVE_VERSION: i64 = 1;
const PREDICTION_VERSION: i64 = 2;
const RUNTIME_VERSION: i64 = 3;
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

pub(crate) struct Store {
    db: Connection,
}

impl Store {
    pub(crate) fn open(path: &Path, seed: &Path) -> Result<Self> {
        if !path.try_exists().context("inspect runtime database path")? {
            initialize(path, seed)
                .with_context(|| format!("initialize runtime database {}", path.display()))?;
        }
        match fs::metadata(seed) {
            Ok(archive) => {
                let target = fs::metadata(path).context("inspect runtime database file")?;
                ensure!(
                    (target.dev(), target.ino()) != (archive.dev(), archive.ino()),
                    "the runtime database aliases the seed"
                );
            }
            Err(error) if error.kind() == ErrorKind::NotFound => (),
            Err(error) => return Err(error).context("inspect seed file identity"),
        }
        let mut db = connect(path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
        migrate(&mut db)?;
        Ok(Self { db })
    }

    pub(crate) fn head(&self, game: Game) -> Result<Head> {
        read_head(&self.db, game)
    }

    pub(crate) fn apply(&mut self, game: Game, records: &[Record]) -> Result<()> {
        let db = &mut self.db;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut latest = read_head(&tx, game)?;
        let original_round = latest.round;
        let accessed_on = seoul_date(Utc::now())?.to_string();
        let code = game.code();
        for record in records {
            let Head { round, date } = record.head;
            ensure!(record.draw.game() == game, "draw does not belong to {code}");
            if round <= original_round {
                let exists: bool = tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM draw WHERE series_code = ?1 AND round_no = ?2)",
                    (code, round),
                    |row| row.get(0),
                )?;
                ensure!(exists, "missing historic {code} round {round}");
                continue;
            }
            ensure!(
                latest.round.checked_add(1) == Some(round) && date > latest.date,
                "{code} round {round} on {date} does not follow {latest:?}"
            );
            insert_draw(&tx, record, &accessed_on)
                .with_context(|| format!("insert {code} round {round}"))?;
            tx.execute(
                "INSERT INTO notice_outbox (series_code, round_no) VALUES (?1, ?2)",
                (code, round),
            )
            .with_context(|| format!("enqueue notice for {code} round {round}"))?;
            latest = record.head;
        }
        tx.commit().context("commit draw batch")
    }

    pub(crate) fn status(path: &Path) -> Result<Value> {
        let mut db = connect(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let tx = db.transaction()?;
        let version = schema_version(&tx)?;
        ensure!(
            matches!(
                version,
                ARCHIVE_VERSION | PREDICTION_VERSION | RUNTIME_VERSION
            ),
            "unsupported database user_version {version}"
        );
        let today = seoul_date(Utc::now())?.to_string();
        let mut status = Map::new();
        for game in Game::ALL {
            let latest = read_head(&tx, game)?;
            let candidate = if version >= PREDICTION_VERSION {
                let stored: Option<String> = tx.query_row(
                    "SELECT json_object('target_round', target_round, 'target_date', target_date,
                                        'based_on_round', based_on_round, 'created_at_utc', created_at_utc,
                                        'algorithm', algorithm, 'ticket', json(ticket_json))
                     FROM prediction WHERE series_code = ?1 AND based_on_round = ?2
                       AND target_round = ?2 + 1 AND target_date > ?3 AND target_date > ?4",
                    (game.code(), latest.round, &today, latest.date.to_string()),
                    |row| row.get(0),
                ).optional()?;
                let parse = |json: String| serde_json::from_str::<Value>(&json);
                stored.map(parse).transpose()?
            } else {
                None
            };
            let entry = json!({"latest": latest, "candidate": candidate});
            status.insert(game.code().to_owned(), entry);
        }
        tx.commit()?;
        Ok(Value::Object(status))
    }
}

fn connect(path: &Path, access: OpenFlags) -> Result<Connection> {
    let db = Connection::open_with_flags(path, access | OpenFlags::SQLITE_OPEN_NOFOLLOW)
        .with_context(|| format!("open database {}", path.display()))?;
    db.busy_timeout(BUSY_TIMEOUT)?;
    db.pragma_update(None, "foreign_keys", true)?;
    Ok(db)
}

fn initialize(path: &Path, seed: &Path) -> Result<()> {
    let mut source = File::open(seed).with_context(|| format!("open seed {}", seed.display()))?;
    let dir = path.parent().filter(|dir| !dir.as_os_str().is_empty());
    let parent = dir.unwrap_or(Path::new("."));
    fs::create_dir_all(parent).context("create runtime database directory")?;
    let mut stage = NamedTempFile::new_in(parent)?;
    io::copy(&mut source, stage.as_file_mut()).context("copy seed to staging file")?;
    {
        let mut db = connect(stage.path(), OpenFlags::SQLITE_OPEN_READ_WRITE)?;
        for game in Game::ALL {
            read_head(&db, game).context("invalid historic seed")?;
        }
        migrate(&mut db)?;
    }
    stage.as_file().sync_all().context("sync staged database")?;
    match stage.persist_noclobber(path) {
        Ok(_) => Ok(()),
        // Another startup published its own complete copy first.
        Err(error) if error.error.kind() == ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(error.error).context("publish staged database"),
    }
}

fn schema_version(db: &Connection) -> Result<i64> {
    db.pragma_query_value(None, "user_version", |row| row.get(0))
        .context("read database user_version")
}

fn migrate(db: &mut Connection) -> Result<()> {
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    match schema_version(&tx)? {
        ARCHIVE_VERSION => {
            tx.execute_batch(include_str!("store_v2.sql"))?;
            tx.execute_batch(include_str!("store_v3.sql"))?;
        }
        PREDICTION_VERSION => tx.execute_batch(include_str!("store_v3.sql"))?,
        RUNTIME_VERSION => (),
        version => bail!("unsupported database user_version {version}"),
    }
    tx.commit().context("commit database migration")
}

fn read_head(db: &Connection, game: Game) -> Result<Head> {
    let (round, date): (u32, String) = db.query_row(
        "SELECT round_no, draw_date FROM draw WHERE series_code = ?1 ORDER BY round_no DESC LIMIT 1",
        [game.code()],
        |row| Ok((row.get(0)?, row.get(1)?)),
    ).with_context(|| format!("read latest {} draw", game.code()))?;
    let date = NaiveDate::parse_from_str(&date, "%Y-%m-%d").context("invalid stored draw date")?;
    Ok(Head { round, date })
}

fn insert_draw(db: &Connection, record: &Record, accessed_on: &str) -> Result<()> {
    let game = record.draw.game();
    let code = game.code();
    let round = record.head.round;
    let source_id = format!("daemon:{code}:{round}");
    let url = &record.source_url;
    let host = url.host_str().context("draw source URL has no host")?;
    let (publisher, kind) = match host {
        "www.dhlottery.co.kr" | "dhlottery.co.kr" => ("동행복권", "official"),
        _ => (host, "third_party"),
    };
    let (sales, field) = match &record.draw {
        Draw::Lotto(draw) => (Some(draw.sales_krw), "ltEpsd"),
        Draw::Pension(_) => (None, "psltEpsd"),
    };
    let title = format!("{code} round {round}");
    let url = url.as_str();
    db.execute(
        "INSERT INTO source (id, url, publisher, title, kind, accessed_on)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        (&source_id, url, publisher, title, kind, accessed_on),
    )?;
    db.execute(
        "INSERT INTO draw (series_code, round_no, draw_date, source_id, source_locator, total_sales_krw)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        (code, round, record.head.date.to_string(), &source_id, format!("{field}={round}"), sales),
    )?;
    match &record.draw {
        Draw::Lotto(draw) => {
            let [n1, n2, n3, n4, n5, n6] = draw.numbers;
            db.execute(
                "INSERT INTO lotto_result (series_code, round_no, n1, n2, n3, n4, n5, n6, bonus)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                (code, round, n1, n2, n3, n4, n5, n6, draw.bonus),
            )?;
            let mut prize = db.prepare(
                "INSERT INTO prize (series_code, round_no, tier, winning_games, unit_krw,
                                    payment_period, total_krw) VALUES (?1, ?2, ?3, ?4, ?5, 'once', ?6)",
            )?;
            for (tier, p) in ["1", "2", "3", "4", "5"].into_iter().zip(&draw.prizes) {
                prize.execute((code, round, tier, p.winners, p.unit_krw, p.total_krw))?;
            }
        }
        Draw::Pension(items) => {
            let mut numbers = db.prepare(
                "INSERT INTO pension_number (series_code, round_no, tier, slot, group_no, digits)
                 VALUES (?1, ?2, ?3, 1, ?4, ?5)",
            )?;
            let mut prize = db.prepare(
                "INSERT INTO prize (series_code, round_no, tier, unit_krw, payment_period, payment_months)
                 VALUES (?1, ?2, ?3, ?4, CASE WHEN ?5 IS NULL THEN 'once' ELSE 'monthly' END, ?5)",
            )?;
            for p in items.iter() {
                numbers.execute((code, round, p.tier, p.group, &p.digits))?;
                prize.execute((code, round, p.tier, p.unit_krw, p.months))?;
            }
        }
    }
    Ok(())
}

#[test]
fn enqueue_failure_rolls_back_the_draw() -> Result<()> {
    use notice::tests::{archive, lotto_after};
    let file = archive()?;
    let mut store = Store::open(file.path(), &file.path().with_extension("seed"))?;
    let game = Game::Lotto;
    let head = store.head(game)?;
    store.db.execute_batch(
        "CREATE TEMP TRIGGER reject_notice BEFORE INSERT ON notice_outbox
         BEGIN SELECT RAISE(ABORT, 'fixture blocks enqueue'); END;",
    )?;

    let result = store.apply(game, &[lotto_after(head)?]).err();
    assert!(matches!(
        result.and_then(|error| error.downcast::<rusqlite::Error>().ok()),
        Some(rusqlite::Error::SqliteFailure(_, Some(message))) if message == "fixture blocks enqueue"
    ));
    assert_eq!(store.head(game)?, head);
    assert!(store.pending_notice(game)?.is_none());
    Ok(())
}
