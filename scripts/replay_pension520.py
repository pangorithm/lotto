#!/usr/bin/env python3
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
# How to run: python3 scripts/replay_pension520.py
"""Offline pension520 research replay, an analogue of recent-bias-v1."""

from __future__ import annotations

import json
import sqlite3
from collections import Counter, deque
from collections.abc import Iterable
from contextlib import closing
from dataclasses import dataclass
from math import log
from pathlib import Path
from typing import Final, NewType

SERIES: Final = 'pension520'
WINDOW_DRAWS: Final = 104
WARMUP_DRAWS: Final = 26
PICKS: Final = 5
GROUPS: Final = range(1, 8)
DOMAINS: Final = (GROUPS, range(1, 10)) + (range(10),) * 5
TIER_COUNTS: Final = dict(zip('1234567', (2, 4, 1, 1, 1, 2, 2), strict=True))

RoundNo = NewType('RoundNo', int)
GroupNo = NewType('GroupNo', int)


@dataclass(frozen=True, slots=True)
class Ticket:
    group: GroupNo
    digits: str


@dataclass(frozen=True, slots=True)
class Pattern:
    tier: str
    group: GroupNo | None
    suffix: str


class InvalidDraw(ValueError):
    def __init__(self, round_no: RoundNo, detail: str) -> None:
        super().__init__(f'{SERIES} round {round_no}: {detail}')


def read_draw(
    db: sqlite3.Connection, round_no: RoundNo,
) -> tuple[list[Pattern], tuple[Ticket, ...]]:
    rows: list[tuple[str, int | None, str]] = db.execute(
        'SELECT tier, group_no, digits FROM pension_number '
        'WHERE series_code = ? AND round_no = ? ORDER BY tier, slot',
        (SERIES, round_no),
    ).fetchall()
    if Counter(tier for tier, _, _ in rows) != TIER_COUNTS:
        raise InvalidDraw(round_no, 'unexpected pattern counts by tier')

    widths = (6, 6, 6, 5, 4 if round_no < 66 else 3, 2, 1)
    patterns: list[Pattern] = []
    first_prizes: list[Ticket] = []
    for tier, group_value, digits in rows:
        if len(digits) != widths[int(tier) - 1] or not (digits.isascii() and digits.isdigit()):
            raise InvalidDraw(round_no, f'invalid tier {tier} digits: {digits!r}')
        if group_value is not None and group_value not in GROUPS:
            raise InvalidDraw(round_no, f'invalid tier {tier} group: {group_value!r}')
        group = None if group_value is None else GroupNo(group_value)
        if tier == '1':
            if group is None or digits.startswith('0'):
                raise InvalidDraw(round_no, 'first prize must be a valid grouped ticket')
            first_prizes.append(Ticket(group, digits))
        patterns.append(Pattern(tier, group, digits))
    return patterns, tuple(first_prizes)


def choose_tickets(history: Iterable[tuple[Ticket, ...]]) -> list[Ticket]:
    """Fit independent group and digit marginals, then beam-search joint picks."""
    observations = [(ticket.group, *map(int, ticket.digits)) for draw in history for ticket in draw]
    beam: list[tuple[float, tuple[int, ...]]] = [(0.0, ())]
    for position, domain in enumerate(DOMAINS):
        counts = Counter(values[position] for values in observations)
        fair = 1.0 / len(domain)
        column = sorted(
            (-log(0.9 * fair + 0.1 * (counts[value] / len(observations))), value)
            for value in domain
        )[:PICKS]
        # Ascending cost is descending log probability; ties prefer smaller categories.
        beam = sorted(
            (cost + marginal, (*prefix, value))
            for cost, prefix in beam
            for marginal, value in column
        )[:PICKS]
    return [Ticket(GroupNo(group), ''.join(map(str, digits))) for _, (group, *digits) in beam]


def best_rank(ticket: Ticket, patterns: Iterable[Pattern]) -> str:
    return min((
        pattern.tier for pattern in patterns
        if (pattern.group is None or pattern.group == ticket.group)
        and ticket.digits.endswith(pattern.suffix)
    ), default='miss')


def main() -> None:
    snapshot = Path(__file__).resolve().parents[1] / 'lottery.sqlite3'
    history: deque[tuple[Ticket, ...]] = deque(maxlen=WINDOW_DRAWS)
    with closing(sqlite3.connect(f'{snapshot.as_uri()}?mode=ro&immutable=1', uri=True)) as db:
        rounds: list[tuple[int]] = db.execute(
            'SELECT round_no FROM draw WHERE series_code = ? ORDER BY round_no', (SERIES,),
        ).fetchall()
        for (number,) in rounds:
            round_no = RoundNo(number)
            tickets = choose_tickets(history) if len(history) >= WARMUP_DRAWS else []
            patterns, first_prizes = read_draw(db, round_no)
            ranks = [best_rank(ticket, patterns) for ticket in tickets]
            result = {'series': SERIES, 'round': round_no, 'ranks': ranks}
            print('PERIOD_REPLAY ' + json.dumps(result, separators=(',', ':')))
            history.append(first_prizes)


if __name__ == '__main__':
    main()
