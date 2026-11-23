use std::num::NonZeroU32;

use anyhow::{Context, Result, anyhow, ensure};
use chrono::NaiveDate;
use serde::Deserialize;

use crate::domain::{Game, Head, LottoDraw, LottoPrize, PensionPrize};

#[derive(Deserialize)]
pub(super) struct Envelope<T> {
    pub(super) data: T,
}

#[derive(Deserialize)]
pub(super) struct LottoList<T> {
    pub(super) list: Vec<T>,
}

#[derive(Deserialize)]
pub(super) struct PensionList<T> {
    pub(super) result: Vec<T>,
}

fn ascii_digits(value: &str, len: usize) -> bool {
    value.len() == len && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn draw_head(game: Game, round: NonZeroU32, raw_date: &str) -> Result<Head> {
    ensure!(
        ascii_digits(raw_date, 8),
        "expected an eight-digit draw date"
    );
    let date = NaiveDate::parse_from_str(raw_date, "%Y%m%d").context("invalid draw date")?;
    game.validate_date(date)?;
    Ok(Head {
        round: round.get(),
        date,
    })
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct LottoHead {
    pub(super) lt_epsd: NonZeroU32,
    lt_rfl_ymd: String,
}

impl LottoHead {
    pub(super) fn parse(&self) -> Result<Head> {
        draw_head(Game::Lotto, self.lt_epsd, &self.lt_rfl_ymd)
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct LottoRow {
    #[serde(flatten)]
    pub(super) head: LottoHead,
    tm1_wn_no: u8,
    tm2_wn_no: u8,
    tm3_wn_no: u8,
    tm4_wn_no: u8,
    tm5_wn_no: u8,
    tm6_wn_no: u8,
    bns_wn_no: u8,
    whol_epsd_sum_ntsl_amt: u64,
    rnk1_wn_nope: u64,
    rnk1_wn_amt: u64,
    rnk1_sum_wn_amt: u64,
    rnk2_wn_nope: u64,
    rnk2_wn_amt: u64,
    rnk2_sum_wn_amt: u64,
    rnk3_wn_nope: u64,
    rnk3_wn_amt: u64,
    rnk3_sum_wn_amt: u64,
    rnk4_wn_nope: u64,
    rnk4_wn_amt: u64,
    rnk4_sum_wn_amt: u64,
    rnk5_wn_nope: u64,
    rnk5_wn_amt: u64,
    rnk5_sum_wn_amt: u64,
}

impl LottoRow {
    pub(super) fn parse(&self) -> Result<(Head, LottoDraw)> {
        let head = self.head.parse()?;
        let numbers = [
            self.tm1_wn_no,
            self.tm2_wn_no,
            self.tm3_wn_no,
            self.tm4_wn_no,
            self.tm5_wn_no,
            self.tm6_wn_no,
        ];
        ensure!(
            numbers.iter().all(|number| (1..=45).contains(number))
                && numbers.windows(2).all(|pair| pair[0] < pair[1]),
            "Lotto main numbers must be strictly ascending within 1..=45"
        );
        let bonus = self.bns_wn_no;
        ensure!(
            (1..=45).contains(&bonus) && !numbers.contains(&bonus),
            "invalid Lotto bonus number"
        );
        let prizes = [
            (self.rnk1_wn_nope, self.rnk1_wn_amt, self.rnk1_sum_wn_amt),
            (self.rnk2_wn_nope, self.rnk2_wn_amt, self.rnk2_sum_wn_amt),
            (self.rnk3_wn_nope, self.rnk3_wn_amt, self.rnk3_sum_wn_amt),
            (self.rnk4_wn_nope, self.rnk4_wn_amt, self.rnk4_sum_wn_amt),
            (self.rnk5_wn_nope, self.rnk5_wn_amt, self.rnk5_sum_wn_amt),
        ]
        .map(|(winners, unit_krw, total_krw)| LottoPrize {
            winners,
            unit_krw,
            total_krw,
        });
        Ok((
            head,
            LottoDraw {
                numbers,
                bonus,
                sales_krw: self.whol_epsd_sum_ntsl_amt,
                prizes,
            },
        ))
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PensionHead {
    pub(super) pslt_epsd: NonZeroU32,
    pslt_rfl_ymd: String,
}

impl PensionHead {
    pub(super) fn parse(&self) -> Result<Head> {
        draw_head(Game::Pension, self.pslt_epsd, &self.pslt_rfl_ymd)
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PensionRow {
    #[serde(flatten)]
    pub(super) head: PensionHead,
    wn_sq_no: u8,
    wn_bnd_no: Option<String>,
    wn_rnk_vl: String,
    wn_amt: u64,
}

pub(super) fn pension_draw(mut rows: Vec<PensionRow>) -> Result<(Head, [PensionPrize; 8])> {
    const TIERS: [(u8, &str, usize, Option<u16>); 8] = [
        (1, "1", 6, Some(240)),
        (2, "2", 6, Some(120)),
        (3, "3", 5, None),
        (4, "4", 4, None),
        (5, "5", 3, None),
        (6, "6", 2, None),
        (7, "7", 1, None),
        (21, "bonus", 6, Some(120)),
    ];

    ensure!(rows.len() == TIERS.len(), "expected eight pension tiers");
    rows.sort_unstable_by_key(|row| row.wn_sq_no);
    let first = &rows[0];
    let head = first.head.parse()?;
    ensure!(
        rows.iter()
            .all(|row| row.head.pslt_rfl_ymd == first.head.pslt_rfl_ymd),
        "pension tiers disagree on the draw date"
    );

    let prizes = rows
        .into_iter()
        .zip(TIERS)
        .map(|(row, (rank, tier, digit_count, months))| {
            ensure!(
                row.wn_sq_no == rank,
                "expected pension rank {rank}, got {}",
                row.wn_sq_no
            );
            ensure!(
                ascii_digits(&row.wn_rnk_vl, digit_count),
                "invalid pension {tier} digits"
            );
            let group = match (rank, row.wn_bnd_no.as_deref().map(str::as_bytes)) {
                (1, Some([digit @ b'1'..=b'5'])) => Some(*digit - b'0'),
                (1, _) => return Err(anyhow!("pension rank 1 requires a group from 1 to 5")),
                (_, None) => None,
                (_, Some(_)) => return Err(anyhow!("pension {tier} must not have a group")),
            };
            let installments = u64::from(months.unwrap_or(1));
            ensure!(
                row.wn_amt % installments == 0,
                "invalid pension {tier} payment amount"
            );
            Ok(PensionPrize {
                tier,
                group,
                digits: row.wn_rnk_vl,
                unit_krw: row.wn_amt / installments,
                months,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let prizes: [PensionPrize; 8] = prizes
        .try_into()
        .map_err(|_| anyhow!("expected eight pension tiers"))?;
    ensure!(
        prizes[1..7]
            .iter()
            .all(|prize| prizes[0].digits.ends_with(prize.digits.as_str())),
        "pension ranks 2 through 7 must match suffixes of rank 1"
    );
    Ok((head, prizes))
}
