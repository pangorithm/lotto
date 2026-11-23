# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
# Run: python3 scripts/build_db.py [new-output-path]

import hashlib
import json
import sqlite3
import sys
from contextlib import closing
from datetime import datetime
from pathlib import Path
from tempfile import TemporaryDirectory
from typing import Final

from load_pension import load_pension

ROOT: Final = Path(__file__).resolve().parents[1]
ORIGIN: Final = 'https://www.dhlottery.co.kr'
ACCESSED: Final = '2026-11-23'


def load_lotto(db: sqlite3.Connection) -> None:
    path = ROOT / 'data/raw/lotto/all.json'
    raw = path.read_bytes()
    db.execute(
        'INSERT INTO source VALUES(?,?,?,?,?,?,?,?)',
        ('dh_lotto', f'{ORIGIN}/lt645/selectPstLt645Info.do?srchLtEpsd=all',
         '동행복권', '회차별 추첨결과', 'official', ACCESSED,
         str(path.relative_to(ROOT)), hashlib.sha256(raw).hexdigest()),
    )
    for item in json.loads(raw)['data']['list']:
        number = item['ltEpsd']
        draw_date = datetime.strptime(item['ltRflYmd'], '%Y%m%d').date().isoformat()
        db.execute('INSERT INTO draw VALUES(?,?,?,?,?,?)',
                   ('lotto645', number, draw_date, 'dh_lotto', f'ltEpsd={number}',
                    item['wholEpsdSumNtslAmt']))
        balls = tuple(item[f'tm{i}WnNo'] for i in range(1, 7))
        db.execute('INSERT INTO lotto_result VALUES(?,?,?,?,?,?,?,?,?)',
                   ('lotto645', number, *balls, item['bnsWnNo']))
        for tier in range(1, 6):
            prefix = f'rnk{tier}'
            db.execute('INSERT INTO prize VALUES(?,?,?,?,?,?,?,?)',
                       ('lotto645', number, str(tier), item[f'{prefix}WnNope'],
                        item[f'{prefix}WnAmt'], 'once', None, item[f'{prefix}SumWnAmt']))


def main(output: Path) -> None:
    with TemporaryDirectory(prefix='.lottery-build-', dir=output.parent) as workspace:
        stage = Path(workspace) / 'snapshot.sqlite3'
        with closing(sqlite3.connect(stage)) as db:
            db.executescript((ROOT / 'schema.sql').read_text(encoding='utf-8'))
            with db:
                db.executemany('INSERT INTO series VALUES(?,?,?,?)', (
                    ('lotto645', '로또 6/45', '2002-12-07', None),
                    ('pension520', '연금복권520', '2011-07-06', '2020-04-29'),
                    ('pension720', '연금복권720+', '2020-05-07', None),
                ))
                load_lotto(db)
                load_pension(db, ROOT)
            db.executescript((ROOT / 'data/events.sql').read_text(encoding='utf-8'))
            db.executescript((ROOT / 'scripts/assert_snapshot.sql').read_text(encoding='utf-8'))
        output.hardlink_to(stage)


if __name__ == '__main__':
    main(Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else ROOT / 'lottery.sqlite3')
