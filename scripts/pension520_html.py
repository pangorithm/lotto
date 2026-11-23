import re
from collections import Counter
from dataclasses import dataclass
from datetime import date
from html.parser import HTMLParser
from typing import Final, Literal

ROUND_LINK: Final = re.compile(r'/lotto520/rounds/(\d+)/')
DRAW_DATE: Final = re.compile(r'(\d+)회\s*\((\d+)년\s*(\d+)월\s*(\d+)일 추첨\)')
MONTHLY: Final = re.compile(r'월\s*([\d,]+)만원\s*x\s*(\d+)년')


@dataclass(frozen=True, slots=True)
class Prize:
    unit_krw: int
    period: Literal['once', 'monthly']
    months: int | None = None


@dataclass(frozen=True, slots=True)
class Pattern:
    tier: str
    slot: int
    group_no: int | None
    digits: str
    prize: Prize


@dataclass(frozen=True, slots=True)
class LegacyDraw:
    round_no: int
    date: date
    patterns: tuple[Pattern, ...]


class InvalidLegacyDraw(ValueError):
    def __init__(self, round_no: int, field: str) -> None:
        super().__init__(f'Pension 520 round {round_no}: invalid {field}')


class LegacyResults(HTMLParser):
    def __init__(self) -> None:
        super().__init__()
        self.draws: list[LegacyDraw] = []
        self.number: int | None = None
        self.draw_date: date | None = None
        self.in_body = False
        self.in_row = False
        self.capture: str | None = None
        self.text: list[str] = []
        self.cells: list[str] = []
        self.tier: str | None = None
        self.slots: Counter[str] = Counter()
        self.patterns: list[Pattern] = []
        self.prizes: dict[str, Prize] = {}

    def handle_starttag(self, tag: str, attrs: list[tuple[str, str | None]]) -> None:
        if tag == 'a':
            href = dict(attrs).get('href')
            link = ROUND_LINK.fullmatch(href or '')
            if link is not None and int(link[1]) <= 461:
                self.number = int(link[1])
                self.draw_date = None
                self.tier = None
                self.slots.clear()
                self.patterns.clear()
                self.prizes.clear()
        if self.number is None:
            return
        if tag == 'tbody':
            self.in_body = True
        elif tag == 'tr' and self.in_body:
            self.in_row = True
            self.cells.clear()
        elif (tag == 'div' and self.draw_date is None) or (tag in ('th', 'td') and self.in_row):
            self.capture = tag
            self.text.clear()

    def handle_data(self, data: str) -> None:
        if self.capture is not None:
            self.text.append(data)

    def handle_endtag(self, tag: str) -> None:
        number = self.number
        if number is None:
            return
        if tag == self.capture:
            text = ''.join(self.text).strip()
            if tag == 'div':
                when = DRAW_DATE.fullmatch(text)
                if when is None or int(when[1]) != number:
                    raise InvalidLegacyDraw(number, 'heading/date')
                self.draw_date = date(int(when[2]), int(when[3]), int(when[4]))
            elif tag == 'th':
                self.tier = text.removesuffix('등')
            else:
                self.cells.append(text)
            self.capture = None
        if tag == 'tr' and self.in_row:
            self.in_row = False
            self.finish_row(number)
        elif tag == 'tbody':
            self.in_body = False
        elif tag == 'a':
            if self.draw_date is None or not self.patterns:
                raise InvalidLegacyDraw(number, 'missing draw result')
            self.draws.append(LegacyDraw(number, self.draw_date, tuple(self.patterns)))
            self.number = None

    def finish_row(self, number: int) -> None:
        tier = self.tier
        if tier not in ('1', '2', '3', '4', '5', '6', '7') or len(self.cells) != 8:
            raise InvalidLegacyDraw(number, 'tier/cells')
        pay, group, *digits = self.cells
        if group != '각조' and not re.fullmatch(r'[1-7]조', group):
            raise InvalidLegacyDraw(number, 'group')
        pattern = ''.join(digits).lstrip('-')
        digit_lengths = {'1': (6,), '2': (6,), '3': (6,), '4': (5,), '5': (3, 4), '6': (2,), '7': (1,)}
        if not re.fullmatch(r'[0-9]+', pattern) or len(pattern) not in digit_lengths[tier]:
            raise InvalidLegacyDraw(number, 'digits')
        monthly = MONTHLY.fullmatch(pay)
        prize = (Prize(int(monthly[1].replace(',', '')) * 10_000, 'monthly', int(monthly[2]) * 12)
                 if monthly else Prize(int(pay.replace(',', '')), 'once'))
        if self.prizes.setdefault(tier, prize) != prize:
            raise InvalidLegacyDraw(number, 'conflicting prize')
        self.slots[tier] += 1
        self.patterns.append(Pattern(tier, self.slots[tier], None if group == '각조' else int(group[0]),
                                     pattern, prize))
