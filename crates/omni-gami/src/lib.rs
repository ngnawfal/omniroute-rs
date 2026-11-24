// XP / Level engine — port of gamification/xp.ts pure functions + streaks.ts + badges.ts.
// Persistence lives in omni-store; this crate provides pure state transitions.

use serde::{Deserialize, Serialize};

pub const XP_REWARDS: &[(XpAction, i64)] = &[
    (XpAction::Request, 1),
    (XpAction::ProviderSwitch, 5),
    (XpAction::ModelSwitch, 3),
    (XpAction::ComboCreate, 10),
    (XpAction::ComboUse, 2),
    (XpAction::TokenShare, 1),
    (XpAction::InviteRedeem, 50),
    (XpAction::DailyLogin, 5),
    (XpAction::StreakBonus, 2),
    (XpAction::BadgeUnlock, 10),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum XpAction {
    Request,
    ProviderSwitch,
    ModelSwitch,
    ComboCreate,
    ComboUse,
    TokenShare,
    InviteRedeem,
    DailyLogin,
    StreakBonus,
    BadgeUnlock,
}

impl XpAction {
    pub fn from_str(s: &str) -> Option<Self> {
        Some(match s {
            "request" => Self::Request,
            "provider_switch" => Self::ProviderSwitch,
            "model_switch" => Self::ModelSwitch,
            "combo_create" => Self::ComboCreate,
            "combo_use" => Self::ComboUse,
            "token_share" => Self::TokenShare,
            "invite_redeem" => Self::InviteRedeem,
            "daily_login" => Self::DailyLogin,
            "streak_bonus" => Self::StreakBonus,
            "badge_unlock" => Self::BadgeUnlock,
            _ => return None,
        })
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Request => "request",
            Self::ProviderSwitch => "provider_switch",
            Self::ModelSwitch => "model_switch",
            Self::ComboCreate => "combo_create",
            Self::ComboUse => "combo_use",
            Self::TokenShare => "token_share",
            Self::InviteRedeem => "invite_redeem",
            Self::DailyLogin => "daily_login",
            Self::StreakBonus => "streak_bonus",
            Self::BadgeUnlock => "badge_unlock",
        }
    }
    pub fn base_xp(&self) -> i64 {
        XP_REWARDS
            .iter()
            .find(|(a, _)| *a == *self)
            .map(|(_, xp)| *xp)
            .unwrap_or(0)
    }
}

pub fn xp_for_level(level: u32) -> i64 {
    if level <= 1 {
        return 0;
    }
    (100.0 * (level as f64).powf(1.5)).floor() as i64
}

pub fn cumulative_xp_for_level(level: u32) -> i64 {
    if level <= 1 {
        return 0;
    }
    (2..=level).map(xp_for_level).sum()
}

pub fn calculate_level(total_xp: i64) -> u32 {
    if total_xp <= 0 {
        return 1;
    }
    let bounded = total_xp.min(i64::MAX / 4);
    let est: u32 = (((bounded as f64) * 2.5 / 100.0).powf(0.4)).floor() as u32;
    let mut level = est.max(1);
    while level > 1 && cumulative_xp_for_level(level) > bounded {
        level -= 1;
    }
    while cumulative_xp_for_level(level.checked_add(1).unwrap_or(level)) <= bounded {
        level += 1;
    }
    level
}

pub fn xp_to_next_level(total_xp: i64, next_level: u32) -> i64 {
    (cumulative_xp_for_level(next_level) - total_xp).max(0)
}

pub fn level_tier(level: u32) -> &'static str {
    if level >= 76 {
        "diamond"
    } else if level >= 51 {
        "platinum"
    } else if level >= 26 {
        "gold"
    } else if level >= 11 {
        "silver"
    } else {
        "bronze"
    }
}

// ── Streak logic ────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct StreakData {
    pub current_streak: u32,
    pub longest_streak: u32,
    pub last_active_date: String,
    pub streak_start_date: String,
}

pub fn advance_streak(today: &str, yesterday: &str, prev: &StreakData) -> (u32, bool) {
    if prev.last_active_date == today {
        return (prev.current_streak, false);
    }
    if prev.last_active_date == yesterday {
        return (prev.current_streak + 1, true);
    }
    (1, false)
}

pub fn merged_streak(prev: &StreakData, today: &str, yesterday: &str) -> StreakData {
    let (current, _) = advance_streak(today, yesterday, prev);
    let mut out = prev.clone();
    if prev.last_active_date != today {
        if prev.last_active_date != yesterday || out.streak_start_date.is_empty() {
            out.streak_start_date = today.to_string();
        }
        out.current_streak = current;
        out.last_active_date = today.to_string();
        if current > out.longest_streak {
            out.longest_streak = current;
        }
    }
    out
}

pub fn empty_streak() -> StreakData {
    StreakData::default()
}

// ── Badge criteria evaluation ───────────────────────────────────────

#[derive(Debug, Clone)]
pub struct BadgeCriteria {
    pub kind: String,
    pub action: String,
    pub threshold: i64,
}

pub fn parse_criteria(json: &str) -> Option<BadgeCriteria> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    Some(BadgeCriteria {
        kind: v.get("type")?.as_str()?.to_string(),
        action: v.get("action").and_then(|x| x.as_str()).unwrap_or("").to_string(),
        threshold: v.get("threshold")?.as_i64()?,
    })
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BadgeState {
    #[serde(default)]
    pub action_counts: Vec<(String, i64)>,
    #[serde(default)]
    pub unique_providers: Vec<String>,
    #[serde(default)]
    pub unique_models: Vec<String>,
}

pub fn bump_action(s: &mut BadgeState, action: &str, delta: i64) {
    match s.action_counts.iter_mut().find(|(a, _)| a == action) {
        Some((_, v)) => *v += delta,
        None => s.action_counts.push((action.to_string(), delta)),
    }
}

pub fn record_unique(s: &mut BadgeState, provider: Option<&str>, model: Option<&str>) {
    if let Some(p) = provider.filter(|p| !p.is_empty()) {
        if !s.unique_providers.iter().any(|x| x == p) {
            s.unique_providers.push(p.to_string());
        }
    }
    if let Some(m) = model.filter(|m| !m.is_empty()) {
        if !s.unique_models.iter().any(|x| x == m) {
            s.unique_models.push(m.to_string());
        }
    }
}

pub fn criteria_met(criteria: &BadgeCriteria, state: &BadgeState) -> bool {
    criteria_met_with_streak(criteria, state, 0)
}

pub fn criteria_met_with_streak(criteria: &BadgeCriteria, state: &BadgeState, streak: u32) -> bool {
    match criteria.kind.as_str() {
        "action_count" => state
            .action_counts
            .iter()
            .find(|(a, _)| a == &criteria.action)
            .map(|(_, v)| *v >= criteria.threshold)
            .unwrap_or(false),
        "unique_count" => {
            let list: &Vec<String> = match criteria.action.as_str() {
                "provider" => &state.unique_providers,
                "model" => &state.unique_models,
                _ => return false,
            };
            list.len() as i64 >= criteria.threshold
        }
        "streak" => streak as i64 >= criteria.threshold,
        _ => false,
    }
}

pub static BUILTIN_BADGES: &[(&str, &str, &str, &str, &str)] = &[
    ("first-token", "First Token", "sparkles", "common", r#"{"type":"action_count","action":"request","threshold":1}"#),
    ("token-consumer", "Token Consumer", "zap", "uncommon", r#"{"type":"action_count","action":"request","threshold":1000}"#),
    ("token-machine", "Token Machine", "cpu", "rare", r#"{"type":"action_count","action":"request","threshold":10000}"#),
    ("token-whale", "Token Whale", "whale", "legendary", r#"{"type":"action_count","action":"request","threshold":100000}"#),
    ("generous", "Generous", "gift", "common", r#"{"type":"action_count","action":"token_share","threshold":1000}"#),
    ("philanthropist", "Philanthropist", "heart", "uncommon", r#"{"type":"action_count","action":"token_share","threshold":10000}"#),
    ("token-santa", "Token Santa", "santa", "rare", r#"{"type":"action_count","action":"token_share","threshold":100000}"#),
    ("community-hero", "Community Hero", "trophy", "legendary", r#"{"type":"action_count","action":"token_share","threshold":1000000}"#),
    ("explorer", "Explorer", "compass", "uncommon", r#"{"type":"unique_count","action":"provider","threshold":5}"#),
    ("polyglot", "Polyglot", "languages", "rare", r#"{"type":"unique_count","action":"model","threshold":10}"#),
    ("architect", "Architect", "blocks", "uncommon", r#"{"type":"action_count","action":"combo_create","threshold":3}"#),
    ("streak-7", "Week Warrior", "flame", "uncommon", r#"{"type":"streak","action":"daily","threshold":7}"#),
];

/// Evaluate all builtin badges and return any newly unlocked badge IDs.
pub fn check_new_badges(
    state: &BadgeState,
    streak: u32,
    already_unlocked: &[String],
) -> Vec<String> {
    let mut newly = Vec::new();
    for (id, _, _, _, criteria_json) in BUILTIN_BADGES {
        if already_unlocked.iter().any(|u| u == id) {
            continue;
        }
        if let Some(c) = parse_criteria(criteria_json) {
            if criteria_met_with_streak(&c, state, streak) {
                newly.push((*id).to_string());
            }
        }
    }
    newly
}

// ── antiCheat signal detection ──────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub struct CheatSignal {
    pub kind: &'static str,
    pub severity: u8,
}

pub fn detect_cheat(
    requests_last_minute: u32,
    distinct_providers_last_hour: u32,
    same_text_repeat_count: u32,
) -> Vec<CheatSignal> {
    let mut out = Vec::new();
    if requests_last_minute > 120 {
        out.push(CheatSignal { kind: "burst_spam", severity: 2 });
    }
    if distinct_providers_last_hour > 40 {
        out.push(CheatSignal { kind: "provider_rotation_spam", severity: 1 });
    }
    if same_text_repeat_count > 50 {
        out.push(CheatSignal { kind: "text_flood", severity: 2 });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xp_curve_and_level_math() {
        assert_eq!(xp_for_level(1), 0);
        assert_eq!(xp_for_level(10), 3162);
        assert_eq!(calculate_level(0), 1);
        assert_eq!(calculate_level(cumulative_xp_for_level(10)), 10);
        assert_eq!(calculate_level(50_000), 16);
        assert_eq!(level_tier(1), "bronze");
        assert_eq!(level_tier(76), "diamond");
    }

    #[test]
    fn streak_and_badges_unlock() {
        let mut s = BadgeState::default();
        bump_action(&mut s, "request", 1);
        let unlocked = check_new_badges(&s, 7, &[]);
        assert!(unlocked.contains(&"first-token".to_string()));
        assert!(unlocked.contains(&"streak-7".to_string()));
    }

    #[test]
    fn builtin_badges_parse() {
        for (id, _, _, _, criteria) in BUILTIN_BADGES {
            assert!(parse_criteria(criteria).is_some(), "{id}");
        }
    }
}
