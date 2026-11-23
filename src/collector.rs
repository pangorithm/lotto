use std::collections::BTreeMap;
use std::time::Duration;

use anyhow::{Context, Result, ensure};
use reqwest::{Client, Url, header::ACCEPT, redirect::Policy};
use serde::de::DeserializeOwned;

use crate::domain::{Draw, Game, Head, Record};

#[path = "wire.rs"]
mod wire;

use wire::{Envelope, LottoHead, LottoList, LottoRow, PensionHead, PensionList, PensionRow};

const LOTTO_LATEST: &str = "/lt645/selectPstLt645Info.do";
const LOTTO_DETAIL: &str = "/lt645/selectPstLt645InfoNew.do";
const PENSION_LATEST: &str = "/pt720/selectPstPt720WnList.do";
const PENSION_DETAIL: &str = "/pt720/selectPstPt720Info.do";
const MAX_BODY_BYTES: usize = 1024 * 1024;

pub(crate) struct Collector {
    base: Url,
    client: Client,
}

impl Collector {
    pub(crate) fn new(base: Url) -> Result<Self> {
        ensure!(
            matches!(base.scheme(), "http" | "https") && base.has_host(),
            "lottery base URL must be HTTP(S) with a host"
        );
        ensure!(
            base.username().is_empty() && base.password().is_none(),
            "lottery base URL must not contain credentials"
        );
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(15))
            .pool_max_idle_per_host(2)
            .redirect(Policy::none())
            .user_agent(concat!(
                env!("CARGO_PKG_NAME"),
                "/",
                env!("CARGO_PKG_VERSION")
            ))
            .build()
            .context("build lottery HTTP client")?;
        Ok(Self { base, client })
    }

    pub(crate) async fn latest(&self, game: Game) -> Result<Head> {
        match game {
            Game::Lotto => {
                let url = self.base.join(LOTTO_LATEST)?;
                let data: LottoList<LottoHead> = self.get(&url).await?;
                data.list
                    .iter()
                    .max_by_key(|row| row.lt_epsd)
                    .context("empty Lotto latest list")?
                    .parse()
                    .context("invalid latest Lotto draw")
            }
            Game::Pension => {
                let url = self.base.join(PENSION_LATEST)?;
                let data: PensionList<PensionHead> = self.get(&url).await?;
                data.result
                    .iter()
                    .max_by_key(|row| row.pslt_epsd)
                    .context("empty pension latest list")?
                    .parse()
                    .context("invalid latest pension draw")
            }
        }
    }

    pub(crate) async fn fetch(&self, game: Game, from: u32, to: u32) -> Result<Vec<Record>> {
        ensure!(
            from > 0 && from <= to && to - from < 8,
            "expected 1 to 8 positive rounds, got {from}..={to}"
        );
        let range = from..=to;
        let mut records = match game {
            Game::Lotto => {
                let mut url = self.base.join(LOTTO_DETAIL)?;
                let center = from.midpoint(to).to_string();
                url.query_pairs_mut()
                    .append_pair("srchDir", "center")
                    .append_pair("srchLtEpsd", &center);
                let data: LottoList<LottoRow> = self.get(&url).await?;
                data.list
                    .into_iter()
                    .filter(|row| range.contains(&row.head.lt_epsd.get()))
                    .map(|row| {
                        let (head, draw) = row
                            .parse()
                            .with_context(|| format!("invalid Lotto round {}", row.head.lt_epsd))?;
                        Ok(Record {
                            head,
                            draw: Draw::Lotto(draw),
                            source_url: url.clone(),
                        })
                    })
                    .collect::<Result<Vec<_>>>()?
            }
            Game::Pension => {
                let mut url = self.base.join(PENSION_DETAIL)?;
                url.query_pairs_mut()
                    .append_pair("srchStrPsltEpsd", &from.to_string())
                    .append_pair("srchEndPsltEpsd", &to.to_string());
                let data: PensionList<PensionRow> = self.get(&url).await?;
                let mut by_round = BTreeMap::<_, Vec<_>>::new();
                for row in data.result {
                    let round = row.head.pslt_epsd.get();
                    ensure!(range.contains(&round), "unexpected pension round {round}");
                    by_round.entry(round).or_default().push(row);
                }
                by_round
                    .into_iter()
                    .map(|(round, rows)| {
                        let (head, prizes) = wire::pension_draw(rows)
                            .with_context(|| format!("invalid pension round {round}"))?;
                        Ok(Record {
                            head,
                            draw: Draw::Pension(Box::new(prizes)),
                            source_url: url.clone(),
                        })
                    })
                    .collect::<Result<Vec<_>>>()?
            }
        };
        records.sort_unstable_by_key(|record| record.head.round);
        ensure!(
            records.iter().map(|record| record.head.round).eq(range),
            "{} detail response did not contain exactly rounds {from}..={to}",
            game.code()
        );
        Ok(records)
    }

    async fn get<T: DeserializeOwned>(&self, url: &Url) -> Result<T> {
        let mut response = self
            .client
            .get(url.clone())
            .header(ACCEPT, "application/json")
            .send()
            .await
            .with_context(|| format!("GET {url}"))?;
        let status = response.status();
        ensure!(status.is_success(), "HTTP {status} from {url}");

        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .with_context(|| format!("read response from {url}"))?
        {
            ensure!(
                chunk.len() <= MAX_BODY_BYTES.saturating_sub(body.len()),
                "response exceeds {MAX_BODY_BYTES} bytes from {url}"
            );
            body.extend_from_slice(&chunk);
        }
        let start = body
            .iter()
            .copied()
            .find(|byte| !byte.is_ascii_whitespace());
        ensure!(start == Some(b'{'), "expected a JSON object from {url}");
        let envelope: Envelope<T> =
            serde_json::from_slice(&body).with_context(|| format!("decode JSON from {url}"))?;
        Ok(envelope.data)
    }
}
