use anyhow::{Context, Result, ensure};

use crate::domain::{Game, Ticket};

pub(crate) const ALGORITHM: &str = "recent-bias-v1";
pub(crate) const MAX_HISTORY: u32 = 104;
pub(crate) const MIN_HISTORY: usize = 26;

pub(crate) enum Model {
    Lotto([f64; 45]),
    Pension {
        groups: [f64; 5],
        digits: [[f64; 10]; 6],
    },
}

fn lotto_mask(numbers: &[u8; 6]) -> Result<[bool; 45]> {
    let mut seen = [false; 45];
    for number in numbers {
        let slot = number
            .checked_sub(1)
            .and_then(|n| seen.get_mut(usize::from(n)))
            .context("lotto number out of range")?;
        ensure!(!*slot, "duplicate lotto number");
        *slot = true;
    }
    Ok(seen)
}

fn pension_values(group: u8, digits: &str) -> Result<(usize, [usize; 6])> {
    ensure!((1..=5).contains(&group), "pension group out of range");
    let bytes: [u8; 6] = digits
        .as_bytes()
        .try_into()
        .context("expected six pension digits")?;
    ensure!(
        bytes.iter().all(u8::is_ascii_digit),
        "non-numeric pension digit"
    );
    Ok((usize::from(group - 1), bytes.map(|b| usize::from(b - b'0'))))
}

fn estimate<const N: usize>(counts: [u32; N], samples: f64, uniform: f64) -> [f64; N] {
    counts.map(|count| 0.9 * uniform + 0.1 * (f64::from(count) / samples))
}

pub(crate) fn rank<const N: usize>(scores: &[f64; N]) -> [(usize, f64); N] {
    let mut values = std::array::from_fn(|i| (i, scores[i]));
    values.sort_unstable_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    values
}

impl Model {
    pub(crate) fn fit(game: Game, history: &[Ticket]) -> Result<Option<Self>> {
        let history = &history[history.len().saturating_sub(usize::try_from(MAX_HISTORY)?)..];
        if history.len() < MIN_HISTORY {
            return Ok(None);
        }
        let n = f64::from(u32::try_from(history.len())?);
        let model = match game {
            Game::Lotto => {
                let mut counts = [0; 45];
                for ticket in history {
                    let Ticket::Lotto { numbers } = ticket else {
                        anyhow::bail!("incompatible lotto history");
                    };
                    for (count, hit) in counts.iter_mut().zip(lotto_mask(numbers)?) {
                        *count += u32::from(hit);
                    }
                }
                Self::Lotto(estimate(counts, n, 6.0 / 45.0))
            }
            Game::Pension => {
                let mut groups = [0; 5];
                let mut digits = [[0; 10]; 6];
                for ticket in history {
                    let Ticket::Pension {
                        group_no,
                        digits: value,
                    } = ticket
                    else {
                        anyhow::bail!("incompatible pension history");
                    };
                    let (group, values) = pension_values(*group_no, value)?;
                    groups[group] += 1;
                    for (counts, digit) in digits.iter_mut().zip(values) {
                        counts[digit] += 1;
                    }
                }
                Self::Pension {
                    groups: estimate(groups, n, 0.2),
                    digits: digits.map(|c| estimate(c, n, 0.1)),
                }
            }
        };
        Ok(Some(model))
    }

    pub(crate) fn pick(&self) -> Result<Ticket> {
        match self {
            Self::Lotto(scores) => {
                let mut numbers = [0; 6];
                for (number, (i, _)) in numbers.iter_mut().zip(rank(scores)) {
                    *number = u8::try_from(i + 1)?;
                }
                numbers.sort_unstable();
                Ok(Ticket::Lotto { numbers })
            }
            Self::Pension { groups, digits } => {
                let group_no = u8::try_from(rank(groups)[0].0 + 1)?;
                let digits = digits
                    .iter()
                    .map(|scores| rank(scores)[0].0.to_string())
                    .collect();
                Ok(Ticket::Pension { group_no, digits })
            }
        }
    }

    pub(crate) fn score(&self, actual: &Ticket) -> Result<Score> {
        match (self, actual, self.pick()?) {
            (Self::Lotto(scores), Ticket::Lotto { numbers }, Ticket::Lotto { numbers: pick }) => {
                let loss = |p, hit| {
                    let d: f64 = p - f64::from(hit);
                    d * d
                };
                let indicators = lotto_mask(numbers)?;
                let model = scores
                    .iter()
                    .zip(indicators)
                    .map(|(&p, y)| loss(p, y))
                    .sum::<f64>()
                    / 45.0;
                let uniform = indicators.iter().map(|&y| loss(6.0 / 45.0, y)).sum::<f64>() / 45.0;
                let matches = pick.iter().filter(|n| numbers.contains(n)).count();
                Ok(Score {
                    model,
                    uniform,
                    matches: u32::try_from(matches)?,
                })
            }
            (
                Self::Pension { groups, digits },
                Ticket::Pension {
                    group_no,
                    digits: value,
                },
                Ticket::Pension {
                    group_no: pick_group,
                    digits: pick,
                },
            ) => {
                let (group, values) = pension_values(*group_no, value)?;
                let model = (-groups[group].ln()
                    - digits
                        .iter()
                        .zip(values)
                        .map(|(p, y)| p[y].ln())
                        .sum::<f64>())
                    / 7.0;
                let uniform = (5.0_f64.ln() + 6.0 * 10.0_f64.ln()) / 7.0;
                let matches = pick
                    .bytes()
                    .zip(value.bytes())
                    .filter(|(a, b)| a == b)
                    .count();
                Ok(Score {
                    model,
                    uniform,
                    matches: u32::try_from(matches)? + u32::from(pick_group == *group_no),
                })
            }
            _ => anyhow::bail!("incompatible forecast evaluation ticket"),
        }
    }
}

pub(crate) struct Score {
    pub(crate) model: f64,
    pub(crate) uniform: f64,
    pub(crate) matches: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranks_each_pension_position_and_keeps_leading_zero() -> Result<()> {
        let history = (0..MIN_HISTORY)
            .map(|_| Ticket::Pension {
                group_no: 4,
                digits: "013291".into(),
            })
            .collect::<Vec<_>>();
        assert!(Model::fit(Game::Pension, &history[..MIN_HISTORY - 1])?.is_none());
        let model = Model::fit(Game::Pension, &history)?.context("missing model")?;
        let Ticket::Pension { group_no, digits } = model.pick()? else {
            anyhow::bail!("wrong game");
        };
        assert_eq!((group_no, digits.as_str()), (4, "013291"));
        assert!(model.score(&history[0])?.model < model.score(&history[0])?.uniform);
        Ok(())
    }

    #[test]
    fn ranks_lotto_instead_of_picking_default_numbers() -> Result<()> {
        let numbers = [7, 10, 21, 29, 34, 45];
        let history = (0..MIN_HISTORY)
            .map(|_| Ticket::Lotto { numbers })
            .collect::<Vec<_>>();
        let model = Model::fit(Game::Lotto, &history)?.context("missing model")?;
        let Ticket::Lotto { numbers: pick } = model.pick()? else {
            anyhow::bail!("wrong game");
        };
        assert_eq!(pick, numbers);
        assert_eq!(model.score(&history[0])?.matches, 6);
        Ok(())
    }
}
