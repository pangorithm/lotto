# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
# python3 scripts/compare_uniform.py > research/uniform-comparison.json
from __future__ import annotations

import hashlib
import json
import sqlite3
from collections import defaultdict
from contextlib import closing
from dataclasses import dataclass
from fractions import Fraction
from math import comb
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


@dataclass(frozen=True, slots=True)
class WinningSpace:
    winners: int
    population: int

    def ticket_rate(self) -> Fraction:
        return Fraction(self.winners, self.population)

    def round_rate(self) -> Fraction:
        return 1 - Fraction(comb(self.population - self.winners, 5), comb(self.population, 5))


def pension_space(groups: int, patterns: list[tuple[int | None, str]]) -> WinningSpace:
    per_group = 900_000 if groups == 7 else 1_000_000
    total = 0
    for group in range(1, groups + 1):
        suffixes = sorted({s for g, s in patterns if g is None or g == group}, key=lambda s: (len(s), s))
        accepted: list[str] = []
        for suffix in suffixes:
            if any(suffix.endswith(shorter) for shorter in accepted):
                continue
            accepted.append(suffix)
            if len(suffix) < 6:
                total += per_group // 10 ** len(suffix)
            else:
                total += int(groups == 5 or suffix[0] != '0')
    return WinningSpace(total, groups * per_group)


def comparison(spaces: list[WinningSpace], metrics: dict[str, int]) -> dict[str, float | None]:
    n = metrics['scored']
    if n != len(spaces) or metrics['tickets'] != 5 * n:
        raise RuntimeError('Replay and baseline denominators differ')
    result = {}
    for unit, method, hits, denom in (
        ('round', WinningSpace.round_rate, metrics['winning_draws'], n),
        ('ticket', WinningSpace.ticket_rate, metrics['winning_tickets'], 5 * n),
    ):
        baseline = sum(map(method, spaces), start=Fraction(0)) * 100 / n if n else None
        model = Fraction(100 * hits, denom) if denom else None
        delta = model - baseline if (model is not None and baseline is not None) else None
        for label, val in (('model_pct', model), ('uniform_pct', baseline), ('delta_pp', delta)):
            result[f'{unit}_{label}'] = round(float(val), 6) if val is not None else None
    return result


def main() -> None:
    rates = json.loads((ROOT / 'research/replay-period-rates.json').read_text())
    path = ROOT / 'lottery.sqlite3'
    with path.open('rb') as source:
        if hashlib.file_digest(source, 'sha256').hexdigest() != rates['source_sha256']:
            raise RuntimeError('Wrong source snapshot')
    series = {}
    lotto = WinningSpace(sum(comb(6, k) * comb(39, 6-k) for k in range(3, 7)), comb(45, 6))
    with closing(sqlite3.connect(f'{path.as_uri()}?immutable=1&mode=ro', uri=True)) as db:
        for code, report in rates['series'].items():
            spaces = {}
            if code == 'lotto645':
                spaces = {n: lotto for (n,) in db.execute('SELECT round_no FROM lotto_result WHERE round_no > 26')}
            else:
                patterns: defaultdict[int, list[tuple[int | None, str]]] = defaultdict(list)
                for n, group, digits in db.execute(
                    'SELECT round_no,group_no,digits FROM pension_number WHERE series_code=? AND round_no > 26', (code,)):
                    patterns[n].append((group, digits))
                spaces = {n: pension_space(7 if code == 'pension520' else 5, ps) for n, ps in patterns.items()}
            period_rows = []
            for period in report['periods']:
                start, end = period['start'], period['end']
                subset = [spaces[n] for n in range(max(27, start), end + 1)]
                period_rows.append({'start': start, 'end': end, **comparison(subset, period)})
            series[code] = {'overall': comparison(list(spaces.values()), report['overall']), 'periods': period_rows}
    print(json.dumps({'source_sha256': rates['source_sha256'], 'baseline': 'five distinct uniform tickets without replacement',
                      'series': series}, indent=2))


if __name__ == '__main__':
    main()
