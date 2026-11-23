use anyhow::{Result, ensure};

use crate::domain::Ticket;
use crate::forecast::{Model, rank};

pub(crate) const PICKS: usize = 5;

impl Model {
    pub(crate) fn picks(&self) -> Result<Vec<Ticket>> {
        match self {
            Self::Lotto(scores) => lotto_picks(scores),
            Self::Pension { groups, digits } => pension_picks(groups, digits),
        }
    }
}

fn lotto_picks(scores: &[f64; 45]) -> Result<Vec<Ticket>> {
    let ranked = rank(scores);
    let top = &ranked[..6 + PICKS - 1];
    let mut choices = Vec::new();
    for a in 0..top.len() {
        for b in a + 1..top.len() {
            for c in b + 1..top.len() {
                for d in c + 1..top.len() {
                    for e in d + 1..top.len() {
                        for f in e + 1..top.len() {
                            let indices = [a, b, c, d, e, f];
                            let score = indices.into_iter().map(|i| top[i].1).sum::<f64>();
                            choices.push((score, indices));
                        }
                    }
                }
            }
        }
    }
    choices.sort_unstable_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
    choices
        .into_iter()
        .take(PICKS)
        .map(|(_, indices)| {
            let mut numbers = [0; 6];
            for (number, i) in numbers.iter_mut().zip(indices) {
                *number = u8::try_from(top[i].0 + 1)?;
            }
            numbers.sort_unstable();
            Ok(Ticket::Lotto { numbers })
        })
        .collect()
}

fn pension_picks(groups: &[f64; 5], digits: &[[f64; 10]; 6]) -> Result<Vec<Ticket>> {
    let group_ranks = rank(groups);
    let digit_ranks = digits.map(|d| rank(&d));
    let columns =
        std::iter::once(group_ranks.as_slice()).chain(digit_ranks.iter().map(<[_; 10]>::as_slice));
    let mut prefixes = vec![(0.0_f64, [0; 7])];
    for (position, column) in columns.enumerate() {
        let mut expanded = Vec::new();
        for (score, prefix) in &prefixes {
            for (ordering, (_, probability)) in column.iter().take(PICKS).enumerate() {
                let mut indices = *prefix;
                indices[position] = ordering;
                expanded.push((score + probability.ln(), indices));
            }
        }
        expanded.sort_unstable_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
        expanded.truncate(PICKS);
        prefixes = expanded;
    }
    ensure!(
        prefixes.len() == PICKS,
        "incorrect pension projection count"
    );
    prefixes
        .into_iter()
        .map(|(_, [group, a, b, c, d, e, f])| {
            let group_no = u8::try_from(group_ranks[group].0 + 1)?;
            let digits = digit_ranks
                .iter()
                .zip([a, b, c, d, e, f])
                .map(|(column, i)| column[i].0.to_string())
                .collect();
            Ok(Ticket::Pension { group_no, digits })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn both_models_return_five_distinct_tickets_without_changing_the_top_pick() -> Result<()> {
        let lotto = Model::Lotto(std::array::from_fn(|i| if i < 12 { 0.2 } else { 0.1 }));
        let pension = Model::Pension {
            groups: [0.2; 5],
            digits: [[0.1; 10]; 6],
        };
        for model in [lotto, pension] {
            let tickets = model.picks()?;
            let unique = tickets
                .iter()
                .map(serde_json::to_string)
                .collect::<serde_json::Result<HashSet<_>>>()?;
            assert_eq!((tickets.len(), unique.len()), (5, 5));
            assert_eq!(
                serde_json::to_value(tickets.first())?,
                serde_json::to_value(model.pick()?)?
            );
        }
        Ok(())
    }
}
