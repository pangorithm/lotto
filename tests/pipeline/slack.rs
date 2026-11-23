use std::sync::{Arc, Mutex};

use anyhow::{Context, Result, ensure};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::process::Command;
use tokio::task::JoinSet;

use super::{Duration, Fixture, Output, Stdio, timeout};

pub(super) fn assert_candidate(status: &Value, code: &str, date: chrono::NaiveDate) -> Result<()> {
    let game = &status[code];
    assert_eq!(
        game["latest"],
        serde_json::json!({"round":102,"date":date.to_string()})
    );
    assert!(game.get("candidate").is_none());
    let candidate = &game["model_candidate"];
    assert_eq!(candidate["target_round"], 103);
    assert_eq!(
        candidate["target_date"],
        (date + chrono::Duration::days(7)).to_string()
    );
    assert_eq!(candidate["based_on_round"], 102);
    assert_eq!(candidate["algorithm"], "recent-bias-v1");
    assert!(candidate["created_at_utc"].as_str().is_some());
    let tickets = candidate["tickets"].as_array().context("model tickets")?;
    assert_eq!(tickets.len(), 5);
    for (index, ticket) in tickets.iter().enumerate() {
        assert!(!tickets[..index].contains(ticket), "duplicate model ticket");
        if code == "lotto645" {
            let numbers = ticket["numbers"].as_array().context("Lotto numbers")?;
            assert_eq!(numbers.len(), 6);
            let numbers: Vec<u64> = numbers
                .iter()
                .map(|n| n.as_u64().context("numeric Lotto number"))
                .collect::<Result<_>>()?;
            assert!(
                numbers.iter().all(|n| (1..=45).contains(n))
                    && numbers.windows(2).all(|p| p[0] < p[1])
            );
        } else {
            assert!((1..=5).contains(&ticket["group_no"].as_u64().context("pension group")?));
            let digits = ticket["digits"].as_str().context("pension digits")?;
            assert!(digits.len() == 6 && digits.bytes().all(|b| b.is_ascii_digit()));
        }
    }
    Ok(())
}

async fn once(fixture: &Fixture) -> Result<Output> {
    let mut command = Command::new(env!("CARGO_BIN_EXE_lottery-daemon"));
    command
        .arg("--db")
        .arg(fixture.dir.path().join("runtime.sqlite3"))
        .arg("--seed")
        .arg(fixture.dir.path().join("seed.sqlite3"))
        .arg("--base-url")
        .arg(&fixture.base)
        .arg("--slack-webhook-file")
        .arg(fixture.dir.path().join("webhook"))
        .arg("once")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    Ok(timeout(Duration::from_secs(30), command.output()).await??)
}

#[tokio::test]
async fn retries_failed_slack_notice_and_never_replays_sent_notices() -> Result<()> {
    let fixture = Fixture::new(false).await?;
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    std::fs::write(
        fixture.dir.path().join("webhook"),
        format!("http://{}/services/test", listener.local_addr()?),
    )?;
    let messages = Arc::new(Mutex::new(Vec::<Value>::new()));
    let captured = Arc::clone(&messages);
    let mut server: JoinSet<Result<()>> = JoinSet::new();
    server.spawn(async move {
        loop {
            let (stream, _) = listener.accept().await?;
            let mut reader = BufReader::new(stream);
            let mut line = String::new();
            reader.read_line(&mut line).await?;
            ensure!(line.starts_with("POST /services/test "), "unexpected webhook request");
            let mut length = None;
            loop {
                line.clear();
                reader.read_line(&mut line).await?;
                if line == "\r\n" { break; }
                if let Some((name, value)) = line.split_once(':') {
                    if name.eq_ignore_ascii_case("content-length") { length = Some(value.trim().parse::<usize>()?); }
                }
            }
            let mut body = vec![0; length.context("missing content length")?];
            reader.read_exact(&mut body).await?;
            let failed = {
                let mut messages = captured.lock().map_err(|_| anyhow::anyhow!("capture lock poisoned"))?;
                messages.push(serde_json::from_slice(&body)?);
                messages.len() == 1
            };
            let response = if failed { "HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n" } else { "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok" };
            reader.get_mut().write_all(response.as_bytes()).await?;
        }
    });
    let first = once(&fixture).await?;
    assert!(!first.status.success(), "HTTP 500 must fail the poll");
    let db = fixture.db()?;
    let first_counts: (u32, u32) = db.query_row(
        "SELECT count(*), count(sent_at_utc) FROM notice_outbox",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(first_counts, (4, 2));
    let retry = once(&fixture).await?;
    assert!(
        retry.status.success(),
        "{}",
        String::from_utf8_lossy(&retry.stderr)
    );
    let sent: (u32, u32) = db.query_row(
        "SELECT count(*), count(sent_at_utc) FROM notice_outbox",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(sent, (4, 4));
    let restart = once(&fixture).await?;
    assert!(
        restart.status.success(),
        "{}",
        String::from_utf8_lossy(&restart.stderr)
    );
    let status = fixture.status().await?;
    let messages = messages
        .lock()
        .map_err(|_| anyhow::anyhow!("capture lock poisoned"))?;
    assert_eq!(messages.len(), 5);
    assert_eq!(messages[0], messages[3]);
    for game in ["로또 6/45", "연금복권 720+"] {
        for round in [101, 102] {
            let matching: Vec<&str> = messages
                .iter()
                .filter_map(|value| value["text"].as_str())
                .filter(|text| text.contains(game) && text.contains(&format!("{round}회")))
                .collect();
            assert_eq!(
                matching.len(),
                if game == "로또 6/45" && round == 101 {
                    2
                } else {
                    1
                }
            );
            let text = matching[0];
            assert!(text.contains("추첨 번호:"));
            assert!(!text.contains("uniform-v1"));
            assert!(!text.contains("균등 무작위"));
            if round == 102 {
                let next = text
                    .split_once("다음 103회 예측")
                    .context("next forecast")?
                    .1;
                let code = if game == "로또 6/45" {
                    "lotto645"
                } else {
                    "pension720"
                };
                let tickets = status[code]["model_candidate"]["tickets"]
                    .as_array()
                    .context("model tickets")?;
                assert_eq!(tickets.len(), 5);
                let mut remainder = next;
                for (index, ticket) in tickets.iter().enumerate() {
                    let value = if code == "lotto645" {
                        let numbers = ticket["numbers"].as_array().context("lotto numbers")?;
                        format!(
                            "{:?}",
                            numbers
                                .iter()
                                .map(|n| n.as_u64().context("lotto number"))
                                .collect::<Result<Vec<_>>>()?
                        )
                    } else {
                        format!(
                            "{}조 {}",
                            ticket["group_no"].as_u64().context("pension group")?,
                            ticket["digits"].as_str().context("pension digits")?
                        )
                    };
                    let label = format!("{}. {value}", index + 1);
                    remainder = remainder
                        .split_once(&label)
                        .context("ordered forecast ticket")?
                        .1;
                }
            } else {
                assert!(text.contains("이전 저장 예측 없음"));
            }
        }
    }
    Ok(())
}
