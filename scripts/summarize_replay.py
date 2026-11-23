# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
# python3 scripts/summarize_replay.py active.log pension520.log > research/replay-period-rates.json
from collections import Counter
from dataclasses import asdict, dataclass
import hashlib
import json
from math import sqrt
from pathlib import Path
import sys
from typing import Final

ROOT: Final = Path(__file__).resolve().parents[1]
SERIES: Final = {'lotto645': 1251, 'pension520': 461, 'pension720': 342}
MARKER: Final = 'PERIOD_REPLAY '


class ReplayInputError(ValueError):
    def __init__(self, context: str) -> None:
        super().__init__(f'Invalid replay stream: {context}')


@dataclass(frozen=True, slots=True)
class Metrics:
    draws: int
    warmup: int
    scored: int
    tickets: int
    winning_draws: int
    winning_tickets: int
    draw_hit_pct: float | None
    ticket_hit_pct: float | None
    draw_wilson95_pct: tuple[float, float] | None
    tickets_by_best_prize: dict[str, int]


def summarize(rows: list[tuple[str, ...]]) -> Metrics:
    graded = [row for row in rows if row]
    histogram = Counter(rank for row in graded for rank in row)
    n = len(graded)
    tickets = sum(histogram.values())
    hits = tickets - histogram['miss']
    winning_draws = sum(any(rank != 'miss' for rank in row) for row in graded)
    interval = None
    if n:
        p, z = winning_draws / n, 1.959964
        center = p + z * z / (2 * n)
        width = z * sqrt(p * (1 - p) / n + z * z / (4 * n * n))
        scale = 100 / (1 + z * z / n)
        interval = (round(max(0, (center - width) * scale), 2), round(min(100, (center + width) * scale), 2))
    return Metrics(len(rows), len(rows) - n, n, tickets, winning_draws, hits,
                   round(100 * winning_draws / n, 2) if n else None,
                   round(100 * hits / tickets, 2) if tickets else None, interval, dict(sorted(histogram.items())))


def main(paths: list[Path]) -> None:
    boundaries = json.loads((ROOT / 'research/period-statistics.json').read_text())
    with (ROOT / 'lottery.sqlite3').open('rb') as snapshot:
        digest = hashlib.file_digest(snapshot, 'sha256').hexdigest()
    if digest != boundaries['sqlite_sha256']:
        raise ReplayInputError('source snapshot differs from dated period analysis')
    rows: dict[str, dict[int, tuple[str, ...]]] = {k: {} for k in SERIES}
    for path in paths:
        with path.open(encoding='utf-8') as stream:
            for line in stream:
                if MARKER not in line:
                    continue
                item = json.loads(line.split(MARKER, 1)[1])
                code, number, ranks = item['series'], item['round'], item['ranks']
                allowed = {str(i) for i in range(1, 6 if code == 'lotto645' else 8)} | {'miss'}
                if code == 'pension720':
                    allowed.add('bonus')
                if code not in SERIES or number in rows[code] or not (1 <= number <= SERIES[code]):
                    raise ReplayInputError(f'duplicate or unexpected {code}:{number}')
                if not isinstance(ranks, list) or len(ranks) != (0 if number <= 26 else 5) or not set(ranks) <= allowed:
                    raise ReplayInputError(f'bad rank list in {code}:{number}: {ranks}')
                rows[code][number] = tuple(ranks)
    result = {}
    for code, last in SERIES.items():
        if set(rows[code]) != set(range(1, last + 1)):
            raise ReplayInputError(f'missing rounds in {code}')
        periods = []
        previous_end = 0
        for item in boundaries['series'][code]['dated']:
            start, end = item['start'], item['end']
            if start != previous_end + 1 or end < start:
                raise ReplayInputError(f'non-contiguous {code} boundaries')
            previous_end = end
            sample = [rows[code][i] for i in range(start, end + 1)]
            periods.append({**{k: item[k] for k in ('start', 'end', 'start_date', 'end_date', 'events')},
                            **asdict(summarize(sample))})
        if previous_end != last:
            raise ReplayInputError(f'incomplete {code} periods')
        result[code] = {'model': 'legacy-520-research-only' if code == 'pension520' else 'recent-bias-v1',
                        'overall': asdict(summarize(list(rows[code].values()))), 'periods': periods}
    print(json.dumps({'source_sha256': digest, 'boundary_view': 'dated', 'series': result},
                     ensure_ascii=False, indent=2))


if __name__ == '__main__':
    main([Path(p) for p in sys.argv[1:]])
