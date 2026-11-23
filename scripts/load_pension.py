import hashlib
import json
import sqlite3
from datetime import datetime
from pathlib import Path

from pension520_html import LegacyResults


def load_520(db: sqlite3.Connection, root: Path) -> None:
    for page in range(1, 117):
        path = root / f'data/raw/pension520/page-{page}.html'
        raw = path.read_bytes()
        source = f'pyony_520_{page:03}'
        db.execute('INSERT INTO source VALUES(?,?,?,?,?,?,?,?)', (
            source, f'https://pyony.com/lotto520/rounds/?page={page}', '펴늬',
            '연금복권520 회차별 당첨번호', 'third_party', '2026-11-23',
            str(path.relative_to(root)), hashlib.sha256(raw).hexdigest(),
        ))
        parser = LegacyResults()
        parser.feed(raw.decode('utf-8'))
        parser.close()
        for draw in parser.draws:
            db.execute('INSERT INTO draw VALUES(?,?,?,?,?,NULL)', (
                'pension520', draw.round_no, draw.date.isoformat(), source,
                f'a[href="/lotto520/rounds/{draw.round_no}/"]',
            ))
            for pattern in draw.patterns:
                db.execute('INSERT INTO pension_number VALUES(?,?,?,?,?,?)', (
                    'pension520', draw.round_no, pattern.tier, pattern.slot,
                    pattern.group_no, pattern.digits,
                ))
                if pattern.slot == 1:
                    db.execute('INSERT INTO prize VALUES(?,?,?,NULL,?,?,?,NULL)', (
                        'pension520', draw.round_no, pattern.tier, pattern.prize.unit_krw,
                        pattern.prize.period, pattern.prize.months,
                    ))


def load_720(db: sqlite3.Connection, root: Path) -> None:
    path = root / 'data/raw/pension720/prizes-1-342.json'
    raw = path.read_bytes()
    db.execute('INSERT INTO source VALUES(?,?,?,?,?,?,?,?)', (
        'dh_720_prizes', 'https://www.dhlottery.co.kr/pt720/selectPstPt720Info.do?'
        'srchStrPsltEpsd=1&srchEndPsltEpsd=342', '동행복권', '연금복권720+ 등위별 추첨결과',
        'official', '2026-11-23', str(path.relative_to(root)), hashlib.sha256(raw).hexdigest(),
    ))
    details = json.loads(raw)['data']['result']
    dates: dict[int, str] = {}
    for item in details:
        number = item['psltEpsd']
        draw_date = datetime.strptime(item['psltRflYmd'], '%Y%m%d').date().isoformat()
        if number in dates and dates[number] != draw_date:
            raise InconsistentDates(number)
        if number not in dates:
            dates[number] = draw_date
            db.execute('INSERT INTO draw VALUES(?,?,?,?,?,NULL)', (
                'pension720', number, draw_date, 'dh_720_prizes', f'psltEpsd={number}',
            ))
        rank = item['wnSqNo']
        tier = 'bonus' if rank == 21 else str(rank)
        group_text = item['wnBndNo']
        group_no = None if group_text is None else int(group_text)
        db.execute('INSERT INTO pension_number VALUES(?,?,?,1,?,?)', (
            'pension720', number, tier, group_no, item['wnRnkVl'],
        ))
        months = {1: 240, 2: 120, 21: 120}.get(rank)
        amount = item['wnAmt']
        if months is not None and amount % months != 0:
            raise InconsistentPayment(number, rank)
        db.execute('INSERT INTO prize VALUES(?,?,?,NULL,?,?,?,NULL)', (
            'pension720', number, tier, amount if months is None else amount // months,
            'once' if months is None else 'monthly', months,
        ))
    verify_720_draw_list(db, root)


def verify_720_draw_list(db: sqlite3.Connection, root: Path) -> None:
    path = root / 'data/raw/pension720/draws.json'
    raw = path.read_bytes()
    expected = sorted(
        (row['psltEpsd'], row['psltRflYmd'], int(row['wnBndNo']), row['wnRnkVl'], row['bnsRnkVl'])
        for row in json.loads(raw)['data']['result']
    )
    actual = db.execute(
        "SELECT first.round_no, replace(first.draw_date, '-', ''), first.group_no, first.digits, bonus.digits "
        "FROM v_pension_draws first JOIN pension_number bonus USING (series_code, round_no) "
        "WHERE first.series_code = 'pension720' AND first.tier = '1' AND bonus.tier = 'bonus' "
        "ORDER BY first.round_no"
    ).fetchall()
    if actual != expected:
        raise InconsistentDrawList(path)
    db.execute('INSERT INTO source VALUES(?,?,?,?,?,?,?,?)', (
        'dh_720_draws', 'https://www.dhlottery.co.kr/pt720/selectPstPt720WnList.do',
        '동행복권', '연금복권720+ 1등·보너스 교차 확인 원천', 'official', '2026-11-23',
        str(path.relative_to(root)), hashlib.sha256(raw).hexdigest(),
    ))


class InconsistentDates(ValueError):
    def __init__(self, round_no: int) -> None:
        super().__init__(f'Pension 720+ round {round_no}: conflicting source dates')


class InconsistentPayment(ValueError):
    def __init__(self, round_no: int, rank: int) -> None:
        super().__init__(f'Pension 720+ round {round_no}, rank {rank}: nonintegral monthly amount')


class InconsistentDrawList(ValueError):
    def __init__(self, path: Path) -> None:
        super().__init__(f'Pension 720+ stored results do not match {path}')


def load_pension(db: sqlite3.Connection, root: Path) -> None:
    load_520(db, root)
    load_720(db, root)
