# 스냅샷 검증 기록

- 기준: 2026-11-23 KST. `lottery.sqlite3`는 1,499,136바이트이며 SHA-256은 `5dda58c213a6a97a24aa9ff2242c33ed08045d7e902a86837a6f628dd0198910`이다.
- SQLite 3.45.1에서 빌드. `PRAGMA integrity_check`는 `ok`, `PRAGMA foreign_key_check`는 0행, `PRAGMA user_version`은 `1`이었다. [`assert_snapshot.sql`](../scripts/assert_snapshot.sql)의 회차, 등위, 당첨번호, 이력 연결 검사를 통과했다.
- 2,054회: 로또 1,251회, 520 461회, 720+ 342회. 연금복권 번호/등위 패턴 8,729행, 상금 12,218행. 520 제2회 5등 `0772`로 선행 0 보존을 확인했다.
- 로또: 공식 원본과 [smok95 아카이브](https://smok95.github.io/lotto/results/all.json)의 전 1,251회 날짜·번호·보너스·판매액 및 양쪽 모두에 채워진 등위 필드를 비교했다. 비교한 값은 일치했다. 비공식 아카이브 일부는 당첨 게임 및 금액이 0인 등위 항목을 생략한다. 예를 들어 로또 1회 1등은 공식 값 `0`을 보존했다. 보조 아카이브는 독립적인 추첨 기록이 아니라 별도로 수집한 공개 결과다.
- 520: 공식 구형 페이지의 변경 후 경로를 확인하지 못해 [펴늬 116페이지](https://pyony.com/lotto520/rounds/?page=116)를 사용했다. [JTBC 제76회 보도](https://news.jtbc.co.kr/article/NB10215223)와 1등 표본을 확인했다. 5등 패턴 1–65회는 4자리, 66–461회는 3자리로 저장되며 [당시 보도](https://kidd.co.kr/news/160495)의 전환 경계와 일치한다. 전수 교차 검증은 아니다.
- 720+: [동행복권 등위별 결과](https://www.dhlottery.co.kr/pt720/selectPstPt720Info.do?srchStrPsltEpsd=1&srchEndPsltEpsd=342)와 [별도 목록](https://www.dhlottery.co.kr/pt720/selectPstPt720WnList.do)의 날짜, 1등 조·숫자, 보너스가 전 342회 일치했다. 결과 건수와 지급 총액은 적재하지 않았다.
- 이력은 32건(확인 23건, 보도 7건, 미확인 2건), 사건과 출처 연결 68건이다. 회차 결과·이력에 등록한 URL 164건은 37개 호스트에 걸쳐 있다. URL 수에는 520의 페이지별 원본도 포함된다. 세부 근거와 상태는 [`events.sql`](../data/events.sql) 또는 `v_event_timeline`·`event_evidence`·`source`에서 조회한다.
