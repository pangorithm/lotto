# 대한민국 로또·연금복권 당첨 기록

2026년 11월 23일(한국시간) 기준 [SQLite 스냅샷](lottery.sqlite3)이다. 공개 추첨 결과와 당첨 조건·운영의 변경 이력을 구분해 저장했다.

| 상품 코드 | 상품 | 회차 | 첫 추첨 | 마지막 수록 추첨 | 당첨 결과의 출처 |
|---|---|---:|---|---|---|
| `lotto645` | 로또 6/45 | 1–1,251 | 2002-12-07 | 2026-11-21 | [동행복권 회차별 전체 결과](https://www.dhlottery.co.kr/lt645/selectPstLt645Info.do?srchLtEpsd=all) |
| `pension520` | 연금복권520 | 1–461 | 2011-07-06 | 2020-04-29 | [펴늬 보존 아카이브](https://pyony.com/lotto520/rounds/), 공식 구형 조회 경로 미확인 |
| `pension720` | 연금복권720+ | 1–342 | 2020-05-07 | 2026-11-19 | [동행복권 등위별 결과](https://www.dhlottery.co.kr/pt720/selectPstPt720Info.do?srchStrPsltEpsd=1&srchEndPsltEpsd=342), [1등·보너스 별도 목록](https://www.dhlottery.co.kr/pt720/selectPstPt720WnList.do)으로 대조 |

연금복권520 아카이브의 462회는 번호 없이 결과 대기만 표시된 항목이라 제외했다. 720+는 520의 462회가 아니라 별도 상품의 1회부터 시작한다. 연금복권의 `winning_games`, `total_krw`, 판매액은 일관되게 검증된 자료가 없어 `NULL`이다. 당첨 번호, 520의 각 등위별 당첨 패턴과 금액, 720+의 각 등위별 및 보너스 번호와 금액은 수록했다. 당첨자 신원은 수록하지 않았다.

## 추첨·운영 변경 조사

| 기준일 | 구분 | 주요 변화와 근거 |
|---|---|---|
| 2003-02-08 / 2003년 10월 예정 | 공개 | [SBS 제10회 생방송 예고](https://news.sbs.co.kr/news/endPage.do?news_id=N0311369478)와 [8월 정례 전환 발표 보도](https://mobile.busan.com/view/busan/view.php?code=20030822000013)를 분리했다. 연속 생방송 시작 회차는 미확인이다. |
| 2007-12-02 / 2007-12-08 | 운영·장비 | [국민은행 → 나눔로또 인계](https://www.bokgwon.go.kr/post/postView.do?boardSeq=7&category=&pageNum=1&seq=676). 로또 262회부터 [할로겐 회전식 → 비너스 공기혼합식](https://www.mk.co.kr/news/all/4342908). [263회 소개 자료](https://www.bokgwon.go.kr/post/postView.do?boardSeq=7&category=&pageNum=1&seq=677)는 새 공을 약 4g 폴리우레탄으로 명시한다. |
| 2012-10-03 | 당첨 조건 | 연금복권520 [66회부터 5등: 끝 4자리·20만원 → 끝 3자리·2만원](https://kidd.co.kr/news/160495). |
| 2013-12-02 | 운영 | [나눔로또 제3기 통합 수탁](https://mpb.go.kr/web/main/bbs/b_0005/590). 로또 수탁자는 유지되고 연금복권은 한국연합복권에서 이관됐다. |
| 2016-12-17 | 장비 | 로또 733회 [비너스 RL2 적용 보도](https://www.seoulfn.com/news/articleView.html?idxno=268634). 원 공지 및 개별 볼 세트 교체 여부를 확보하지 못해 `reported`로 구분했다. |
| 2018-12-02 / 12-05 / 12-08 | 운영·공개 | [동행복권 제4기 인계 및 방송사 변경](https://news.tf.co.kr/read/economy/1739557.htm). 연금복권520 제388회부터 MBC드라마넷, 로또 제836회부터 [MBC 생방송](https://enews.imbc.com/M/Detail/249475). |
| 2020-05-07 | 당첨 조건·장비·공개 | [연금복권720+ 첫 추첨](https://playvod.imbc.com/Vod/VodPlay?broadcastId=1004971100001100000). 조 범위, 상금, 보너스와 추첨 구조가 바뀌었고 [화살·회전판에서 Topaze](https://www.akanis.tech/donghang-lottery-welfare-lottery-720/)로 전환했다. |
| 2023-06-10 / 2024-11-23 | 참관 | 로또 [1,071회](https://m.blog.naver.com/donghanglottery1004/223127776934)와 [1,147회](https://news.tf.co.kr/read/economy/2154191.htm) 확대 공개. 일회성 참관 행사이며 추첨 방식 변경으로 표시하지 않았다. |
| 2024-01-01 | 운영 | [동행복권 제5기 계약 개시](https://www.bokgwon.go.kr/intro/07.jsp). 수탁사 자체는 바뀌지 않았다. |
| 2026-02-09 | 판매 | [모바일 웹 로또 시범판매 시작](https://www.korea.kr/news/policyNewsView.do?newsId=148959331). 번호 추첨이 아닌 판매 채널의 변화다. |

그 밖의 방송사 변경, 일회성 예비 장비 사용, 가격·상금, 결과 조회 경로 이력은 [이력 데이터](data/events.sql)와 DB에 있다. **전체 회차의 실제 추첨기 일련번호, 볼 세트 투입·정비 이력은 확인하지 못했다.** 520의 역사적 번호는 아카이브에서 전 회차를 수록했지만, 공식 원천으로 전수 대조한 것은 아니다. 예를 들어 520 제76회 1등의 `6조 409876`과 `5조 645504`는 [당시 JTBC 보도](https://news.jtbc.co.kr/article/NB10215223)와 일치한다.

## Rust 데몬에서 읽기

아래는 원본 스냅샷 조회 계약이다. 원본은 **읽기 전용**으로 열고 `PRAGMA user_version`이 `1`인지 확인한다. 날짜는 한국 현지 추첨일 `YYYY-MM-DD`다. 별도 운영 DB에 쓰는 Rust 데몬의 실행법은 [아래](#rust-데몬-실행)를 참고한다.

```sql
SELECT round_no, draw_date, n1, n2, n3, n4, n5, n6, bonus
FROM v_lotto_draws
WHERE round_no < ?1
ORDER BY round_no;

SELECT round_no, draw_date, tier, slot, group_no, digits
FROM v_pension_draws
WHERE series_code = ?1 AND round_no < ?2
ORDER BY round_no, tier, slot;

SELECT t.series_code, t.date_from, t.date_precision, t.first_round, t.round_role,
       t.category, t.effect, t.status, t.title, s.url, e.anchor
FROM v_event_timeline t
JOIN event_evidence e ON e.event_id = t.id
JOIN source s ON s.id = e.source_id
ORDER BY t.date_from IS NULL, t.date_from, t.id, t.series_code;
```

- 로또의 `n1`–`n6`은 **오름차순으로 발표된 번호**이며 공이 나온 순서가 아니다. 연금복권의 `digits`는 Rust에서 `String`으로 읽는다. 선행 0을 보존하고, 끝자리 일치만 필요한 경우 해당 길이로 저장된다. `group_no IS NULL`은 그룹 조건이 없는 패턴이며 특정 그룹을 뜻하지 않는다. `slot`은 같은 등위 내 패턴 구분용이지 추첨 차례가 아니다.
- `prize.unit_krw`는 일시금이면 게임당 금액, `payment_period='monthly'`이면 월 금액이다. `payment_months`로 지급 기간을 구분한다. `draw`와 `source`를 연결하면 각 회차의 URL, 원본 경로, SHA-256, 응답 내 위치를 확인할 수 있다.
- `event.date_from`은 사건일이나 알려진 시점의 범위 시작이고, `announced_on`과 `event_scope.first_round`는 각각 발표일과 관련 회차다. `round_role='observed_in'`은 관측 사례이지 적용 시작이 아니다. `reported`와 `unresolved`는 확정 경계로 쓰지 않는다.
- 회차를 키로 쓸 때는 상품을 함께 지정한다. 예측을 실험한다면 대상 회차 이후 결과와 당시 미공개였던 사후 이력을 학습에 넣지 말고, 시간순 평가에서 균등 무작위 생성기를 기준선으로 둔다. 이력이 있다고 해서 다음 회차의 당첨 확률을 높일 수 있다는 뜻은 아니다.

## 재현과 검사

추첨·운영 변경 시점으로 나눈 통계와 번호 패턴은 [기간별 분석](research/period-analysis.md), 전체 빈도·검정 결과는 [집계 JSON](research/period-statistics.json)을 참고한다.

Python 3.11 이상 및 SQLite 3.37 이상. 네트워크나 별도 Python 패키지 없이 보관한 응답에서 새 파일을 만든다. 기존 출력 파일은 덮어쓰지 않는다.

```sh
python3 scripts/build_db.py /tmp/lottery-rebuilt.sqlite3
```

[스키마](schema.sql)와 [스냅샷 검사](scripts/assert_snapshot.sql)는 번호 범위, 회차·등위 완전성, 520의 5등 규칙 경계, 720+의 끝자리 일관성, 외래키 및 SQLite 무결성을 검사한다. 실제 검사 결과와 파일 해시는 [검증 기록](research/verification.md)에 있다. 이 스크립트는 이 날짜의 스냅샷을 재생성하는 도구이며 새 회차 자동 수집기는 아니다.

## Rust 데몬 실행

Linux에서 Podman과 Compose 제공자(`podman-compose` 등)가 필요하다. 저장소 루트에서 실행한다.

```sh
podman compose -p lotto-daemon up -d --build
podman compose -p lotto-daemon exec -T daemon /usr/local/bin/lottery-daemon status
podman compose -p lotto-daemon logs --tail=30 daemon
```

- 컨테이너는 첫 시작에 이미지에 포함된 스냅샷을 `state` 볼륨으로 복사하고 운영 스키마 v3를 적용한다. 이후에는 기존 볼륨을 재사용하고, 이전 운영 스키마도 시작 시 마이그레이션한다. 리포지터리의 `lottery.sqlite3`는 덮어쓰지 않는다. 운영 DB 경로는 `/var/lib/lottery/lottery.sqlite3`다. `podman compose -p lotto-daemon down`은 볼륨을 유지하며, `down -v`는 운영 기록까지 삭제한다.
- 기본 설정은 시작 즉시, 그리고 각 확인이 끝난 뒤 900초마다 동행복권을 확인한다(`LOTTERY_INTERVAL_SECONDS`로 변경 가능). 로또 6/45·연금복권720+의 공개된 최신 회차까지만 누락분을 연속해서 저장한다. 판매가 끝난 520은 보존만 한다. 두 상품은 독립적으로 처리되며 응답 오류는 다음 폴링에서 재시도한다. 720+의 당첨 게임 수는 이 수집 경로의 범위에 포함되지 않는다.
- `prediction`에는 공정 추첨을 가정한 균등 무작위 후보를 상품·회차별로 한 번만 저장한다(`uniform-v1`). 로또는 1–45에서 중복 없이 6개, 720+는 1–5조와 `000000`–`999999`다. 다음 추첨일은 마지막 발표일에 7일을 더해 계산한다. 지연된 결과 때문에 이미 추첨됐을 수 있는 회차의 번호를 사후 생성하지 않도록 대상 추첨일 자정(KST)부터는 새 후보를 생성하지 않는다. 번호 구매 기능은 없고, 당첨 확률을 높인다는 근거도 없다.
- `status`는 최신 저장 회차와 현재 후보를 JSON으로 출력한다. `once`는 한 차례 실행하고 오류가 있으면 비정상 종료한다. 실행 중인 데몬과 별개로 수동 확인이 필요하다면 먼저 데몬을 멈추거나 다른 Compose 프로젝트명을 사용한다.

```sh
podman compose -p lotto-daemon-test -f compose.test.yaml build test
podman compose -p lotto-daemon-test -f compose.test.yaml run --rm test
```

테스트는 로컬 HTTP 응답과 임시 SQLite를 사용한다. 운영 볼륨과 외부 복권 사이트는 테스트 중에 사용하지 않는다.

### 슬랙 결과 알림

호스트의 `/etc/env/slack/.env`에서 `WEBHOOK_URL_DOT_CHANNEL`만 읽어 Podman Secret으로 등록한다. 웹훅 값은 Git, 이미지, Compose 환경 변수 또는 데몬 로그에 넣지 않는다. 아래 명령은 기존 `lotto-daemon` 프로젝트와 볼륨을 그대로 사용한다.

```sh
bash scripts/setup-slack-secret.sh
podman compose -p lotto-daemon -f compose.yaml -f compose.slack.yaml up -d --build --force-recreate
```

새 회차를 수집하면 해당 회차의 당첨번호·보너스, 발표 전에 저장한 후보와 실제 번호의 일치 결과, 그리고 다음 회차에 저장한 후보를 상품별 메시지로 전송한다. 해당 후보를 저장한 적이 없거나 새 후보의 생성 시간이 이미 지났으면 `없음`으로 표시하며 뒤늦게 만든 번호를 예측 결과로 표시하지 않는다. 운영 DB에 새로 추가되는 회차만 알림 대기열에 넣으며, 기존 스냅샷·운영 DB의 과거 회차를 소급 전송하지 않는다.

운영 스키마 v3의 대기열에 수집 결과를 함께 저장한다. Slack이 수락한 회차는 정상 재실행·재시작 시 다시 보내지 않으며, 실패한 전송은 재시도한다. Slack이 수락한 직후 성공 기록을 남기기 전에 프로세스가 중단되면 같은 메시지가 재전송될 수 있다. 기존 Podman Secret이 있으면 설정 스크립트는 값을 덮어쓰지 않는다. 웹훅을 교체하려면 데몬을 중지하고 기존 Secret을 제거한 다음 위 두 명령을 다시 실행한다.
