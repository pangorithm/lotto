use anyhow::{Result, bail, ensure};
use clap::ValueEnum;

use crate::domain::{Game, Ticket};
use crate::forecast::{ALGORITHM, MAX_HISTORY, MIN_HISTORY, Model};
use crate::jackpot_lotto::LottoJoint;
use crate::jackpot_pension::PensionJoint;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum Strategy {
    #[value(name = "recent-bias-v1")]
    Recent,
    #[value(name = "jackpot-joint-v2")]
    Jackpot,
}

impl Strategy {
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Recent => ALGORITHM,
            Self::Jackpot => "jackpot-joint-v2",
        }
    }

    pub(crate) fn from_name(name: &str) -> Result<Self> {
        match name {
            ALGORITHM => Ok(Self::Recent),
            "jackpot-joint-v2" => Ok(Self::Jackpot),
            _ => bail!("unsupported saved strategy: {name}"),
        }
    }

    pub(crate) fn picks(self, game: Game, history: &[Ticket]) -> Result<Option<Vec<Ticket>>> {
        if history.len() < MIN_HISTORY {
            return Ok(None);
        }
        match self {
            Self::Recent => Model::fit(game, history)?
                .map(|model| model.picks())
                .transpose(),
            Self::Jackpot => Ok(Some(Joint::fit(game, history)?.picks()?)),
        }
    }
}

pub(crate) enum Joint {
    Lotto(Box<LottoJoint>),
    Pension(Box<PensionJoint>),
}

impl Joint {
    pub(crate) fn fit(game: Game, history: &[Ticket]) -> Result<Self> {
        ensure!(
            history.len() >= MIN_HISTORY,
            "insufficient training history"
        );
        let history = &history[history.len().saturating_sub(usize::try_from(MAX_HISTORY)?)..];
        match game {
            Game::Lotto => Ok(Self::Lotto(Box::new(LottoJoint::fit(history)?))),
            Game::Pension => Ok(Self::Pension(Box::new(PensionJoint::fit(history)?))),
        }
    }

    pub(crate) fn probability(&self, ticket: &Ticket) -> Result<f64> {
        match self {
            Self::Lotto(model) => model.probability(ticket),
            Self::Pension(model) => model.probability(ticket),
        }
    }

    pub(crate) fn picks(&self) -> Result<Vec<Ticket>> {
        match self {
            Self::Lotto(model) => model.picks(),
            Self::Pension(model) => model.picks(),
        }
    }
}
