use std::process::{Output, Stdio};
use std::time::Duration;

use anyhow::{Context, Result, ensure};
use chrono::{Datelike, Duration as Days, FixedOffset, NaiveDate, Utc, Weekday};
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::process::Command;
use tokio::task::JoinSet;
use tokio::time::timeout;

#[path = "pipeline/slack.rs"]
mod slack;

struct Fixture {
    dir: TempDir,
    lotto: NaiveDate,
    pension: NaiveDate,
    base: String,
    server: JoinSet<Result<()>>,
}

impl Fixture {
    async fn new(bad_lotto: bool) -> Result<Self> {
        let dir = tempfile::tempdir()?;
        let offset = FixedOffset::east_opt(9 * 3600).context("KST offset")?;
        let today = Utc::now().with_timezone(&offset).date_naive();
        let latest = |day: Weekday| {
            today
                - Days::days(i64::from(
                    (today.weekday().num_days_from_monday() + 7 - day.num_days_from_monday()) % 7,
                ))
        };
        let lotto = latest(Weekday::Sat);
        let pension = latest(Weekday::Thu);
        let db = Connection::open(dir.path().join("seed.sqlite3"))?;
        db.execute_batch(include_str!("../schema.sql"))?;
        db.execute_batch("INSERT INTO series VALUES ('lotto645','Lotto','2002-12-07',NULL), ('pension720','Pension','2020-05-07',NULL);
            INSERT INTO source (id,url,publisher,title,kind,accessed_on) VALUES ('seed','https://example.org/','fixture','fixture','third_party','2020-01-01');")?;
        for round in 75..=100 {
            for (code, date) in [("lotto645", lotto), ("pension720", pension)] {
                db.execute("INSERT INTO draw (series_code,round_no,draw_date,source_id,source_locator) VALUES (?1,?2,?3,'seed','seed')", params![code, round, (date - Days::days(i64::from(102 - round) * 7)).to_string()])?;
            }
            db.execute(
                "INSERT INTO lotto_result VALUES ('lotto645',?1,1,2,3,4,5,6,7)",
                [round],
            )?;
            db.execute(
                "INSERT INTO pension_number VALUES ('pension720',?1,'1',1,1,'000123')",
                [round],
            )?;
        }
        drop(db);

        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let base = format!("http://{}/", listener.local_addr()?);
        let mut server = JoinSet::new();
        server.spawn(async move {
            loop {
                let (stream, _) = listener.accept().await?;
                let mut reader = BufReader::new(stream);
                let mut line = String::new();
                timeout(Duration::from_secs(5), reader.read_line(&mut line)).await??;
                let path = line.split_whitespace().nth(1).context("HTTP request target")?;
                let body = if path.starts_with("/lt645/selectPstLt645InfoNew.do") {
                    if bad_lotto { "<html>unavailable</html>".to_owned() } else {
                        json!({"data":{"list":[lotto_row(101, lotto - Days::days(7)), lotto_row(102, lotto)]}}).to_string()
                    }
                } else if path.starts_with("/lt645/selectPstLt645Info.do") {
                    json!({"data":{"list":[{"ltEpsd":102,"ltRflYmd":lotto.format("%Y%m%d").to_string()}]}}).to_string()
                } else if path.starts_with("/pt720/selectPstPt720Info.do") {
                    let rows: Vec<Value> = [(101, pension - Days::days(7)), (102, pension)].into_iter().flat_map(|(round, date)| pension_rows(round, date)).collect();
                    json!({"data":{"result":rows}}).to_string()
                } else if path.starts_with("/pt720/selectPstPt720WnList.do") {
                    json!({"data":{"result":[{"psltEpsd":102,"psltRflYmd":pension.format("%Y%m%d").to_string()}]}}).to_string()
                } else {
                    anyhow::bail!("unexpected request: {path}");
                };
                let response = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body);
                timeout(Duration::from_secs(5), reader.get_mut().write_all(response.as_bytes())).await??;
            }
        });
        Ok(Self {
            dir,
            lotto,
            pension,
            base,
            server,
        })
    }

    async fn cli(&self, action: &str) -> Result<Output> {
        let mut command = Command::new(env!("CARGO_BIN_EXE_lottery-daemon"));
        command
            .arg("--db")
            .arg(self.dir.path().join("runtime.sqlite3"))
            .arg("--seed")
            .arg(self.dir.path().join("seed.sqlite3"))
            .arg("--base-url")
            .arg(&self.base)
            .arg(action)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        Ok(timeout(Duration::from_secs(30), command.output())
            .await
            .context("CLI timed out")??)
    }

    async fn status(&self) -> Result<Value> {
        let output = self.cli("status").await?;
        ensure!(
            output.status.success(),
            "status failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        Ok(serde_json::from_slice(&output.stdout)?)
    }

    fn db(&self) -> Result<Connection> {
        Ok(Connection::open(self.dir.path().join("runtime.sqlite3"))?)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort_all();
    }
}

fn lotto_row(round: u32, date: NaiveDate) -> Value {
    let mut row = json!({"ltEpsd":round,"ltRflYmd":date.format("%Y%m%d").to_string(),
        "tm1WnNo":1,"tm2WnNo":2,"tm3WnNo":3,"tm4WnNo":4,"tm5WnNo":5,"tm6WnNo":6,
        "bnsWnNo":7,"wholEpsdSumNtslAmt":100000});
    for rank in 1..=5 {
        row[format!("rnk{rank}WnNope")] = json!(2);
        row[format!("rnk{rank}WnAmt")] = json!(1000);
        row[format!("rnk{rank}SumWnAmt")] = json!(2000);
    }
    row
}

fn pension_rows(round: u32, date: NaiveDate) -> Vec<Value> {
    [
        (1, "000123", Some("2"), 1680000000_u64),
        (2, "000123", None, 120000000),
        (3, "00123", None, 1000000),
        (4, "0123", None, 100000),
        (5, "123", None, 50000),
        (6, "23", None, 5000),
        (7, "3", None, 1000),
        (21, "000456", None, 120000000),
    ]
    .into_iter()
    .map(|(rank, digits, group, amount)| {
        json!({"psltEpsd":round,
            "psltRflYmd":date.format("%Y%m%d").to_string(),"wnSqNo":rank,
            "wnBndNo":group,"wnRnkVl":digits,"wnAmt":amount})
    })
    .collect()
}

use slack::assert_candidate;

#[tokio::test]
async fn appends_both_feeds_and_preserves_candidates_on_repeat() -> Result<()> {
    let fixture = Fixture::new(false).await?;
    let first = fixture.cli("once").await?;
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let status = fixture.status().await?;
    assert_candidate(&status, "lotto645", fixture.lotto)?;
    assert_candidate(&status, "pension720", fixture.pension)?;
    let repeated = fixture.cli("once").await?;
    assert!(
        repeated.status.success(),
        "{}",
        String::from_utf8_lossy(&repeated.stderr)
    );
    let again = fixture.status().await?;
    for code in ["lotto645", "pension720"] {
        assert_eq!(
            status[code]["model_candidate"],
            again[code]["model_candidate"]
        );
        let db = fixture.db()?;
        let rounds: Vec<u32> = db
            .prepare("SELECT round_no FROM draw WHERE series_code=?1 ORDER BY round_no")?
            .query_map([code], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        assert_eq!(rounds, (75..=102).collect::<Vec<_>>());
        let count: u32 = db.query_row(
            "SELECT count(*) FROM prediction WHERE series_code=?1",
            [code],
            |row| row.get(0),
        )?;
        assert_eq!(count, 0);
    }
    let db = fixture.db()?;
    let digits: String = db.query_row("SELECT digits FROM pension_number WHERE series_code='pension720' AND round_no=102 AND tier='1'", [], |row| row.get(0))?;
    assert_eq!(digits, "000123");
    let tiers: u32 = db.query_row(
        "SELECT count(*) FROM pension_number WHERE series_code='pension720' AND round_no=102",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(tiers, 8);
    Ok(())
}

#[tokio::test]
async fn malformed_lotto_detail_preserves_head_while_pension_succeeds() -> Result<()> {
    let fixture = Fixture::new(true).await?;
    let output = fixture.cli("once").await?;
    assert!(!output.status.success());
    let status = fixture.status().await?;
    assert_eq!(
        status["lotto645"]["latest"],
        json!({"round":100,"date":(fixture.lotto - Days::days(14)).to_string()})
    );
    assert!(status["lotto645"].get("candidate").is_none());
    assert_candidate(&status, "pension720", fixture.pension)?;
    let db = fixture.db()?;
    let lotto: u32 = db.query_row(
        "SELECT count(*) FROM draw WHERE series_code='lotto645'",
        [],
        |row| row.get(0),
    )?;
    let pension: u32 = db.query_row(
        "SELECT count(*) FROM draw WHERE series_code='pension720'",
        [],
        |row| row.get(0),
    )?;
    let candidates: u32 = db.query_row(
        "SELECT count(*) FROM prediction WHERE series_code='lotto645'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!((lotto, pension, candidates), (26, 28, 0));
    Ok(())
}
