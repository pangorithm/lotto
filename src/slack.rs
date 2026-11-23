use std::{fs, path::Path, time::Duration};

use anyhow::{Context, Result, anyhow, ensure};
use reqwest::{Client, StatusCode, Url, redirect::Policy};
use serde_json::json;

use crate::domain::{Game, Notice, NoticeResult, Ticket};

pub(crate) struct Slack {
    client: Client,
    url: Url,
}

impl Slack {
    pub(crate) fn new(path: &Path) -> Result<Self> {
        let secret = fs::read_to_string(path).context("read Slack webhook secret")?;
        let url = Url::parse(secret.trim()).map_err(|_| anyhow!("invalid Slack webhook URL"))?;
        let slack = url.scheme() == "https"
            && matches!(
                url.host_str(),
                Some("hooks.slack.com" | "hooks.slack-gov.com")
            )
            && url.port().is_none();
        let loopback =
            url.scheme() == "http" && matches!(url.host_str(), Some("127.0.0.1" | "[::1]"));
        ensure!(
            (slack || loopback)
                && url.username().is_empty()
                && url.password().is_none()
                && url.path().starts_with("/services/")
                && url.query().is_none()
                && url.fragment().is_none(),
            "invalid Slack webhook URL"
        );
        let client = Client::builder()
            .use_rustls_tls()
            .redirect(Policy::none())
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|_| anyhow!("create Slack client failed"))?;
        Ok(Self { client, url })
    }

    pub(crate) async fn send(&self, notice: &Notice) -> Result<()> {
        let mut response = self
            .client
            .post(self.url.clone())
            .json(&json!({ "text": message(notice) }))
            .send()
            .await
            .map_err(|_| anyhow!("Slack request failed"))?;
        ensure!(
            response.status() == StatusCode::OK,
            "Slack request rejected with HTTP {}",
            response.status()
        );
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| anyhow!("read Slack response failed"))?
        {
            ensure!(
                body.len().saturating_add(chunk.len()) <= 32,
                "invalid Slack response"
            );
            body.extend_from_slice(&chunk);
        }
        ensure!(body.trim_ascii() == b"ok", "invalid Slack response");
        Ok(())
    }
}

fn lotto_score(ticket: &[u8; 6], numbers: &[u8; 6], bonus: u8) -> (Vec<u8>, bool, Option<u8>) {
    let matched: Vec<u8> = ticket
        .iter()
        .copied()
        .filter(|n| numbers.contains(n))
        .collect();
    let bonus_match = ticket.contains(&bonus);
    let tier = match (matched.len(), bonus_match) {
        (6, _) => Some(1),
        (5, true) => Some(2),
        (5, false) => Some(3),
        (4, _) => Some(4),
        (3, _) => Some(5),
        _ => None,
    };
    (matched, bonus_match, tier)
}

fn suffix_match(left: &str, right: &str) -> usize {
    left.bytes()
        .rev()
        .zip(right.bytes().rev())
        .take_while(|(a, b)| a == b)
        .count()
}

fn ticket_text(ticket: &Ticket) -> String {
    match ticket {
        Ticket::Lotto { numbers } => format!("{numbers:?}"),
        Ticket::Pension { group_no, digits } => format!("{group_no}조 {digits}"),
    }
}

fn score_text(ticket: &Ticket, result: &NoticeResult) -> String {
    match (ticket, result) {
        (Ticket::Lotto { numbers: selected }, NoticeResult::Lotto { numbers, bonus }) => {
            let (matched, bonus_match, tier) = lotto_score(selected, numbers, *bonus);
            let tier = tier.map_or_else(|| "해당 없음".to_owned(), |value| format!("{value}등"));
            format!("일치 번호: {matched:?} / 보너스 일치: {bonus_match} / 등위 조건: {tier}\n")
        }
        (
            Ticket::Pension { group_no, digits },
            NoticeResult::Pension {
                group_no: drawn_group,
                digits: drawn,
                bonus,
            },
        ) => {
            let suffix = suffix_match(digits, drawn);
            let bonus_suffix = suffix_match(digits, bonus);
            format!(
                "번호 전체 일치: {} / 조 일치: {} / 끝자리 연속 일치: {suffix}자리\n보너스 번호 전체 일치 (조 무관): {} / 보너스 끝자리 연속 일치: {bonus_suffix}자리\n",
                digits == drawn,
                group_no == drawn_group,
                digits == bonus
            )
        }
        (Ticket::Lotto { .. }, NoticeResult::Pension { .. })
        | (Ticket::Pension { .. }, NoticeResult::Lotto { .. }) => {
            "이전 후보와 추첨 상품이 달라 비교할 수 없습니다.\n".to_owned()
        }
    }
}

fn message(notice: &Notice) -> String {
    let game = match notice.game {
        Game::Lotto => "로또 6/45",
        Game::Pension => "연금복권 720+",
    };
    let mut text = format!("{game} {}회 ({})\n", notice.head.round, notice.head.date);
    match &notice.result {
        NoticeResult::Lotto { numbers, bonus } => {
            text.push_str(&format!("추첨 번호: {numbers:?} / 보너스: {bonus}\n"))
        }
        NoticeResult::Pension {
            group_no,
            digits,
            bonus,
        } => text.push_str(&format!(
            "추첨 번호: {group_no}조 {digits} / 보너스 번호: {bonus}\n"
        )),
    }
    if notice.model_prior.is_empty() {
        text.push_str("이전 저장 예측 없음\n");
    } else {
        text.push_str("저장된 예측과 실제 당첨번호 비교:\n");
        for (i, ticket) in notice.model_prior.iter().enumerate() {
            text.push_str(&format!("{}. {}\n", i + 1, ticket_text(ticket)));
            text.push_str(&score_text(ticket, &notice.result));
        }
    }
    if notice.model_next.is_empty() {
        text.push_str("다음 예측 없음\n");
    } else {
        text.push_str(&format!(
            "다음 {}회 예측 (recent-bias-v1 실험):\n",
            notice.head.round.saturating_add(1)
        ));
        for (i, ticket) in notice.model_next.iter().enumerate() {
            text.push_str(&format!("{}. {}\n", i + 1, ticket_text(ticket)));
        }
    }
    text.push_str("저장된 후보의 번호 비교이며 구매 또는 실제 당첨을 뜻하지 않습니다.");
    text
}

#[cfg(test)]
mod tests {
    use super::{lotto_score, suffix_match};

    #[test]
    fn lotto_tiers_distinguish_bonus_from_regular_matches() {
        let drawn = [1, 2, 3, 4, 5, 6];
        for (ticket, expected) in [
            ([1, 2, 3, 4, 5, 6], (vec![1, 2, 3, 4, 5, 6], false, Some(1))),
            ([1, 2, 3, 4, 5, 7], (vec![1, 2, 3, 4, 5], true, Some(2))),
            ([1, 2, 3, 4, 5, 8], (vec![1, 2, 3, 4, 5], false, Some(3))),
            ([1, 2, 3, 4, 7, 8], (vec![1, 2, 3, 4], true, Some(4))),
            ([1, 2, 3, 7, 8, 9], (vec![1, 2, 3], true, Some(5))),
            ([1, 2, 7, 8, 9, 10], (vec![1, 2], true, None)),
        ] {
            assert_eq!(lotto_score(&ticket, &drawn, 7), expected);
        }
    }

    #[test]
    fn pension_suffix_preserves_leading_zeroes_and_stops_at_first_difference() {
        assert_eq!(suffix_match("001234", "991234"), 4);
        assert_eq!(suffix_match("001234", "001234"), 6);
        assert_eq!(suffix_match("001234", "001235"), 0);
    }
}
