use std::time::Duration;

use anyhow::{Context, Result, ensure};
use chrono::{DateTime, Days, Utc};
use tokio::task::block_in_place;

use crate::collector::Collector;
use crate::domain::{Candidate, Game, Head, seoul_date};
use crate::slack::Slack;
use crate::store::Store;

pub(crate) struct Service {
    pub(crate) store: Store,
    pub(crate) collector: Collector,
    pub(crate) slack: Option<Slack>,
}

impl Service {
    pub(crate) async fn poll(&mut self) -> Result<()> {
        self.poll_with_clock(&Utc::now).await
    }

    pub(crate) async fn poll_with_clock(&mut self, now: &impl Fn() -> DateTime<Utc>) -> Result<()> {
        let mut failed = Vec::new();
        for game in Game::ALL {
            if let Err(error) = self.poll_game(game, now).await {
                tracing::warn!(game = game.code(), error = %format_args!("{error:#}"), "collection failed");
                failed.push(game.code());
                continue;
            }
            if let Err(error) = self.deliver_notices(game).await {
                tracing::warn!(game = game.code(), %error, "Slack delivery pending; will retry");
                failed.push(game.code());
            }
        }
        ensure!(
            failed.is_empty(),
            "poll incomplete for {}",
            failed.join(", ")
        );
        Ok(())
    }

    async fn deliver_notices(&mut self, game: Game) -> Result<()> {
        let Some(slack) = &self.slack else {
            return Ok(());
        };
        for _ in 0..8 {
            let Some(notice) = block_in_place(|| self.store.pending_notice(game))? else {
                break;
            };
            slack.send(&notice).await?;
            let round = notice.head.round;
            block_in_place(|| self.store.mark_notice_sent(game, round))?;
            tracing::info!(game = game.code(), round, "Slack result sent");
        }
        Ok(())
    }

    async fn poll_game(&mut self, game: Game, now: &impl Fn() -> DateTime<Utc>) -> Result<()> {
        let remote = self
            .collector
            .latest(game)
            .await
            .context("fetch latest round")?;
        let mut local = block_in_place(|| self.store.head(game))?;
        ensure!(
            remote.date <= seoul_date(Utc::now())?,
            "source returned a future draw"
        );
        ensure!(
            remote.round >= local.round,
            "stale source ({} < {})",
            remote.round,
            local.round
        );
        while local.round < remote.round {
            let first = local.round.checked_add(1).context("round overflow")?;
            let end = first.saturating_add(7).min(remote.round);
            let records = self
                .collector
                .fetch(game, first, end)
                .await
                .context("fetch missing draws")?;
            let last = records
                .last()
                .context("missing final draw in response")?
                .head;
            ensure!(
                last.round == end && last.date <= remote.date,
                "archive batch exceeds latest draw"
            );
            if last.round == remote.round {
                ensure!(last == remote, "latest and archive draw dates disagree");
            }
            block_in_place(|| self.store.apply(game, &records))
                .context("append result transaction")?;
            tracing::info!(game = game.code(), first, last = end, "draws stored");
            local = last;
            if local.round < remote.round {
                tokio::time::sleep(Duration::from_millis(350)).await;
            }
        }
        ensure!(local == remote, "latest and stored draw dates disagree");
        if let Some(candidate) = candidate(game, local, now())? {
            if block_in_place(|| self.store.save_model_candidate(&candidate))? {
                tracing::info!(
                    game = game.code(),
                    round = candidate.target.round,
                    "experimental forecast stored"
                );
            }
        } else {
            tracing::debug!(
                game = game.code(),
                latest = local.round,
                "candidate window closed"
            );
        }
        Ok(())
    }
}

fn candidate(game: Game, head: Head, now: DateTime<Utc>) -> Result<Option<Candidate>> {
    let date = head
        .date
        .checked_add_days(Days::new(7))
        .context("draw date overflow")?;
    if seoul_date(now)? >= date {
        return Ok(None);
    }
    let round = head.round.checked_add(1).context("round overflow")?;
    Ok(Some(Candidate {
        game,
        based_on: head,
        target: Head { round, date },
        created_at: now,
    }))
}

#[cfg(test)]
mod tests {
    use anyhow::{Context, Result};
    use chrono::{DateTime, NaiveDate, Utc};

    use super::candidate;
    use crate::domain::{Game, Head};

    #[test]
    fn stops_generating_at_midnight_in_seoul() -> Result<()> {
        let head = Head {
            round: 1251,
            date: NaiveDate::from_ymd_opt(2026, 11, 21).context("fixture draw date")?,
        };
        let before = DateTime::parse_from_rfc3339("2026-11-27T14:59:59Z")?.with_timezone(&Utc);
        let midnight = DateTime::parse_from_rfc3339("2026-11-27T15:00:00Z")?.with_timezone(&Utc);
        assert!(candidate(Game::Lotto, head, before)?.is_some());
        assert!(candidate(Game::Lotto, head, midnight)?.is_none());
        Ok(())
    }
}
