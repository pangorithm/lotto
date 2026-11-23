use anyhow::{Context, Result, ensure};
use chrono::{DateTime, Datelike, FixedOffset, NaiveDate, Utc, Weekday};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Game {
    Lotto,
    Pension,
}

impl Game {
    pub(crate) const ALL: [Self; 2] = [Self::Lotto, Self::Pension];

    pub(crate) const fn code(self) -> &'static str {
        match self {
            Self::Lotto => "lotto645",
            Self::Pension => "pension720",
        }
    }

    pub(crate) fn validate_date(self, date: NaiveDate) -> Result<()> {
        let expected = match self {
            Self::Lotto => Weekday::Sat,
            Self::Pension => Weekday::Thu,
        };
        ensure!(
            date.weekday() == expected,
            "unexpected draw weekday for {}: {date}",
            self.code()
        );
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub(crate) struct Head {
    pub(crate) round: u32,
    pub(crate) date: NaiveDate,
}

#[derive(Debug)]
pub(crate) struct LottoPrize {
    pub(crate) winners: u64,
    pub(crate) unit_krw: u64,
    pub(crate) total_krw: u64,
}

#[derive(Debug)]
pub(crate) struct LottoDraw {
    pub(crate) numbers: [u8; 6],
    pub(crate) bonus: u8,
    pub(crate) sales_krw: u64,
    pub(crate) prizes: [LottoPrize; 5],
}

#[derive(Debug)]
pub(crate) struct PensionPrize {
    pub(crate) tier: &'static str,
    pub(crate) group: Option<u8>,
    pub(crate) digits: String,
    pub(crate) unit_krw: u64,
    pub(crate) months: Option<u16>,
}

#[derive(Debug)]
pub(crate) enum Draw {
    Lotto(LottoDraw),
    Pension(Box<[PensionPrize; 8]>),
}

impl Draw {
    pub(crate) const fn game(&self) -> Game {
        match self {
            Self::Lotto(_) => Game::Lotto,
            Self::Pension(_) => Game::Pension,
        }
    }
}

#[derive(Debug)]
pub(crate) struct Record {
    pub(crate) head: Head,
    pub(crate) draw: Draw,
    pub(crate) source_url: reqwest::Url,
}

#[derive(Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub(crate) enum Ticket {
    Lotto { numbers: [u8; 6] },
    Pension { group_no: u8, digits: String },
}

#[derive(Debug)]
pub(crate) enum NoticeResult {
    Lotto {
        numbers: [u8; 6],
        bonus: u8,
    },
    Pension {
        group_no: u8,
        digits: String,
        bonus: String,
    },
}

#[derive(Debug)]
pub(crate) struct Notice {
    pub(crate) game: Game,
    pub(crate) head: Head,
    pub(crate) result: NoticeResult,
    pub(crate) model_prior: Vec<Ticket>,
    pub(crate) model_next: Vec<Ticket>,
    pub(crate) model_next_algorithm: Option<String>,
}

#[derive(Debug)]
pub(crate) struct Candidate {
    pub(crate) game: Game,
    pub(crate) based_on: Head,
    pub(crate) target: Head,
    pub(crate) created_at: DateTime<Utc>,
}

pub(crate) fn seoul_date(now: DateTime<Utc>) -> Result<NaiveDate> {
    let offset =
        FixedOffset::east_opt(9 * 60 * 60).context("invalid Korea Standard Time offset")?;
    Ok(now.with_timezone(&offset).date_naive())
}
