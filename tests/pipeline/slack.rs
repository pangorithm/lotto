use std::sync::{Arc, Mutex};

use anyhow::{Context, Result, ensure};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::process::Command;
use tokio::task::JoinSet;

use super::{Duration, Fixture, Output, Stdio, timeout};

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
            assert!(text.contains("이전 저장 후보 없음"));
            if round == 102 {
                assert!(text.contains("다음 103회 후보"));
            }
        }
    }
    Ok(())
}
