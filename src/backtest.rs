use std::path::Path;

use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OpenFlags};
use serde::Serialize;

use crate::domain::{Game, Ticket};
use crate::forecast::{ALGORITHM, MAX_HISTORY, MIN_HISTORY, Model};
use crate::store::read_history;

#[derive(Serialize)]
pub(crate) struct Evaluation {
    game: &'static str,
    algorithm: &'static str,
    first_round: u32,
    last_round: u32,
    evaluated_draws: u32,
    loss_metric: &'static str,
    model_loss: f64,
    uniform_loss: f64,
    model_matches_per_draw: f64,
    uniform_expected_matches: f64,
    fair_draw_upper_tail: f64,
    snapshot_next_round: u32,
    snapshot_next_ticket: Ticket,
    snapshot_next_candidates: Vec<Ticket>,
}

fn choose(n: u32, k: u32) -> f64 {
    (0..k).fold(1.0, |p, i| p * f64::from(n - i) / f64::from(i + 1))
}

fn at_least_matches(game: Game, count: u32, observed: u32) -> Result<f64> {
    let per_round = match game {
        Game::Lotto => (0..=6)
            .map(|k| choose(6, k) * choose(39, 6 - k) / choose(45, 6))
            .collect::<Vec<_>>(),
        Game::Pension => {
            let mut probabilities = vec![1.0];
            for probability in [0.2, 0.1, 0.1, 0.1, 0.1, 0.1, 0.1] {
                let mut next = vec![0.0; probabilities.len() + 1];
                for (k, p) in probabilities.into_iter().enumerate() {
                    next[k] += p * (1.0 - probability);
                    next[k + 1] += p * probability;
                }
                probabilities = next;
            }
            probabilities
        }
    };
    let mut distribution = vec![1.0];
    for _ in 0..count {
        let mut next = vec![0.0; distribution.len() + per_round.len() - 1];
        for (i, p) in distribution.into_iter().enumerate() {
            for (j, q) in per_round.iter().enumerate() {
                next[i + j] += p * q;
            }
        }
        distribution = next;
    }
    Ok(distribution
        .into_iter()
        .skip(usize::try_from(observed)?)
        .sum::<f64>()
        .min(1.0))
}

pub(crate) fn run(path: &Path) -> Result<[Evaluation; 2]> {
    let db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let evaluate = |game: Game, count: u32, loss_metric| -> Result<Evaluation> {
        let rows = read_history(&db, game, u32::MAX, MAX_HISTORY + count)?;
        let rounds = rows.iter().map(|row| row.0).collect::<Vec<_>>();
        let tickets = rows.into_iter().map(|row| row.1).collect::<Vec<_>>();
        let count_usize = usize::try_from(count)?;
        ensure!(
            tickets.len() >= count_usize + MIN_HISTORY,
            "insufficient history for {} backtest",
            game.code()
        );
        ensure!(
            rounds
                .windows(2)
                .all(|pair| pair[0].checked_add(1) == Some(pair[1])),
            "non-consecutive archive"
        );
        let start = tickets.len() - count_usize;
        let last_round = *rounds.last().context("empty archive")?;
        let (mut model_loss, mut uniform_loss, mut matches) = (0.0, 0.0, 0);
        for i in start..tickets.len() {
            let history_start = i.saturating_sub(usize::try_from(MAX_HISTORY)?);
            let fitted = Model::fit(game, &tickets[history_start..i])?
                .context("insufficient training draws")?;
            let score = fitted.score(&tickets[i])?;
            model_loss += score.model;
            uniform_loss += score.uniform;
            matches += score.matches;
        }
        let next = Model::fit(game, &tickets)?.context("missing next-round model")?;
        Ok(Evaluation {
            game: game.code(),
            algorithm: ALGORITHM,
            first_round: rounds[start],
            last_round,
            evaluated_draws: count,
            loss_metric,
            model_loss: model_loss / f64::from(count),
            uniform_loss: uniform_loss / f64::from(count),
            model_matches_per_draw: f64::from(matches) / f64::from(count),
            uniform_expected_matches: 0.8,
            fair_draw_upper_tail: at_least_matches(game, count, matches)?,
            snapshot_next_round: last_round.checked_add(1).context("round overflow")?,
            snapshot_next_ticket: next.pick()?,
            snapshot_next_candidates: next.picks()?,
        })
    };
    Ok([
        evaluate(Game::Lotto, 208, "marginal_brier")?,
        evaluate(Game::Pension, 104, "group_position_log_loss")?,
    ])
}
