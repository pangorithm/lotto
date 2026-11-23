use anyhow::{Result, bail, ensure};

use crate::domain::Ticket;

const LAST_LABEL: u8 = 45;
const BALL_COUNT: usize = LAST_LABEL as usize;
const DRAW_SIZE: usize = 6;
const PICK_COUNT: usize = 5;
const MAX_DRAWS: u32 = 104;

/// Joint distribution of the six main lotto balls, without the bonus ball.
pub(crate) struct LottoJoint {
    balls: [u32; BALL_COUNT],
    pairs: PairWeights,
    joint: Summary,
}

impl LottoJoint {
    pub(crate) fn fit(history: &[Ticket]) -> Result<Self> {
        let n = u32::try_from(history.len())?;
        ensure!((1..=MAX_DRAWS).contains(&n), "invalid lotto history length");

        let mut counts = [0; BALL_COUNT];
        let mut adjacent_pairs = 0;
        for ticket in history {
            let LottoSet(numbers) = LottoSet::parse(ticket)?;
            for label in numbers {
                counts[usize::from(label - 1)] += 1;
            }
            for pair in numbers.windows(2) {
                adjacent_pairs += u32::from(pair[1] == pair[0] + 1);
            }
        }

        // Ball weight = 3(6n + 5count)/(20n). The adjacent pair factor is
        // (18n + 3*adjacent_pairs)/(20n), and a gap contributes 1. Dropping
        // the common factors at each chosen count leaves integer weights.
        let balls = counts.map(|count| 6 * n + 5 * count);
        let pairs = PairWeights {
            adjacent: 18 * n + 3 * adjacent_pairs,
            separated: 20 * n,
        };
        let joint = summarize(&balls, pairs);
        Ok(Self {
            balls,
            pairs,
            joint,
        })
    }

    pub(crate) fn probability(&self, ticket: &Ticket) -> Result<f64> {
        let LottoSet(numbers) = LottoSet::parse(ticket)?;
        let mut previous = 0;
        let mut weight = 1.0;
        for (chosen, label) in numbers.into_iter().enumerate() {
            let ball = self.balls[usize::from(label - 1)];
            weight *= f64::from(ball * self.pairs.multiplier(chosen, label == previous + 1));
            previous = label;
        }
        Ok(weight / self.joint.mass)
    }

    /// Five highest-weight sets, with lexicographically smaller numbers first on ties.
    pub(crate) fn picks(&self) -> Result<Vec<Ticket>> {
        let mut tickets = Vec::new();
        tickets.try_reserve_exact(PICK_COUNT)?;
        tickets.extend(self.joint.best.iter().flatten().map(|pick| Ticket::Lotto {
            numbers: pick.numbers,
        }));
        Ok(tickets)
    }
}

struct LottoSet([u8; DRAW_SIZE]);

impl LottoSet {
    fn parse(ticket: &Ticket) -> Result<Self> {
        let mut numbers = match ticket {
            Ticket::Lotto { numbers } => *numbers,
            Ticket::Pension { .. } => bail!("expected lotto ticket"),
        };
        numbers.sort_unstable();
        ensure!(
            numbers[0] > 0 && numbers[DRAW_SIZE - 1] <= LAST_LABEL,
            "lotto number out of range"
        );
        ensure!(
            numbers.windows(2).all(|pair| pair[0] < pair[1]),
            "duplicate lotto number"
        );
        Ok(Self(numbers))
    }
}

#[derive(Clone, Copy)]
struct PairWeights {
    adjacent: u32,
    separated: u32,
}

impl PairWeights {
    const fn multiplier(self, chosen: usize, adjacent: bool) -> u32 {
        if chosen == 0 {
            1
        } else if adjacent {
            self.adjacent
        } else {
            self.separated
        }
    }
}

#[derive(Clone, Copy)]
struct Ranked {
    weight: u128,
    numbers: [u8; DRAW_SIZE],
}

#[derive(Clone, Copy, Default)]
struct Summary {
    mass: f64,
    best: [Option<Ranked>; PICK_COUNT],
}

impl Summary {
    fn merge(&mut self, other: Self) {
        self.mass += other.mass;
        for pick in other.best.into_iter().flatten() {
            let position = self.best.partition_point(|slot| {
                slot.is_some_and(|entry| {
                    entry.weight > pick.weight
                        || (entry.weight == pick.weight && entry.numbers < pick.numbers)
                })
            });
            if position < PICK_COUNT {
                self.best
                    .copy_within(position..PICK_COUNT - 1, position + 1);
                self.best[position] = Some(pick);
            }
        }
    }
}

fn summarize(balls: &[u32], pairs: PairWeights) -> Summary {
    // One path per set; each state is indexed by chosen count and whether the
    // previous label was included. All prefixes in a state share the same
    // possible suffixes and multipliers, so only five are needed for ranking.
    let mut states = [[Summary::default(); 2]; DRAW_SIZE + 1];
    states[0][0].mass = 1.0;
    states[0][0].best[0] = Some(Ranked {
        weight: 1,
        numbers: [0; DRAW_SIZE],
    });

    for (label, &ball) in (1..=LAST_LABEL).zip(balls) {
        let mut next = [[Summary::default(); 2]; DRAW_SIZE + 1];
        for (chosen, endings) in states.into_iter().enumerate() {
            for (previous, mut state) in endings.into_iter().enumerate() {
                next[chosen][0].merge(state);
                if chosen < DRAW_SIZE {
                    let factor = ball * pairs.multiplier(chosen, previous == 1);
                    state.mass *= f64::from(factor);
                    for pick in state.best.iter_mut().flatten() {
                        // At n <= 104 the product is bounded by 1144^6 * 3432^5.
                        pick.weight *= u128::from(factor);
                        pick.numbers[chosen] = label;
                    }
                    next[chosen + 1][1].merge(state);
                }
            }
        }
        states = next;
    }

    let [mut skipped, included] = states[DRAW_SIZE];
    skipped.merge(included);
    skipped
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contiguous_history() -> [Ticket; 26] {
        std::array::from_fn(|_| Ticket::Lotto {
            numbers: [6, 1, 3, 2, 4, 5],
        })
    }

    #[test]
    fn partition_and_top_five_match_exhaustive_enumeration() {
        let balls = [4_u32, 4, 4, 4, 4, 3, 3, 3, 4, 4, 4, 4, 4];
        for (adjacent, separated) in [(3_u32, 2_u32), (2, 3)] {
            let mut brute = Vec::new();
            for mask in 0..(1_u32 << balls.len()) {
                if mask.count_ones() != 6 {
                    continue;
                }
                let mut numbers = [0; DRAW_SIZE];
                let labels = (1..=13_u8).filter(|&label| mask & (1 << (label - 1)) != 0);
                for (i, label) in labels.enumerate() {
                    numbers[i] = label;
                }
                let pairs = (mask & (mask >> 1)).count_ones();
                let weight = numbers
                    .map(|label| balls[usize::from(label - 1)])
                    .into_iter()
                    .product::<u32>()
                    * adjacent.pow(pairs)
                    * separated.pow(5 - pairs);
                brute.push((weight, numbers));
            }
            let expected_mass: f64 = brute.iter().map(|&(weight, _)| f64::from(weight)).sum();
            brute.sort_unstable_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));

            let actual = summarize(
                &balls,
                PairWeights {
                    adjacent,
                    separated,
                },
            );

            assert!((expected_mass / actual.mass - 1.0).abs() < 1e-12);
            assert_eq!(
                actual.best.map(|pick| pick.map(|pick| pick.numbers)),
                std::array::from_fn(|i| Some(brute[i].1))
            );
        }
    }

    #[test]
    fn probabilities_use_learned_factors_and_ignore_number_order() -> Result<()> {
        let history = contiguous_history();
        let model = LottoJoint::fit(&history)?;
        let sorted = Ticket::Lotto {
            numbers: [1, 2, 3, 4, 5, 6],
        };
        let disjoint = Ticket::Lotto {
            numbers: [8, 10, 12, 14, 16, 45],
        };

        let contiguous = model.probability(&sorted)?;
        let expected_ratio = (1.65_f64 / 0.9).powi(6) * 1.65_f64.powi(5);
        assert!((contiguous / model.probability(&disjoint)? / expected_ratio - 1.0).abs() < 1e-12);
        assert!((model.probability(&history[0])? / contiguous - 1.0).abs() < 1e-12);
        Ok(())
    }

    #[test]
    fn picks_return_five_distinct_sorted_sets_with_lexicographic_ties() -> Result<()> {
        let model = LottoJoint::fit(&contiguous_history())?;
        let expected = [
            [1, 2, 3, 4, 5, 6],
            [2, 3, 4, 5, 6, 7],
            [1, 2, 3, 4, 5, 7],
            [1, 2, 3, 4, 5, 8],
            [1, 2, 3, 4, 5, 9],
        ];

        let picks = model.picks()?;

        assert_eq!(picks, expected.map(|numbers| Ticket::Lotto { numbers }));
        Ok(())
    }

    #[test]
    fn malformed_history_and_scoring_tickets_return_errors() -> Result<()> {
        let mut history = contiguous_history();
        let model = LottoJoint::fit(&history)?;

        assert!(LottoJoint::fit(&[]).is_err());
        for ticket in [
            Ticket::Lotto {
                numbers: [0, 1, 2, 3, 4, 5],
            },
            Ticket::Lotto {
                numbers: [1, 2, 3, 4, 5, 255],
            },
            Ticket::Lotto {
                numbers: [2, 3, 1, 4, 5, 2],
            },
            Ticket::Pension {
                group_no: 1,
                digits: "123456".into(),
            },
        ] {
            assert!(model.probability(&ticket).is_err(), "{ticket:?}");
            history[25] = ticket;
            assert!(LottoJoint::fit(&history).is_err(), "{:?}", history[25]);
        }
        Ok(())
    }
}
