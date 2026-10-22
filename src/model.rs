use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};

pub fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Window {
    pub used_percent: f64,
    pub window_duration_mins: Option<i64>,
    pub resets_at: Option<i64>,
}
impl Window {
    pub fn remaining(&self) -> f64 {
        (100.0 - self.used_percent).clamp(0.0, 100.0)
    }
    pub fn label(&self) -> String {
        match self.window_duration_mins {
            Some(m) if m >= 10080 && m % 10080 == 0 => format!("{}周", m / 10080),
            Some(m) if m >= 1440 && m % 1440 == 0 => format!("{}天", m / 1440),
            Some(m) if m >= 60 && m % 60 == 0 => format!("{}小时", m / 60),
            Some(m) if m > 0 => format!("{m}分钟"),
            _ => "当前周期".into(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Bucket {
    pub limit_id: Option<String>,
    pub limit_name: Option<String>,
    pub primary: Option<Window>,
    pub secondary: Option<Window>,
    pub rate_limit_reached_type: Option<String>,
    pub credits: Option<CreditBalance>,
    pub individual_limit: Option<SpendLimit>,
    pub spend_control_reached: Option<bool>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreditBalance {
    pub balance: Option<String>,
    pub has_credits: bool,
    pub unlimited: bool,
}
impl CreditBalance {
    pub fn rounded_balance(&self) -> Option<String> {
        self.balance
            .as_deref()?
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|value| value.is_finite() && *value >= 0.0)
            .map(|value| format!("{:.0}", value.ceil().max(0.0)))
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpendLimit {
    pub remaining_percent: f64,
    pub resets_at: i64,
    pub limit: String,
    pub used: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResetCard {
    pub id: String,
    pub reset_type: String,
    pub status: String,
    pub granted_at: i64,
    // The upstream schema explicitly defines null as non-expiring.
    // Missing expiresAt is rejected below when parsing a full snapshot.
    pub expires_at: Option<i64>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResetCards {
    pub available_count: i64,
    pub credits: Option<Vec<ResetCard>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Quota {
    #[serde(default)]
    pub rate_limits: Bucket,
    pub rate_limits_by_limit_id: Option<BTreeMap<String, Bucket>>,
    pub rate_limit_reset_credits: Option<ResetCards>,
    pub account_id: Option<String>,
    pub ordinary_usage_allowed: Option<bool>,
    #[serde(default)]
    pub fetched_at: i64,
    #[serde(default)]
    pub expiry_details_known: bool,
}
impl Quota {
    pub fn parse(value: serde_json::Value, fetched_at: i64) -> Result<Self, String> {
        if !value
            .get("rateLimits")
            .is_some_and(serde_json::Value::is_object)
        {
            return Err("额度响应不完整，请更新官方组件或稍后重试".into());
        }
        let expiry_known = value
            .pointer("/rateLimitResetCredits/credits")
            .and_then(|v| v.as_array())
            .is_some_and(|rows| rows.iter().all(|r| r.get("expiresAt").is_some()));
        let mut q: Self =
            serde_json::from_value(value).map_err(|_| "额度数据格式不兼容".to_string())?;
        q.fetched_at = fetched_at;
        q.expiry_details_known = expiry_known;
        if q.windows()
            .iter()
            .any(|(_, w)| !w.used_percent.is_finite() || w.used_percent < 0.0)
        {
            return Err("额度百分比无效".into());
        }
        Ok(q)
    }
    pub fn buckets(&self) -> Vec<(&str, &Bucket)> {
        if let Some(map) = &self.rate_limits_by_limit_id {
            if !map.is_empty() {
                return map.iter().map(|(k, v)| (k.as_str(), v)).collect();
            }
        }
        vec![(
            self.rate_limits.limit_id.as_deref().unwrap_or("codex"),
            &self.rate_limits,
        )]
    }
    pub fn windows(&self) -> Vec<(String, &Window)> {
        self.buckets()
            .into_iter()
            .flat_map(|(key, b)| {
                let name = b.limit_name.as_deref().unwrap_or(key);
                [b.primary.as_ref(), b.secondary.as_ref()]
                    .into_iter()
                    .flatten()
                    .map(move |w| (format!("{name} · {}", w.label()), w))
            })
            .collect()
    }
    pub fn tightest(&self) -> Option<(String, &Window)> {
        self.windows()
            .into_iter()
            .min_by(|a, b| a.1.remaining().total_cmp(&b.1.remaining()))
    }
    pub fn depleted(&self) -> bool {
        self.buckets().iter().any(|(k, b)| {
            *k == "codex"
                && (b.rate_limit_reached_type.as_deref() == Some("rate_limit_reached")
                    || [b.primary.as_ref(), b.secondary.as_ref()]
                        .into_iter()
                        .flatten()
                        .any(|w| w.used_percent >= 100.0))
        })
    }
    pub fn stale(&self, time: i64, interval: u64) -> bool {
        self.fetched_at > time + 5 || time - self.fetched_at > (interval * 2 + 5) as i64
    }
    pub fn card_count(&self) -> Option<i64> {
        self.rate_limit_reset_credits
            .as_ref()
            .map(|r| r.available_count)
    }
    pub fn compact_suffix(&self) -> String {
        let mut text = String::new();
        if let Some(count) = self.card_count().filter(|count| *count > 0) {
            text.push_str(&format!(" *{count}重置卡"));
        }
        let credits = self.rate_limits.credits.as_ref().or_else(|| {
            self.rate_limits_by_limit_id
                .as_ref()?
                .get("codex")?
                .credits
                .as_ref()
        });
        if let Some(credits) = credits {
            if credits.unlimited {
                text.push_str(" *不限积分");
            } else if credits.has_credits {
                if let Some(value) = credits.rounded_balance().filter(|value| value != "0") {
                    text.push_str(&format!(" *{value}积分"));
                }
            }
        }
        text
    }
    pub fn choose_card(&self, time: i64) -> Result<&ResetCard, String> {
        if !self.depleted() {
            return Err("当前没有已耗尽的 Codex 额度".into());
        }
        let summary = self
            .rate_limit_reset_credits
            .as_ref()
            .ok_or("重置卡数量未知")?;
        if summary.available_count <= 0 {
            return Err("没有可用的重置卡".into());
        }
        let cards = summary
            .credits
            .as_ref()
            .ok_or("官方暂未提供卡片明细，无法确定最早到期卡片")?;
        let available: Vec<_> = cards.iter().filter(|c| c.status == "available").collect();
        let unique: HashSet<_> = available.iter().map(|c| &c.id).collect();
        if !self.expiry_details_known
            || unique.len() != available.len()
            || available.len() as i64 != summary.available_count
        {
            return Err("卡片明细不完整，暂不使用，避免消耗错卡".into());
        }
        if available
            .iter()
            .any(|c| c.id.is_empty() || c.reset_type != "codexRateLimits")
        {
            return Err("存在无法识别的卡片类型，暂不使用".into());
        }
        available
            .into_iter()
            .filter(|c| c.expires_at.is_none_or(|t| t > time))
            .min_by_key(|c| {
                (
                    c.expires_at.unwrap_or(i64::MAX),
                    c.granted_at,
                    c.id.as_str(),
                )
            })
            .ok_or_else(|| "卡片已到期，请刷新后重试".into())
    }
}

/// Continuous RGB interpolation through red → orange → yellow → green.
pub fn quota_color(percent: f64) -> [u8; 3] {
    let stops = [
        (0.0, [255.0, 48.0, 64.0]),
        (20.0, [255.0, 128.0, 24.0]),
        (45.0, [255.0, 220.0, 0.0]),
        (70.0, [80.0, 240.0, 32.0]),
        (100.0, [0.0, 235.0, 128.0]),
    ];
    let p = if percent.is_finite() {
        percent.clamp(0.0, 100.0)
    } else {
        0.0
    };
    for pair in stops.windows(2) {
        if p <= pair[1].0 {
            let t = (p - pair[0].0) / (pair[1].0 - pair[0].0);
            return std::array::from_fn(|i| {
                (pair[0].1[i] * (1.0 - t) + pair[1].1[i] * t).round() as u8
            });
        }
    }
    [0, 235, 128]
}
pub fn remaining_text(p: f64) -> String {
    if p > 0.0 && p < 1.0 {
        "<1%".into()
    } else {
        format!("{:.0}%", p.floor())
    }
}
pub fn time_label(ts: Option<i64>) -> String {
    ts.and_then(|t| chrono::DateTime::from_timestamp(t, 0))
        .map(|d| {
            d.with_timezone(&chrono::Local)
                .format("%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_else(|| "无到期时间".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn q(cards: serde_json::Value, count: i64) -> Quota {
        Quota::parse(json!({"rateLimits":{"limitId":"codex","primary":{"usedPercent":100}},"rateLimitResetCredits":{"availableCount":count,"credits":cards}}),100).unwrap()
    }
    fn c(id: &str, expiry: Option<i64>) -> serde_json::Value {
        json!({"id":id,"expiresAt":expiry,"grantedAt":10,"resetType":"codexRateLimits","status":"available"})
    }
    #[test]
    fn picks_soonest_not_list_order() {
        let x = q(
            json!([c("late", Some(900)), c("soon", Some(200)), c("never", None)]),
            3,
        );
        assert_eq!(x.choose_card(100).unwrap().id, "soon");
    }
    #[test]
    fn skips_expired_and_places_non_expiring_last() {
        let x = q(
            json!([c("old", Some(100)), c("never", None), c("valid", Some(101))]),
            3,
        );
        assert_eq!(x.choose_card(100).unwrap().id, "valid");
        assert_eq!(x.choose_card(200).unwrap().id, "never");
    }
    #[test]
    fn incomplete_or_duplicate_details_block_consumption() {
        assert!(q(json!([c("a", Some(200))]), 2).choose_card(100).is_err());
        assert!(
            q(json!([c("a", Some(200)), c("a", Some(200))]), 2)
                .choose_card(100)
                .is_err()
        );
        assert!(q(serde_json::Value::Null, 1).choose_card(100).is_err());
    }
    #[test]
    fn unknown_expiry_not_assumed_non_expiring() {
        let mut card = c("a", None);
        card.as_object_mut().unwrap().remove("expiresAt");
        assert!(q(json!([card]), 1).choose_card(100).is_err());
    }
    #[test]
    fn rounded_zero_is_not_exhaustion() {
        let mut x = q(json!([c("a", Some(300))]), 1);
        x.rate_limits.primary.as_mut().unwrap().used_percent = 99.9;
        assert!(!x.depleted());
        assert_eq!(remaining_text(0.1), "<1%");
    }
    #[test]
    fn missing_data_not_zero() {
        assert!(Quota::parse(json!({}), 1).is_err());
        let x = q(serde_json::Value::Null, 0);
        assert_eq!(x.card_count(), Some(0));
    }
    #[test]
    fn partial_notifications_must_not_be_parsed_as_full_reads() {
        assert!(Quota::parse(json!({"primary":{"usedPercent":20}}), 1).is_err());
    }
    #[test]
    fn color_continuous_at_stops() {
        for p in [20.0, 50.0] {
            let a = quota_color(p - 0.01);
            let b = quota_color(p + 0.01);
            for i in 0..3 {
                assert!((a[i] as i16 - b[i] as i16).abs() <= 1)
            }
        }
    }

    #[test]
    fn compact_suffix_hides_zero_unknown_and_invalid_balances() {
        let mut quota = q(json!([]), 0);
        assert_eq!(quota.compact_suffix(), "");
        for balance in ["0", "0.000", "-2", "NaN", "inf", "unknown", ""] {
            quota.rate_limits.credits = Some(CreditBalance {
                balance: Some(balance.into()),
                has_credits: true,
                unlimited: false,
            });
            assert_eq!(quota.compact_suffix(), "", "{balance}");
        }
        quota.rate_limits.credits.as_mut().unwrap().balance = None;
        assert_eq!(quota.compact_suffix(), "");
    }

    #[test]
    fn compact_suffix_keeps_real_values_and_does_not_sum_duplicate_buckets() {
        let mut quota = q(json!([c("a", None), c("b", None), c("c", None)]), 3);
        quota.rate_limits.credits = Some(CreditBalance {
            balance: Some("200.000".into()),
            has_credits: true,
            unlimited: false,
        });
        quota.rate_limits_by_limit_id = Some(BTreeMap::from([(
            "codex".into(),
            quota.rate_limits.clone(),
        )]));
        assert_eq!(quota.compact_suffix(), " *3重置卡 *200积分");
        quota.rate_limits.credits.as_mut().unwrap().balance = Some("0.0004".into());
        assert_eq!(quota.compact_suffix(), " *3重置卡 *1积分");
        quota.rate_limits.credits.as_mut().unwrap().has_credits = false;
        assert_eq!(quota.compact_suffix(), " *3重置卡");
    }

    #[test]
    fn compact_suffix_can_use_the_codex_bucket_and_unlimited_balance() {
        let mut quota = q(json!([]), 0);
        let bucket = Bucket {
            credits: Some(CreditBalance {
                balance: None,
                has_credits: false,
                unlimited: true,
            }),
            ..Default::default()
        };
        quota.rate_limits_by_limit_id = Some(BTreeMap::from([("codex".into(), bucket)]));
        assert_eq!(quota.compact_suffix(), " *不限积分");
    }

    #[test]
    fn credit_balances_round_up_to_whole_points() {
        for (raw, expected) in [
            ("0", Some("0")),
            ("-0.0", Some("0")),
            ("0.0004", Some("1")),
            ("1.2", Some("2")),
            ("1.9", Some("2")),
            ("200.000", Some("200")),
            ("200.001", Some("201")),
            (" 1.01e2 ", Some("101")),
            ("4e-4", Some("1")),
            ("-2", None),
            ("NaN", None),
            ("inf", None),
            ("unknown", None),
        ] {
            let credits = CreditBalance {
                balance: Some(raw.into()),
                has_credits: true,
                unlimited: false,
            };
            assert_eq!(credits.rounded_balance().as_deref(), expected, "{raw}");
            assert_eq!(credits.balance.as_deref(), Some(raw));
        }
    }
}
