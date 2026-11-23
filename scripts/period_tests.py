from __future__ import annotations

from dataclasses import dataclass
from typing import Final

import numpy as np
from numpy.typing import NDArray

Counts = NDArray[np.int64]
TRIALS: Final = 9_999
SEED: Final = 645_520_720


@dataclass(frozen=True, slots=True)
class Comparison:
    key: str
    rounds: Counts
    counts: Counts
    starts: tuple[int, ...]


@dataclass(frozen=True, slots=True)
class Result:
    key: str
    starts: tuple[int, ...]
    p_value: float


class InvalidPeriods(ValueError):
    def __init__(self, key: str) -> None:
        super().__init__(f'Empty or unmatched comparison period: {key}')


def period_test(comparison: Comparison, rng: np.random.Generator) -> Result:
    idx = np.searchsorted(comparison.rounds, comparison.starts)
    if (idx[0] != 0 or idx[-1] >= len(comparison.rounds)
            or np.any(np.diff(idx) <= 0) or not np.array_equal(comparison.rounds[idx], comparison.starts)):
        raise InvalidPeriods(comparison.key)
    size = np.diff(np.r_[idx, len(comparison.rounds)])
    pooled = comparison.counts.sum(axis=0)
    used = comparison.counts[:, pooled > 0]
    expected = size[:, None] * pooled[pooled > 0] / len(comparison.rounds)

    def statistic(counts: Counts) -> float:
        differences = np.add.reduceat(counts, idx, axis=0) - expected
        return float(np.sum(differences**2 / expected))

    observed = statistic(used)
    exceed = sum(statistic(used[rng.permutation(len(used))]) >= observed - 1e-10 for _ in range(TRIALS))
    return Result(comparison.key, comparison.starts, (1 + exceed) / (1 + TRIALS))


def holm(results: list[Result]) -> dict[str, float]:
    adjusted: dict[str, float] = {}
    floor = 0.0
    for index, item in enumerate(sorted(results, key=lambda row: row.p_value)):
        floor = max(floor, min(1.0, (len(results) - index) * item.p_value))
        adjusted[item.key] = floor
    return adjusted
