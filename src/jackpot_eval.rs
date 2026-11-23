use std::path::Path;

use anyhow::{Context, Result, bail, ensure};
use rusqlite::{Connection, OpenFlags};
use serde::Serialize;

use crate::domain::{Game, Ticket};
use crate::forecast::{MAX_HISTORY, MIN_HISTORY, Model};
use crate::forecast_batch::PICKS;
use crate::jackpot::Joint;
use crate::store::read_history;

fn legacy_probability(model: &Model, outcome: &Ticket) -> Result<f64> {
    match (model, outcome) {
        (Model::Lotto(scores), Ticket::Lotto { numbers }) => {
            let sum = numbers
                .iter()
                .map(|&n| scores[usize::from(n - 1)])
                .sum::<f64>();
            Ok(sum / (1_086_008.0 * scores.iter().sum::<f64>()))
        }
        (
            Model::Pension { groups, digits },
            Ticket::Pension {
                group_no,
                digits: actual,
            },
        ) => {
            let mut p = groups[usize::from(*group_no - 1)];
            for (table, value) in digits.iter().zip(actual.bytes()) {
                p *= table[usize::from(value - b'0')];
            }
            Ok(p)
        }
        _ => bail!("legacy scoring game mismatch"),
    }
}

#[derive(Serialize)]
struct Window {
    first_round: u32,
    last_round: u32,
    draws: u32,
    v1_jackpot_rounds: u32,
    v2_jackpot_rounds: u32,
    uniform_expected_jackpot_rounds: f64,
    uniform_mean_nll: f64,
    v1_mean_nll: f64,
    v2_mean_nll: f64,
}

struct Trial {
    round: u32,
    old_hit: bool,
    new_hit: bool,
    old_loss: f64,
    new_loss: f64,
}

fn summarize(trials: &[Trial], outcomes: f64) -> Result<Window> {
    let n = u32::try_from(trials.len())?;
    let first = trials.first().context("empty evaluation window")?;
    let last = trials.last().context("empty evaluation window")?;
    Ok(Window {
        first_round: first.round,
        last_round: last.round,
        draws: n,
        v1_jackpot_rounds: u32::try_from(trials.iter().filter(|r| r.old_hit).count())?,
        v2_jackpot_rounds: u32::try_from(trials.iter().filter(|r| r.new_hit).count())?,
        uniform_expected_jackpot_rounds: f64::from(n) * 5.0 / outcomes,
        uniform_mean_nll: outcomes.ln(),
        v1_mean_nll: trials.iter().map(|r| r.old_loss).sum::<f64>() / f64::from(n),
        v2_mean_nll: trials.iter().map(|r| r.new_loss).sum::<f64>() / f64::from(n),
    })
}

#[derive(Serialize)]
pub(crate) struct Comparison {
    series: &'static str,
    all: Window,
    recent: Window,
    next_round: u32,
    next_v2_tickets: Vec<Ticket>,
}

pub(crate) fn run(path: &Path) -> Result<Vec<Comparison>> {
    let db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let mut comparisons = Vec::new();
    for (game, outcomes, recent_n) in [
        (Game::Lotto, 8_145_060.0, 208),
        (Game::Pension, 5_000_000.0, 104),
    ] {
        let (rounds, history): (Vec<_>, Vec<_>) = read_history(&db, game, u32::MAX, u32::MAX)?
            .into_iter()
            .unzip();
        ensure!(
            history.len() >= MIN_HISTORY + recent_n
                && rounds.first() == Some(&1)
                && rounds.windows(2).all(|w| w[0].checked_add(1) == Some(w[1])),
            "incomplete evaluation history"
        );
        let mut trials = Vec::new();
        for i in MIN_HISTORY..history.len() {
            let prior = &history[i.saturating_sub(usize::try_from(MAX_HISTORY)?)..i];
            let old = Model::fit(game, prior)?.context("missing v1 fit")?;
            let new = Joint::fit(game, prior)?;
            let old_picks = old.picks()?;
            let new_picks = new.picks()?;
            ensure!(
                old_picks.len() == PICKS
                    && new_picks.len() == PICKS
                    && new_picks
                        .iter()
                        .enumerate()
                        .all(|(j, t)| !new_picks[..j].contains(t)),
                "invalid evaluation batch"
            );
            let actual = &history[i];
            let p = new.probability(actual)?;
            let old_p = legacy_probability(&old, actual)?;
            ensure!(
                p.is_finite()
                    && p > 0.0
                    && p <= 1.0
                    && old_p.is_finite()
                    && old_p > 0.0
                    && old_p <= 1.0,
                "invalid outcome probability"
            );
            trials.push(Trial {
                round: rounds[i],
                old_hit: old_picks.contains(actual),
                new_hit: new_picks.contains(actual),
                old_loss: -old_p.ln(),
                new_loss: -p.ln(),
            });
        }
        comparisons.push(Comparison {
            series: game.code(),
            all: summarize(&trials, outcomes)?,
            recent: summarize(&trials[trials.len() - recent_n..], outcomes)?,
            next_round: rounds
                .last()
                .context("empty archive")?
                .checked_add(1)
                .context("round overflow")?,
            next_v2_tickets: Joint::fit(game, &history)?.picks()?,
        });
    }
    Ok(comparisons)
}
