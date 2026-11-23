use std::fs;

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Days, NaiveDate, Utc};
use rusqlite::{Connection, OpenFlags, OptionalExtension};

use crate::collector::Collector;
use crate::domain::{Game, Ticket, seoul_date};
use crate::replay_feed::ReplayFeed;
use crate::service::Service;
use crate::slack::Slack;
use crate::store::Store;

const CUTOFF: [(Game, u32); 2] = [(Game::Lotto, 1243), (Game::Pension, 334)];
const REPLAYS: u32 = 8;
const COMPARISON: &str = "저장된 예측과 실제 당첨번호 비교:\n";
const READ_ONLY: OpenFlags = OpenFlags::SQLITE_OPEN_READ_ONLY;

#[derive(Debug, PartialEq, Eq)]
struct ResultNumbers {
    date: NaiveDate,
    winner: Ticket,
    bonus: String,
}

fn result_numbers(db: &Connection, game: Game, round: u32) -> Result<Option<ResultNumbers>> {
    let sql = match game {
        Game::Lotto => "SELECT draw_date,json_object('numbers',json_array(n1,n2,n3,n4,n5,n6)),CAST(bonus AS TEXT)
                        FROM v_lotto_draws WHERE round_no=?2 AND ?1='lotto645'",
        Game::Pension => "SELECT a.draw_date,json_object('group_no',a.group_no,'digits',a.digits),b.digits
                          FROM v_pension_draws a JOIN pension_number b USING (series_code,round_no)
                          WHERE a.series_code=?1 AND a.round_no=?2 AND a.tier='1' AND b.tier='bonus'",
    };
    let read =
        |r: &rusqlite::Row<'_>| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get(2)?));
    let row = db.query_row(sql, (game.code(), round), read).optional()?;
    let decode = |(date, ticket, bonus): (String, String, String)| {
        let (date, winner) = (date.parse()?, serde_json::from_str(&ticket)?);
        Ok(ResultNumbers {
            date,
            winner,
            bonus,
        })
    };
    row.map(decode).transpose()
}

#[derive(Debug, PartialEq, Eq)]
struct Forecast {
    date: NaiveDate,
    created: DateTime<Utc>,
    tickets: [Ticket; 5],
}

fn forecast(db: &Connection, game: Game, round: u32) -> Result<Forecast> {
    let sql = "SELECT target_date,created_at_utc,ticket_json,alternatives_json
               FROM model_prediction WHERE series_code=?1 AND target_round=?2";
    let (date, created, first, rest): (String, String, String, String) =
        db.query_row(sql, (game.code(), round), |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })?;
    let [a, b, c, d] = serde_json::from_str(&rest)?;
    let tickets = [serde_json::from_str(&first)?, a, b, c, d];
    for (i, ticket) in tickets.iter().enumerate() {
        assert!(!tickets[..i].contains(ticket), "duplicate ticket");
    }
    let (date, created) = (date.parse()?, created.parse()?);
    Ok(Forecast {
        date,
        created,
        tickets,
    })
}

fn ticket_text(ticket: &Ticket) -> String {
    match ticket {
        Ticket::Lotto { numbers } => format!("{numbers:?}"),
        Ticket::Pension { group_no, digits } => format!("{group_no}조 {digits}"),
    }
}

#[derive(Debug, Default)]
struct Scores {
    lotto_best_matches: [usize; 7],
    lotto_tickets_with_3_or_more: usize,
    lotto_first_prize: usize,
    lotto_bonus_number_matches: usize,
    pension_best_suffix: [usize; 7],
    pension_first_prize: usize,
    pension_bonus_prize: usize,
}

impl Scores {
    fn compare(&mut self, result: &ResultNumbers, prior: (&str, &[Ticket; 5])) -> Result<()> {
        let (text, tickets) = prior;
        let mut best = 0;
        for (i, ticket) in tickets.iter().enumerate() {
            let row = format!("{}. {}\n", i + 1, ticket_text(ticket));
            let score = text.split_once(&row).context("prior ticket missing")?.1;
            match (&result.winner, ticket) {
                (Ticket::Lotto { numbers: winning }, Ticket::Lotto { numbers }) => {
                    let hits = winning.iter().filter(|n| numbers.contains(n));
                    let matched: Vec<_> = hits.copied().collect();
                    let bonus = numbers.contains(&result.bonus.parse()?);
                    best = best.max(matched.len());
                    self.lotto_tickets_with_3_or_more += usize::from(matched.len() >= 3);
                    self.lotto_first_prize += usize::from(numbers == winning);
                    self.lotto_bonus_number_matches += usize::from(bonus);
                    let expected = format!("일치 번호: {matched:?} / 보너스 일치: {bonus} /");
                    assert!(score.starts_with(&expected));
                }
                (
                    Ticket::Pension {
                        group_no: group,
                        digits: winning,
                    },
                    Ticket::Pension { group_no, digits },
                ) => {
                    let suffix = (1..=6).rev().find(|n| digits.ends_with(&winning[6 - n..]));
                    let suffix = suffix.unwrap_or_default();
                    best = best.max(suffix);
                    self.pension_first_prize += usize::from(group_no == group && digits == winning);
                    self.pension_bonus_prize += usize::from(digits == &result.bonus);
                    let expected = format!("끝자리 연속 일치: {suffix}자리");
                    let line = score.lines().next().context("pension comparison")?;
                    assert!(line.ends_with(&expected));
                }
                (Ticket::Lotto { .. }, Ticket::Pension { .. })
                | (Ticket::Pension { .. }, Ticket::Lotto { .. }) => {
                    bail!("forecast game differs from draw")
                }
            }
        }
        match result.winner {
            Ticket::Lotto { .. } => self.lotto_best_matches[best] += 1,
            Ticket::Pension { .. } => self.pension_best_suffix[best] += 1,
        }
        Ok(())
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn forecasts_precede_publication_and_notices_survive_restart() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let seed = dir.path().join("cutoff.sqlite3");
    let runtime = dir.path().join("runtime.sqlite3");
    let webhook = dir.path().join("webhook");
    let archive = format!(
        "file:{}/lottery.sqlite3?mode=ro&immutable=1",
        env!("CARGO_MANIFEST_DIR")
    );
    let cutoff = Connection::open(&seed)?;
    cutoff.execute_batch(include_str!("../schema.sql"))?;
    cutoff.execute("ATTACH DATABASE ?1 AS archive", [&archive])?;
    cutoff.execute_batch(
        "INSERT INTO source VALUES ('cutoff','urn:replay:cutoff','replay','Cutoff history',
                                    'third_party','2026-09-27',NULL,NULL);
         INSERT INTO series SELECT code,name,first_draw_date,NULL FROM archive.series
         WHERE code IN ('lotto645','pension720');",
    )?;
    for (game, head) in CUTOFF {
        let copied = cutoff.execute(
            "INSERT INTO draw SELECT series_code,round_no,draw_date,'cutoff','observed history',total_sales_krw
             FROM archive.draw WHERE series_code=?1 AND round_no BETWEEN ?2 AND ?3",
            (game.code(), head - 103, head),
        )?;
        assert_eq!(copied, 104, "{} cutoff history", game.code());
    }
    for table in ["lotto_result", "pension_number", "prize"] {
        cutoff.execute_batch(&format!(
            "INSERT INTO {table} SELECT a.* FROM archive.{table} a JOIN main.draw USING (series_code,round_no);"
        ))?;
    }
    drop(cutoff);
    let archive = Connection::open_with_flags(archive, READ_ONLY | OpenFlags::SQLITE_OPEN_URI)?;
    let mut events = Vec::new();
    for (game, head) in CUTOFF {
        for round in head + 1..=head + REPLAYS {
            let result = result_numbers(&archive, game, round)?.context("archive result")?;
            events.push((game, round, result));
        }
    }
    drop(archive);
    events.sort_unstable_by_key(|(_, _, result)| result.date);

    let feed = ReplayFeed::start().await?;
    let url = feed.url().join("/services/replay/test/token")?;
    fs::write(&webhook, url.as_str())?;
    let start = || -> Result<Service> {
        Ok(Service {
            strategy: crate::jackpot::Strategy::Recent,
            store: Store::open(&runtime, &seed)?,
            collector: Collector::new(feed.url().clone())?,
            slack: Some(Slack::new(&webhook)?),
        })
    };
    let mut service = start()?;
    let db = Connection::open_with_flags(&runtime, READ_ONLY)?;
    let mut now = "2026-09-27T13:00:00Z".parse::<DateTime<Utc>>()?;
    for (game, round) in CUTOFF {
        let head = service.store.head(game)?;
        assert!(head.round == round && head.date < seoul_date(now)?);
    }
    service.poll_with_clock(&|| now).await?;
    assert!(feed.notices()?.is_empty(), "seed rounds announced");
    let [(lotto, mut lotto_round), (pension, mut pension_round)] = CUTOFF;
    let mut lotto_next = forecast(&db, lotto, lotto_round + 1)?;
    let mut pension_next = forecast(&db, pension, pension_round + 1)?;
    let mut scores = Scores::default();
    for (i, (game, round, expected)) in events.into_iter().enumerate() {
        let case = format!("{} {round}", game.code());
        let (released, next) = match game {
            Game::Lotto => (&mut lotto_round, &mut lotto_next),
            Game::Pension => (&mut pension_round, &mut pension_next),
        };
        let prior = forecast(&db, game, round)?;
        assert_eq!(&prior, next, "{case}: prior forecast changed");
        assert_eq!(prior.date, expected.date);
        assert!(prior.created <= now && seoul_date(now)? < prior.date);
        let hidden = result_numbers(&db, game, round)?.is_none();
        assert!(hidden, "{case}: leaked result");

        let time = expected.date.and_hms_opt(13, 0, 0);
        now = time.context("publication timestamp")?.and_utc();
        assert_eq!(round, *released + 1);
        *released = round;
        feed.reveal(lotto_round, pension_round);
        service.poll_with_clock(&|| now).await?;
        assert_eq!(service.store.head(lotto)?.round, lotto_round);
        assert_eq!(service.store.head(pension)?.round, pension_round);
        assert_eq!(forecast(&db, game, round)?, prior);
        assert_eq!(result_numbers(&db, game, round)?.as_ref(), Some(&expected));
        *next = forecast(&db, game, round + 1)?;
        assert_eq!(next.created, now);
        assert_eq!(next.date, expected.date + Days::new(7));

        let notices = feed.notices()?;
        assert_eq!(notices.len(), i + 1, "{case}: webhook deliveries");
        let (opening, prior_text) = notices[i].split_once(COMPARISON).context("comparison")?;
        let (label, bonus) = match game {
            Game::Lotto => ("로또 6/45", "보너스"),
            Game::Pension => ("연금복권 720+", "보너스 번호"),
        };
        let actual = ticket_text(&expected.winner);
        let (date, winning_bonus) = (expected.date, &expected.bonus);
        let header =
            format!("{label} {round}회 ({date})\n추첨 번호: {actual} / {bonus}: {winning_bonus}\n");
        assert_eq!(opening, header);
        let heading = format!("다음 {}회 예측 (recent-bias-v1 실험):\n", round + 1);
        let (comparison, following) = prior_text.split_once(&heading).context("next forecast")?;
        scores.compare(&expected, (comparison, &prior.tickets))?;
        let numbered = |line: &&str| line.starts_with(|c: char| c.is_ascii_digit());
        let listed = following.lines().filter(numbered);
        let stored = next.tickets.iter().enumerate();
        let stored = stored.map(|(i, t)| format!("{}. {}", i + 1, ticket_text(t)));
        assert!(listed.eq(stored), "{case}: next forecast differs from DB");

        if i == 7 {
            drop(service);
            service = start()?;
        }
        service.poll_with_clock(&|| now).await?;
        assert_eq!(feed.notices()?, notices, "{case}: repeated webhook");
        assert_eq!(forecast(&db, game, round)?, prior);
        assert_eq!(&forecast(&db, game, round + 1)?, next);
    }
    println!("Replay (40 trial tickets per game; histogram index = match count): {scores:?}");
    Ok(())
}
