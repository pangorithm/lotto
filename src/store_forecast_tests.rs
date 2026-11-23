use anyhow::{Context, Result};
use chrono::{NaiveDate, TimeDelta, Utc};
use rusqlite::Connection;
use serde_json::json;
use tempfile::NamedTempFile;

use crate::domain::{Candidate, Game, Ticket};
use crate::jackpot::Strategy::{Jackpot, Recent};
use crate::store::notice::tests::{archive, lotto_after};
use crate::store::{Store, read_head, schema_version};

const FIRST: &str = r#"{ "numbers": [1, 2, 3, 4, 5, 6] }"#;
const OTHER: &str = r#"{"numbers":[8,9,10,11,12,13]}"#;
const CREATED: &str = "2000-01-01T00:00:00Z";

fn lotto_fixture(start_date: &str, rounds: u32) -> Result<NamedTempFile> {
    let file = archive()?;
    let db = Connection::open(file.path())?;
    db.execute_batch(include_str!("store_v2.sql"))?;
    db.execute_batch(include_str!("store_v3.sql"))?;
    db.execute(
        "UPDATE draw SET draw_date = ?1 WHERE series_code = 'lotto645'",
        [start_date],
    )?;
    db.execute(
        "WITH RECURSIVE rounds(n) AS (VALUES (2) UNION ALL SELECT n + 1 FROM rounds WHERE n < ?2)
         INSERT INTO draw (series_code, round_no, draw_date, source_id, source_locator)
         SELECT 'lotto645', n, date(?1, printf('+%d days', (n - 1) * 7)), 'seed', 'seed'
         FROM rounds",
        (start_date, rounds),
    )?;
    db.execute_batch(
        "INSERT INTO lotto_result (round_no, n1, n2, n3, n4, n5, n6, bonus)
         SELECT round_no, 1, 2, 3, 4, 5, 6, 7 FROM draw WHERE series_code = 'lotto645';",
    )?;
    Ok(file)
}

fn seed_v4_model(db: &Connection, first: &str) -> Result<Candidate> {
    db.execute_batch(include_str!("store_v4.sql"))?;
    let game = Game::Lotto;
    let based_on = read_head(db, game)?;
    let target = lotto_after(based_on)?.head;
    db.execute(
        "INSERT INTO model_prediction VALUES (?1, ?2, ?3, ?4, ?5, 'recent-bias-v1', ?6)",
        (
            game.code(),
            target.round,
            target.date.to_string(),
            based_on.round,
            CREATED,
            first,
        ),
    )?;
    Ok(Candidate {
        game,
        based_on,
        target,
        created_at: CREATED.parse()?,
    })
}

#[test]
fn status_hides_legacy_predictions_without_deleting_them() -> Result<()> {
    let file = archive()?;
    let path = file.path();
    let db = Connection::open(path)?;
    let archive_status = Store::status(path)?;
    assert_eq!(schema_version(&db)?, 1);
    assert!(archive_status["lotto645"].get("candidate").is_none());
    db.execute_batch(include_str!("store_v2.sql"))?;
    db.execute(
        "INSERT INTO prediction VALUES ('lotto645',2,'2099-11-28',1,?1,'uniform-v1',?2)",
        (CREATED, OTHER),
    )?;
    assert_eq!(Store::status(path)?, archive_status);
    assert_eq!(schema_version(&db)?, 2);

    let store = Store::open(path, &path.with_extension("seed"))?;
    assert_eq!(schema_version(&db)?, 6);
    assert_eq!(Store::status(path)?, archive_status);
    let ticket: String =
        db.query_row("SELECT ticket_json FROM prediction", [], |row| row.get(0))?;
    assert_eq!(ticket, OTHER);
    for game in Game::ALL {
        assert!(store.pending_notice(game)?.is_none());
    }
    Ok(())
}

#[test]
fn strategy_changes_only_affect_unpublished_rounds() -> Result<()> {
    let file = lotto_fixture("2099-01-03", 25)?;
    let path = file.path();
    let v3_status = Store::status(path)?;
    assert_eq!(schema_version(&Connection::open(path)?)?, 3);
    let mut store = Store::open(path, &path.with_extension("seed"))?;
    let game = Game::Lotto;
    let head = store.head(game)?;
    let draw26 = lotto_after(head)?;
    let mut candidate = Candidate {
        game,
        based_on: head,
        target: draw26.head,
        created_at: CREATED.parse()?,
    };
    assert!(!store.save_model_candidate_using(&candidate, &Utc::now, Recent)?);
    assert_eq!(Store::status(path)?, v3_status);

    store.apply(game, std::slice::from_ref(&draw26))?;
    assert!(!store.save_model_candidate_using(&candidate, &Utc::now, Recent)?);
    let draw27 = lotto_after(draw26.head)?;
    candidate.based_on = draw26.head;
    candidate.target = draw27.head;
    assert!(store.save_model_candidate_using(&candidate, &Utc::now, Recent)?);
    let status = Store::status(path)?;
    let model = &status[game.code()]["model_candidate"];
    let tickets = model["tickets"]
        .as_array()
        .context("every candidate has a ticket list")?;
    assert_eq!(tickets.len(), 5);
    assert_eq!(tickets[0], json!({"numbers": [1, 2, 3, 4, 5, 6]}));
    assert_eq!(model["algorithm"], "recent-bias-v1");
    assert_eq!(model["based_on_round"], 26);
    assert_eq!(model["target_round"], 27);
    assert_eq!(model["target_date"], json!(candidate.target.date));
    assert_eq!(model["created_at_utc"], CREATED);
    candidate.created_at += TimeDelta::seconds(1);
    assert!(!store.save_model_candidate_using(&candidate, &Utc::now, Jackpot)?);
    assert_eq!(Store::status(path)?, status);

    let notice = store.pending_notice(game)?.context("round 26 notice")?;
    assert!(notice.model_prior.is_empty());
    assert_eq!(json!(notice.model_next), model["tickets"]);
    assert_eq!(
        notice.model_next_algorithm.as_deref(),
        Some("recent-bias-v1")
    );
    store.apply(game, std::slice::from_ref(&draw27))?;
    store.mark_notice_sent(game, draw26.head.round)?;
    let notice = store.pending_notice(game)?.context("round 27 notice")?;
    assert_eq!(json!(notice.model_prior), model["tickets"]);
    assert!(notice.model_next.is_empty());
    assert!(
        Store::status(path)?[game.code()]
            .get("model_candidate")
            .is_none()
    );

    candidate.based_on = draw27.head;
    candidate.target = lotto_after(draw27.head)?.head;
    assert!(store.save_model_candidate_using(&candidate, &Utc::now, Jackpot)?);
    let jackpot_status = Store::status(path)?;
    let jackpot = &jackpot_status[game.code()]["model_candidate"];
    assert_eq!(jackpot["algorithm"], "jackpot-joint-v2");
    assert_eq!(
        jackpot["tickets"]
            .as_array()
            .context("five jackpot tickets")?
            .len(),
        5
    );
    assert_ne!(jackpot["tickets"], model["tickets"]);

    let mut store = Store::open(path, &path.with_extension("seed"))?;
    assert!(!store.save_model_candidate_using(&candidate, &Utc::now, Recent)?);
    assert_eq!(Store::status(path)?, jackpot_status);
    let notice = store.pending_notice(game)?.context("round 27 notice")?;
    assert_eq!(json!(notice.model_next), jackpot["tickets"]);
    assert_eq!(
        notice.model_next_algorithm.as_deref(),
        Some("jackpot-joint-v2")
    );
    Ok(())
}

#[test]
fn completes_a_matching_v4_ticket_with_its_stored_strategy() -> Result<()> {
    let file = lotto_fixture("2099-01-03", 26)?;
    let path = file.path();
    let db = Connection::open(path)?;
    let mut candidate = seed_v4_model(&db, FIRST)?;
    db.execute(
        "INSERT INTO notice_outbox VALUES ('lotto645', 26, NULL)",
        [],
    )?;
    let v4_status = Store::status(path)?;
    let v4 = &v4_status["lotto645"]["model_candidate"];
    assert_eq!(v4["tickets"], json!([{"numbers": [1, 2, 3, 4, 5, 6]}]));
    assert_eq!(schema_version(&db)?, 4);
    let mut store = Store::open(path, &path.with_extension("seed"))?;
    assert_eq!(schema_version(&db)?, 6);
    assert_eq!(Store::status(path)?, v4_status);
    let notice = store
        .pending_notice(Game::Lotto)?
        .context("round 26 notice")?;
    assert_eq!(json!(notice.model_next), v4["tickets"]);
    assert_eq!(
        notice.model_next_algorithm.as_deref(),
        Some("recent-bias-v1")
    );

    candidate.created_at += TimeDelta::seconds(1);
    assert!(store.save_model_candidate_using(&candidate, &Utc::now, Jackpot)?);
    let status = Store::status(path)?;
    let model = &status["lotto645"]["model_candidate"];
    let history: [_; 26] = std::array::from_fn(|_| Ticket::Lotto {
        numbers: [1, 2, 3, 4, 5, 6],
    });
    let expected = Recent
        .picks(Game::Lotto, &history)?
        .context("legacy strategy has enough history")?;
    assert_eq!(model["tickets"], json!(expected));
    assert_eq!(model["algorithm"], "recent-bias-v1");
    assert_eq!(model["created_at_utc"], CREATED);
    let first: String = db.query_row("SELECT ticket_json FROM model_prediction", [], |row| {
        row.get(0)
    })?;
    assert_eq!(first, FIRST);
    assert!(!store.save_model_candidate_using(&candidate, &Utc::now, Jackpot)?);
    assert_eq!(Store::status(path)?, status);
    Ok(())
}

#[test]
fn rejects_a_v4_ticket_that_differs_from_the_fresh_first_pick() -> Result<()> {
    let file = lotto_fixture("2099-01-03", 26)?;
    let path = file.path();
    let db = Connection::open(path)?;
    let candidate = seed_v4_model(&db, OTHER)?;
    let mut store = Store::open(path, &path.with_extension("seed"))?;
    let original = Store::status(path)?;

    let Err(error) = store.save_model_candidate_using(&candidate, &Utc::now, Jackpot) else {
        anyhow::bail!("first pick must match");
    };
    assert_eq!(
        error.to_string(),
        "first model ticket mismatch for lotto645 round 27"
    );
    assert_eq!(Store::status(path)?, original);
    Ok(())
}

#[test]
fn leaves_expired_v4_alternatives_unset_even_with_a_later_requested_date() -> Result<()> {
    let file = lotto_fixture("2000-01-01", 26)?;
    let path = file.path();
    let db = Connection::open(path)?;
    let mut candidate = seed_v4_model(&db, FIRST)?;
    let mut store = Store::open(path, &path.with_extension("seed"))?;
    candidate.target.date = NaiveDate::from_ymd_opt(2099, 1, 3).context("future target date")?;

    assert!(!store.save_model_candidate_using(&candidate, &Utc::now, Recent)?);
    let alternatives: Option<String> = db.query_row(
        "SELECT alternatives_json FROM model_prediction",
        [],
        |row| row.get(0),
    )?;
    assert!(alternatives.is_none());
    assert!(
        Store::status(path)?["lotto645"]
            .get("model_candidate")
            .is_none()
    );
    Ok(())
}
