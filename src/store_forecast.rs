use std::path::Path;

use anyhow::{Context, Result, anyhow, ensure};
use chrono::{DateTime, NaiveDate, SecondsFormat, Utc};
use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior};
use serde_json::{Map, Value, json};

use super::{BATCH_VERSION, MODEL_VERSION, Store, connect, read_head, schema_version};
use crate::domain::{Candidate, Game, Ticket, seoul_date};
use crate::forecast::{ALGORITHM, MAX_HISTORY, Model};
use crate::forecast_batch::PICKS;

impl Store {
    pub(crate) fn save_model_candidate(
        &mut self,
        candidate: &Candidate,
        now: &impl Fn() -> DateTime<Utc>,
    ) -> Result<bool> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let game = candidate.game;
        let latest = read_head(&tx, game)?;
        let target = candidate.target;
        let today = seoul_date(now())?;
        if latest != candidate.based_on
            || latest.round.checked_add(1) != Some(target.round)
            || target.date <= latest.date
            || target.date <= today
        {
            return Ok(false);
        }
        let key = (game.code(), target.round);
        let stored = tx.query_row(
            "SELECT ticket_json, target_date, alternatives_json IS NOT NULL FROM model_prediction
             WHERE series_code = ?1 AND target_round = ?2",
            key,
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, bool>(2)?)),
        ).optional()?;
        let (first_json, deadline) = match stored {
            Some((_, _, true)) => return Ok(false),
            Some((first_json, date, false)) => {
                let stored_date: NaiveDate = date.parse().context("invalid stored target date")?;
                if stored_date <= latest.date || stored_date <= today {
                    return Ok(false);
                }
                (Some(first_json), target.date.min(stored_date))
            }
            None => (None, target.date),
        };
        let history: Vec<Ticket> = super::read_history(&tx, game, latest.round, MAX_HISTORY)?
            .into_iter()
            .map(|(_, ticket)| ticket)
            .collect();
        let Some(model) = Model::fit(game, &history)? else {
            return Ok(false);
        };
        let [first, alternatives @ ..]: [Ticket; PICKS] = model
            .picks()?
            .try_into()
            .map_err(|_| anyhow!("expected {PICKS} model tickets"))?;
        if deadline <= seoul_date(now())? {
            return Ok(false);
        }
        let alternatives = serde_json::to_string(&alternatives)?;
        if let Some(first_json) = first_json {
            let stored: Ticket =
                serde_json::from_str(&first_json).context("invalid stored first model ticket")?;
            ensure!(
                first == stored,
                "first model ticket mismatch for {} round {}",
                game.code(),
                target.round
            );
            tx.execute(
                "UPDATE model_prediction SET alternatives_json = ?3
                 WHERE series_code = ?1 AND target_round = ?2",
                (key.0, key.1, alternatives),
            )?;
        } else {
            tx.execute(
                "INSERT INTO model_prediction (series_code, target_round, target_date, based_on_round,
                                               created_at_utc, algorithm, ticket_json, alternatives_json)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                (
                    key.0,
                    key.1,
                    target.date.to_string(),
                    latest.round,
                    candidate.created_at.to_rfc3339_opts(SecondsFormat::AutoSi, true),
                    ALGORITHM,
                    serde_json::to_string(&first)?,
                    alternatives,
                ),
            )?;
        }
        tx.commit().context("commit model candidate")?;
        Ok(true)
    }

    pub(crate) fn status(path: &Path) -> Result<Value> {
        let mut db = connect(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let tx = db.transaction()?;
        let version = schema_version(&tx)?;
        let today = seoul_date(Utc::now())?.to_string();
        let alternatives_column = if version >= BATCH_VERSION {
            "alternatives_json"
        } else {
            "NULL"
        };
        let mut status = Map::new();
        for game in Game::ALL {
            let latest = read_head(&tx, game)?;
            let mut entry = json!({"latest": latest});
            if version >= MODEL_VERSION {
                let stored = tx
                    .query_row(
                        &format!(
                        "SELECT target_round, target_date, created_at_utc, algorithm, ticket_json,
                                {alternatives_column}
                         FROM model_prediction WHERE series_code = ?1 AND based_on_round = ?2
                           AND target_round = ?2 + 1 AND target_date > ?3 AND target_date > ?4"
                    ),
                        (game.code(), latest.round, &today, latest.date.to_string()),
                        |row| {
                            Ok((
                                row.get::<_, u32>(0)?,
                                row.get::<_, String>(1)?,
                                row.get::<_, String>(2)?,
                                row.get::<_, String>(3)?,
                                row.get::<_, String>(4)?,
                                row.get::<_, Option<String>>(5)?,
                            ))
                        },
                    )
                    .optional()?;
                if let Some((
                    target_round,
                    target_date,
                    created_at_utc,
                    algorithm,
                    first,
                    alternatives,
                )) = stored
                {
                    entry["model_candidate"] = json!({
                        "based_on_round": latest.round,
                        "target_round": target_round,
                        "target_date": target_date,
                        "created_at_utc": created_at_utc,
                        "algorithm": algorithm,
                        "tickets": decode_model_tickets(Some(&first), alternatives.as_deref())?,
                    });
                }
            }
            status.insert(game.code().to_owned(), entry);
        }
        tx.commit()?;
        Ok(Value::Object(status))
    }
}

pub(super) fn decode_model_tickets(
    first: Option<&str>,
    alternatives: Option<&str>,
) -> Result<Vec<Ticket>> {
    let Some(first) = first else {
        return Ok(Vec::new());
    };
    let first = serde_json::from_str(first).context("invalid stored first model ticket")?;
    if let Some(alternatives) = alternatives {
        let rest: [Ticket; PICKS - 1] =
            serde_json::from_str(alternatives).context("invalid stored model alternatives")?;
        Ok(std::iter::once(first).chain(rest).collect())
    } else {
        Ok(vec![first])
    }
}

/// Read up to `limit` tickets through the inclusive round, oldest first.
pub(crate) fn read_history(
    db: &Connection,
    game: Game,
    through: u32,
    limit: u32,
) -> Result<Vec<(u32, Ticket)>> {
    let sql = match game {
        Game::Lotto => {
            "SELECT round_no, n1, n2, n3, n4, n5, n6 FROM lotto_result
             WHERE series_code = ?1 AND round_no <= ?2 ORDER BY round_no DESC LIMIT ?3"
        }
        Game::Pension => {
            "SELECT round_no, group_no, digits FROM pension_number
             WHERE series_code = ?1 AND round_no <= ?2 AND tier = '1' AND slot = 1
             ORDER BY round_no DESC LIMIT ?3"
        }
    };
    let mut query = db.prepare(sql)?;
    let mut history = query
        .query_map((game.code(), through, limit), |row| {
            let ticket = match game {
                Game::Lotto => Ticket::Lotto {
                    numbers: [
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                    ],
                },
                Game::Pension => Ticket::Pension {
                    group_no: row.get(1)?,
                    digits: row.get(2)?,
                },
            };
            Ok((row.get(0)?, ticket))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    history.reverse();
    Ok(history)
}

#[cfg(test)]
#[path = "store_forecast_tests.rs"]
mod tests;
