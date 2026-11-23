#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = ["numpy>=2.0,<3"]
# ///
# ─── How to run ───
# uv run scripts/analyze_periods.py > research/period-statistics.json
from __future__ import annotations

import hashlib
import json
import sqlite3
from collections import defaultdict
from contextlib import closing
from dataclasses import asdict, dataclass
from math import comb
from pathlib import Path
from typing import Final

import numpy as np

from period_tests import SEED, TRIALS, Comparison, holm, period_test

ROOT: Final = Path(__file__).resolve().parents[1]
PERIOD_VIEWS: Final = (('dated', True), ('confirmed', False))


@dataclass(frozen=True, slots=True)
class Period:
    start: int
    end: int
    start_date: str
    end_date: str
    draws: int
    events: tuple[tuple[str, str, str, str], ...]


def periods(db: sqlite3.Connection, series: str, include_dated_candidates: bool) -> list[Period]:
    events: defaultdict[int, list[tuple[str, str, str, str]]] = defaultdict(list)
    for start, key, title, status, role in db.execute(
        "SELECT first_round,id,title,status,round_role FROM v_event_timeline WHERE series_code=? AND ("
        "(round_role='first_affected' AND status='confirmed') OR "
        "(? AND date_precision='day' AND ((round_role='first_affected' AND status='reported') OR "
        "(id='lotto_live_10' AND round_role='observed_in')))) ORDER BY first_round,id",
        (series, include_dated_candidates),
    ):
        events[start].append((key, title, status, role))
    date_by_round = dict(db.execute('SELECT round_no,draw_date FROM draw WHERE series_code=?', (series,)))
    starts = sorted(set(events) | {min(date_by_round)})
    ends = [x - 1 for x in starts[1:]] + [max(date_by_round)]
    return [Period(start, end, date_by_round[start], date_by_round[end], end - start + 1, tuple(events[start]))
            for start, end in zip(starts, ends, strict=True)]


def lotto(db: sqlite3.Connection, tests: list[Comparison]) -> dict[str, list[dict]]:
    rows = np.array(db.execute(
        'SELECT round_no,n1,n2,n3,n4,n5,n6,bonus,total_sales_krw,winning_games,unit_krw '
        "FROM v_lotto_draws JOIN prize USING(round_no) WHERE series_code='lotto645' AND tier='1' ORDER BY round_no"
    ).fetchall(), dtype=np.int64)
    rounds, numbers, bonus = rows[:, 0], rows[:, 1:7], rows[:, 7]
    main_features = np.eye(45, dtype=np.int64)[numbers - 1].sum(axis=1)
    bonus_features = np.eye(45, dtype=np.int64)[bonus - 1]
    previous_overlap = np.einsum('ij,ij->i', main_features[1:], main_features[:-1])
    report: dict[str, list[dict]] = {}
    for view, dated in PERIOD_VIEWS:
        slices = periods(db, 'lotto645', dated)
        starts = tuple(x.start for x in slices)
        tests.append(Comparison(f'lotto_main_{view}', rounds, main_features, starts))
        tests.append(Comparison(f'lotto_bonus_{view}', rounds, bonus_features, starts))
        report[view] = []
        for period in slices:
            lo, hi = period.start - 1, period.end
            selected = numbers[lo:hi]
            sales, wins, unit = rows[lo:hi, 8:].T
            games = sales / (2_000 if period.start < 88 else 1_000)
            report[view].append({**asdict(period),
                'mean_sales_krw': float(np.mean(sales)), 'mean_games_sold': float(np.mean(games)),
                'mean_first_wins': float(np.mean(wins)), 'zero_first_wins': int(np.count_nonzero(wins == 0)),
                'first_wins_per_million_games': float(1_000_000 * wins.sum() / games.sum()),
                'median_unit_krw_when_won': float(np.median(unit[wins > 0])),
                'main_counts_1_to_45': main_features[lo:hi].sum(axis=0).tolist(),
                'bonus_counts_1_to_45': bonus_features[lo:hi].sum(axis=0).tolist(),
                'mean_sum': float(np.mean(selected.sum(axis=1))),
                'odd_counts_0_to_6': np.bincount((selected % 2).sum(axis=1), minlength=7).tolist(),
                'any_consecutive': int(np.count_nonzero((np.diff(selected, axis=1) == 1).any(axis=1))),
                'previous_overlap_0_to_6': np.bincount(previous_overlap[lo:hi - 1], minlength=7).tolist(),
            })
    for name, left, boundary, right in (('lotto_10_observed', 1, 10, 87), ('lotto_262', 88, 262, 400),
                                        ('lotto_733_reported', 575, 733, 835), ('lotto_836', 733, 836, 1100)):
        tests.append(Comparison(name, rounds[left-1:right], main_features[left-1:right], (left, boundary)))
    report['one_off'] = [{'round': n, 'main': numbers[n-1].tolist(), 'bonus': int(bonus[n-1]),
                          'sum': int(numbers[n-1].sum()), 'previous_overlap': int(previous_overlap[n-2]),
                          'event': event}
                         for n, event in db.execute(
        "SELECT first_round,id FROM v_event_timeline WHERE series_code='lotto645' AND round_role='observed_in' "
        "AND effect IN ('mechanism','observation') ORDER BY first_round,id")]
    return report


def pension(db: sqlite3.Connection, series: str, tests: list[Comparison]) -> dict[str, list[dict]]:
    legacy = series == 'pension520'
    rows = db.execute("SELECT round_no,group_no,digits FROM v_pension_draws WHERE series_code=? AND tier='1' "
                      "ORDER BY round_no,slot", (series,)).fetchall()
    picks, groups = (2, 7) if legacy else (1, 5)
    rounds = np.array([row[0] for row in rows[::picks]], dtype=np.int64)
    digits = np.array([list(row[2]) for row in rows], dtype=np.int64).reshape(-1, picks, 6)
    digit_features = np.eye(10, dtype=np.int64)[digits].sum(axis=1)
    group_ids = np.array([row[1] for row in rows], dtype=np.int64).reshape(-1, picks)
    group_features = np.eye(groups, dtype=np.int64)[group_ids - 1].sum(axis=1)
    features = np.column_stack((digit_features.reshape(len(rounds), -1), group_features))
    repeated = (np.diff(np.sort(digits, axis=-1), axis=-1) == 0).any(axis=-1).sum(axis=1)
    report: dict[str, list[dict]] = {}
    if legacy:
        tier_rows = db.execute("SELECT n.round_no,length(n.digits),p.unit_krw FROM v_pension_draws n "
                               "JOIN prize p USING(series_code,round_no,tier) WHERE n.series_code=? AND n.tier='5' "
                               "ORDER BY n.round_no", (series,)).fetchall()
        for name, left, boundary, right in (('pension520_27_reported', 1, 27, 65),
                                            ('pension520_127', 75, 127, 239), ('pension520_388', 240, 388, 461)):
            tests.append(Comparison(name, rounds[left-1:right], features[left-1:right], (left,boundary)))
    else:
        bonus = np.array([list(row[0]) for row in db.execute(
            "SELECT digits FROM v_pension_draws WHERE series_code=? AND tier='bonus' ORDER BY round_no", (series,)
        )], dtype=np.int64)
        bonus_features = np.eye(10, dtype=np.int64)[bonus]

    for view, dated in PERIOD_VIEWS if legacy else (PERIOD_VIEWS[0],):
        slices = periods(db, series, dated)
        starts = tuple(x.start for x in slices)
        tests.append(Comparison(f'{series}_first_{view}', rounds, features, starts))
        if not legacy:
            tests.append(Comparison('pension720_bonus_dated', rounds, bonus_features.reshape(len(rounds), -1), starts))
        report[view] = []
        for period in slices:
            lo, hi = period.start - 1, period.end
            extra = ({'tier5_rule': sorted(set(row[1:] for row in tier_rows[lo:hi]))} if legacy else
                     {'bonus_position_digit_counts': bonus_features[lo:hi].sum(axis=0).tolist(),
                      'bonus_leading_zero': int(np.count_nonzero(bonus[lo:hi, 0] == 0))})
            report[view].append({**asdict(period), **extra, 'first_patterns': period.draws * picks,
                'first_group_counts': group_features[lo:hi].sum(axis=0).tolist(),
                'first_position_digit_counts': digit_features[lo:hi].sum(axis=0).tolist(),
                'first_repeated_digit_patterns': int(repeated[lo:hi].sum()),
                'first_same_group_draws': int(np.count_nonzero(group_ids[lo:hi, 0] == group_ids[lo:hi, 1])) if legacy else None,
            })
    return report


def main() -> None:
    path = ROOT / 'lottery.sqlite3'
    jobs: list[Comparison] = []
    with closing(sqlite3.connect(f'{path.as_uri()}?mode=ro', uri=True)) as db:
        report = {'lotto645': lotto(db, jobs), 'pension520': pension(db, 'pension520', jobs),
                  'pension720': pension(db, 'pension720', jobs)}
    rng = np.random.default_rng(SEED)
    results = [period_test(job, rng) for job in jobs]
    adjusted = holm(results)
    print(json.dumps({'sqlite_sha256': hashlib.sha256(path.read_bytes()).hexdigest(), 'numpy': np.__version__,
        'primary_period_view': 'dated', 'merged_period_view': 'confirmed',
        'seed': SEED, 'permutations_per_test': TRIALS,
        'lotto_uniform_reference': {'mean_sum': 6 * 23, 'mean_previous_overlap': 36 / 45,
                                    'any_consecutive_rate': 1 - comb(40, 6) / comb(45, 6),
                                    'one_main_number_rate': 6 / 45, 'one_bonus_number_rate': 1 / 45},
        'comparisons': [{**asdict(item), 'holm_p': adjusted[item.key]} for item in results], 'series': report,
    }, ensure_ascii=False, indent=2))


if __name__ == '__main__':
    main()
