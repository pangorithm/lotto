use std::collections::BTreeMap;
use std::num::NonZeroU32;
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, ensure};
use reqwest::{Method, StatusCode, Url};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::{JoinSet, spawn_blocking};
use tokio::time::timeout;

const IO_TIMEOUT: Duration = Duration::from_secs(5);

pub(crate) struct ReplayFeed {
    url: Url,
    fixture: Arc<Fixture>,
    _server: JoinSet<Result<()>>,
}

#[derive(Clone, Copy)]
struct Release {
    lotto: u32,
    pension: u32,
}

impl ReplayFeed {
    pub(crate) async fn start() -> Result<Self> {
        let (lotto, pension) = spawn_blocking(|| {
            Ok::<_, anyhow::Error>((
                Archive::load("lotto/all.json", "list", "ltEpsd")?,
                Archive::load("pension720/prizes-1-342.json", "result", "psltEpsd")?,
            ))
        })
        .await??;
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let url = Url::parse(&format!("http://{}/", listener.local_addr()?))?;
        let fixture = Arc::new(Fixture {
            lotto,
            pension,
            release: Mutex::new(Release {
                lotto: 1243,
                pension: 334,
            }),
            notices: Mutex::default(),
        });
        let mut server = JoinSet::new();
        let serving = Arc::clone(&fixture);
        server.spawn(async move {
            loop {
                let (stream, _) = listener.accept().await?;
                if let Err(error) = serving.serve(stream).await {
                    tracing::debug!(%error, "replay connection failed");
                }
            }
        });
        Ok(Self {
            url,
            fixture,
            _server: server,
        })
    }

    pub(crate) fn url(&self) -> &Url {
        &self.url
    }

    pub(crate) fn reveal(&self, lotto: u32, pension: u32) {
        *self
            .fixture
            .release
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Release { lotto, pension };
    }

    pub(crate) fn notices(&self) -> Result<Vec<String>> {
        let notices = self
            .fixture
            .notices
            .lock()
            .map_err(|_| anyhow!("replay notice capture lock poisoned"))?;
        Ok(notices.clone())
    }
}

struct Archive {
    list: &'static str,
    rows: BTreeMap<u32, Vec<Value>>,
}

impl Archive {
    fn load(file: &str, list: &'static str, round_key: &str) -> Result<Self> {
        let path = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/data/raw")).join(file);
        let bytes = std::fs::read(&path).with_context(|| format!("read {}", path.display()))?;
        let mut raw: Value =
            serde_json::from_slice(&bytes).with_context(|| format!("decode {}", path.display()))?;
        let recorded = raw
            .get_mut("data")
            .and_then(|data| data.get_mut(list))
            .and_then(Value::as_array_mut)
            .with_context(|| format!("expected data.{list} array in {file}"))?;
        let mut rows = BTreeMap::<_, Vec<_>>::new();
        for row in recorded.drain(..) {
            let round = row[round_key]
                .as_u64()
                .and_then(|round| NonZeroU32::try_from(u32::try_from(round).ok()?).ok())
                .with_context(|| format!("invalid {round_key} in {file}"))?;
            rows.entry(round.get()).or_default().push(row);
        }
        Ok(Self { list, rows })
    }

    fn latest(&self, cutoff: u32) -> Vec<u8> {
        let latest = self.rows.range(..=cutoff).next_back();
        let row = latest.and_then(|(_, rows)| rows.first());
        self.envelope(row.into_iter().collect())
    }

    fn range(&self, from: u32, to: u32) -> Vec<u8> {
        let rows = if from <= to {
            self.rows
                .range(from..=to)
                .flat_map(|(_, rows)| rows)
                .collect()
        } else {
            Vec::new()
        };
        self.envelope(rows)
    }

    fn envelope(&self, rows: Vec<&Value>) -> Vec<u8> {
        json!({"resultCode": null, "resultMessage": null, "data": { (self.list): rows }})
            .to_string()
            .into_bytes()
    }
}

struct Fixture {
    lotto: Archive,
    pension: Archive,
    release: Mutex<Release>,
    notices: Mutex<Vec<String>>,
}

impl Fixture {
    async fn serve(&self, stream: TcpStream) -> Result<()> {
        let mut reader = BufReader::new(stream);
        let reply = match timeout(IO_TIMEOUT, Request::read(&mut reader)).await {
            Ok(Ok(request)) => self.reply(request),
            Ok(Err(_)) => Err(StatusCode::BAD_REQUEST),
            Err(_) => Err(StatusCode::REQUEST_TIMEOUT),
        };
        let (status, content_type, body) = match reply {
            Ok((content_type, body)) => (StatusCode::OK, content_type, body),
            Err(status) => (status, "text/plain", Vec::new()),
        };
        let mut response = format!(
            "HTTP/1.1 {} {}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            status.as_u16(),
            status.canonical_reason().unwrap_or_default(),
            body.len()
        )
        .into_bytes();
        response.extend(body);
        timeout(IO_TIMEOUT, reader.get_mut().write_all(&response))
            .await?
            .context("write replay HTTP response")
    }

    fn reply(&self, request: Request) -> std::result::Result<(&'static str, Vec<u8>), StatusCode> {
        let release = *self.release.lock().unwrap_or_else(PoisonError::into_inner);
        let query_round = |name: &str| {
            request
                .url
                .query_pairs()
                .find(|(key, _)| key == name)
                .and_then(|(_, value)| value.parse::<NonZeroU32>().ok())
                .map(NonZeroU32::get)
                .ok_or(StatusCode::BAD_REQUEST)
        };
        let body = match (request.method, request.url.path()) {
            (Method::GET, "/lt645/selectPstLt645Info.do") => self.lotto.latest(release.lotto),
            (Method::GET, "/pt720/selectPstPt720WnList.do") => self.pension.latest(release.pension),
            (Method::GET, "/lt645/selectPstLt645InfoNew.do") => {
                if !request
                    .url
                    .query_pairs()
                    .any(|(key, value)| key == "srchDir" && value == "center")
                {
                    return Err(StatusCode::BAD_REQUEST);
                }
                let center = query_round("srchLtEpsd")?;
                // Collector rounds down the midpoint of its batches of up to eight draws.
                let from = center.saturating_sub(3);
                let to = center.saturating_add(4).min(release.lotto);
                self.lotto.range(from, to)
            }
            (Method::GET, "/pt720/selectPstPt720Info.do") => {
                let from = query_round("srchStrPsltEpsd")?;
                let to = query_round("srchEndPsltEpsd")?;
                if from > to {
                    return Err(StatusCode::BAD_REQUEST);
                }
                self.pension.range(from, to.min(release.pension))
            }
            (Method::POST, "/services/replay/test/token") => {
                #[derive(Deserialize)]
                struct Webhook {
                    text: String,
                }
                let message: Webhook =
                    serde_json::from_slice(&request.body).map_err(|_| StatusCode::BAD_REQUEST)?;
                self.notices
                    .lock()
                    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
                    .push(message.text);
                return Ok(("text/plain", b"ok".to_vec()));
            }
            _ => return Err(StatusCode::NOT_FOUND),
        };
        Ok(("application/json", body))
    }
}

struct Request {
    method: Method,
    url: Url,
    body: Vec<u8>,
}

impl Request {
    async fn read(reader: &mut BufReader<TcpStream>) -> Result<Self> {
        let mut headers = (&mut *reader).take(8 * 1024);
        let mut line = String::new();
        headers.read_line(&mut line).await?;
        ensure!(
            line.ends_with("\r\n"),
            "missing HTTP request line terminator"
        );
        let mut words = line.split_ascii_whitespace();
        let method = words.next().context("missing HTTP method")?.parse()?;
        let target = words.next().context("missing HTTP target")?;
        ensure!(target.starts_with('/'), "expected a local HTTP target");
        ensure!(
            matches!(words.next(), Some("HTTP/1.0" | "HTTP/1.1")) && words.next().is_none(),
            "invalid HTTP request line"
        );
        let url = Url::parse(&format!("http://localhost{target}"))?;

        let mut length = None;
        loop {
            line.clear();
            headers.read_line(&mut line).await?;
            ensure!(line.ends_with("\r\n"), "missing HTTP header terminator");
            if line == "\r\n" {
                break;
            }
            let (name, value) = line.split_once(':').context("invalid HTTP header")?;
            ensure!(
                !name.eq_ignore_ascii_case("transfer-encoding"),
                "expected a fixed-length body"
            );
            if name.eq_ignore_ascii_case("content-length") {
                ensure!(length.is_none(), "duplicate HTTP content length");
                length = Some(value.trim().parse::<usize>()?);
            }
        }
        let length = length.unwrap_or(0);
        ensure!(length <= 64 * 1024, "replay request body too large");
        let mut body = vec![0; length];
        reader.read_exact(&mut body).await?;
        Ok(Self { method, url, body })
    }
}
