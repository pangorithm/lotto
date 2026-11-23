use std::cmp::Ordering;

use anyhow::{Context, Result, bail, ensure};

use crate::domain::Ticket;

const GROUP_COUNT: usize = 5;
const DIGIT_COUNT: usize = 10;
const POSITIONS: usize = 6;
const PICK_COUNT: usize = 5;
const PRIOR_WEIGHT: f64 = 80.0;

/// A joint distribution over the group and six ordered digits of a Pension 720+ ticket.
pub(crate) struct PensionJoint {
    groups: [f64; GROUP_COUNT],
    first_digit: [[f64; DIGIT_COUNT]; GROUP_COUNT],
    transitions: [[[f64; DIGIT_COUNT]; DIGIT_COUNT]; POSITIONS - 1],
}

fn pension_indices(ticket: &Ticket) -> Result<(usize, [usize; POSITIONS])> {
    let (group_no, digits) = match ticket {
        Ticket::Pension { group_no, digits } => (*group_no, digits),
        Ticket::Lotto { .. } => bail!("expected a pension ticket"),
    };
    let group = usize::from(group_no);
    ensure!(
        (1..=GROUP_COUNT).contains(&group),
        "pension group out of range"
    );
    let bytes: [u8; POSITIONS] = digits
        .as_bytes()
        .try_into()
        .context("expected six pension digits")?;
    ensure!(
        bytes.iter().all(u8::is_ascii_digit),
        "non-ASCII pension digit"
    );
    Ok((group - 1, bytes.map(|b| usize::from(b - b'0'))))
}

fn mixed_marginal<const N: usize>(counts: [u32; N], n: f64, uniform_mass: f64) -> [f64; N] {
    counts.map(|count| uniform_mass + 0.1 * (f64::from(count) / n))
}

fn conditional_row(counts: [u32; DIGIT_COUNT], marginal: [f64; DIGIT_COUNT]) -> [f64; DIGIT_COUNT] {
    let context_count = counts.iter().sum::<u32>();
    std::array::from_fn(|digit| {
        (PRIOR_WEIGHT * marginal[digit] + f64::from(counts[digit]))
            / (PRIOR_WEIGHT + f64::from(context_count))
    })
}

#[derive(Clone, Copy)]
struct Prefix {
    group: usize,
    digits: usize,
    probability: f64,
}

impl Prefix {
    fn rank(&self, other: &Self) -> Ordering {
        other
            .probability
            .total_cmp(&self.probability)
            .then_with(|| (self.group, self.digits).cmp(&(other.group, other.digits)))
    }
}

impl PensionJoint {
    /// Fits smoothed marginals and conditionals from first-prize tickets.
    ///
    /// # Errors
    /// Returns an error for empty or malformed history, or a history too large to count in `u32`.
    pub(crate) fn fit(history: &[Ticket]) -> Result<Self> {
        ensure!(!history.is_empty(), "empty pension history");
        let n = f64::from(u32::try_from(history.len()).context("pension history too large")?);
        let mut group_counts = [0; GROUP_COUNT];
        let mut digit_counts = [[0; DIGIT_COUNT]; POSITIONS];
        let mut first_counts = [[0; DIGIT_COUNT]; GROUP_COUNT];
        let mut transition_counts = [[[0; DIGIT_COUNT]; DIGIT_COUNT]; POSITIONS - 1];

        for (index, ticket) in history.iter().enumerate() {
            let (group, digits) = pension_indices(ticket)
                .with_context(|| format!("invalid pension history entry {}", index + 1))?;
            group_counts[group] += 1;
            for (counts, digit) in digit_counts.iter_mut().zip(digits) {
                counts[digit] += 1;
            }
            let [first, rest @ ..] = digits;
            first_counts[group][first] += 1;
            let mut previous = first;
            for (counts, digit) in transition_counts.iter_mut().zip(rest) {
                counts[previous][digit] += 1;
                previous = digit;
            }
        }

        let [first, rest @ ..] = digit_counts.map(|counts| mixed_marginal(counts, n, 0.9 / 10.0));
        Ok(Self {
            groups: mixed_marginal(group_counts, n, 0.9 / 5.0),
            first_digit: first_counts.map(|row| conditional_row(row, first)),
            transitions: std::array::from_fn(|i| {
                transition_counts[i].map(|row| conditional_row(row, rest[i]))
            }),
        })
    }

    /// Scores a complete Pension 720+ ticket under the fitted joint distribution.
    ///
    /// # Errors
    /// Returns an error if the ticket is not a valid Pension 720+ ticket.
    pub(crate) fn probability(&self, ticket: &Ticket) -> Result<f64> {
        let (group, digits) = pension_indices(ticket)?;
        Ok(self.joint_probability(group, digits))
    }

    fn joint_probability(&self, group: usize, digits: [usize; POSITIONS]) -> f64 {
        let first = self.groups[group] * self.first_digit[group][digits[0]];
        self.transitions
            .iter()
            .zip(digits.windows(2))
            .fold(first, |p, (table, pair)| p * table[pair[0]][pair[1]])
    }

    /// Returns the five highest ranked tickets, breaking ties by group and then digits.
    ///
    /// # Errors
    /// Returns an error if a ranked group cannot be represented as a ticket group number.
    pub(crate) fn picks(&self) -> Result<Vec<Ticket>> {
        let mut states: [[Prefix; PICK_COUNT]; DIGIT_COUNT] = std::array::from_fn(|digit| {
            std::array::from_fn(|group| Prefix {
                group,
                digits: digit,
                probability: self.groups[group] * self.first_digit[group][digit],
            })
        });

        for table in &self.transitions {
            // Prefixes ending in the same digit have identical suffix options and weights.
            states = std::array::from_fn(|digit| {
                let mut candidates: [_; DIGIT_COUNT * PICK_COUNT] = std::array::from_fn(|i| {
                    let previous = i / PICK_COUNT;
                    let prefix = states[previous][i % PICK_COUNT];
                    Prefix {
                        group: prefix.group,
                        digits: prefix.digits * DIGIT_COUNT + digit,
                        probability: prefix.probability * table[previous][digit],
                    }
                });
                candidates.sort_unstable_by(Prefix::rank);
                std::array::from_fn(|i| candidates[i])
            });
        }

        states.as_flattened_mut().sort_unstable_by(Prefix::rank);
        states
            .into_iter()
            .flatten()
            .take(PICK_COUNT)
            .map(|prefix| {
                let digits = prefix.digits;
                Ok(Ticket::Pension {
                    group_no: u8::try_from(prefix.group + 1)?,
                    digits: format!("{digits:06}"),
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ticket(group_no: u8, digits: &str) -> Ticket {
        Ticket::Pension {
            group_no,
            digits: digits.into(),
        }
    }

    #[test]
    fn probability_multiplies_the_group_and_position_specific_conditionals() -> Result<()> {
        let history = [ticket(1, "012345"), ticket(2, "112345")];
        let model = PensionJoint::fit(&history)?;
        let expected = 0.23
            * ((80.0 * 0.14 + 1.0) / 81.0)
            * ((80.0 * 0.19 + 1.0) / 81.0)
            * ((80.0_f64 * 0.19 + 2.0) / 82.0).powi(4);
        assert!((model.probability(&history[0])? - expected).abs() < 1e-14);
        Ok(())
    }

    #[test]
    fn top_five_and_joint_mass_match_exhaustive_enumeration() -> Result<()> {
        // Six common first digits outrank 0 initially, but each has a spread-out
        // second digit. The rarer 0 leads to a concentrated suffix.
        let mut history = Vec::new();
        for first in 1..=6 {
            for rest in 0..10 {
                let digits = format!("{first}{}", rest.to_string().repeat(5));
                history.extend((0..3).map(|_| ticket(1, &digits)));
            }
        }
        history.extend((0..24).map(|_| ticket(1, "099999")));
        let model = PensionJoint::fit(&history)?;

        let mut ranked: Vec<(f64, usize, usize)> = Vec::with_capacity(6);
        let mut mass = 0.0;
        for group in 0..5 {
            for code in 0..1_000_000 {
                let digits = [100_000, 10_000, 1_000, 100, 10, 1].map(|place| code / place % 10);
                let p = model.joint_probability(group, digits);
                mass += p;
                // Enumeration is lexicographic, so equal scores already precede this ticket.
                if ranked.len() < 5 || p > ranked[4].0 {
                    let insert_at = ranked.partition_point(|&(other, _, _)| other >= p);
                    ranked.insert(insert_at, (p, group, code));
                    ranked.truncate(5);
                }
            }
        }
        let expected = ranked
            .into_iter()
            .map(|(_, group, code)| Ok(ticket(u8::try_from(group + 1)?, &format!("{code:06}"))))
            .collect::<Result<Vec<_>>>()?;
        assert_eq!(model.picks()?, expected);
        assert!((mass - 1.0).abs() < 1e-9, "joint mass was {mass}");
        Ok(())
    }

    #[test]
    fn tied_tickets_sort_by_group_then_digits() -> Result<()> {
        let history = (1..=5)
            .flat_map(|group| (0..10).map(move |digit| ticket(group, &digit.to_string().repeat(6))))
            .collect::<Vec<_>>();
        let expected =
            ["000000", "111111", "222222", "333333", "444444"].map(|digits| ticket(1, digits));
        assert_eq!(PensionJoint::fit(&history)?.picks()?, expected);
        Ok(())
    }

    #[test]
    fn fitting_and_scoring_reject_malformed_tickets() -> Result<()> {
        assert!(PensionJoint::fit(&[]).is_err());
        let valid = ticket(5, "012345");
        let model = PensionJoint::fit(&[valid])?;
        for invalid in [
            Ticket::Lotto {
                numbers: [1, 2, 3, 4, 5, 6],
            },
            ticket(0, "012345"),
            ticket(6, "012345"),
            ticket(1, "01234"),
            ticket(1, "0123456"),
            ticket(1, "01234x"),
            ticket(1, "012\u{FF11}"),
        ] {
            assert!(model.probability(&invalid).is_err(), "{invalid:?}");
            assert!(PensionJoint::fit(&[invalid]).is_err());
        }
        Ok(())
    }
}
