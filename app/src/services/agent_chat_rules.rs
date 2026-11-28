//! Deterministic conversation rules for the agent chat.
//!
//! The translator model only extracts language: brief
//! updates and a draft message. Whether a turn asks,
//! searches or replies is decided — or overridden — here and
//! in [`crate::services::agent_chat_service::enforce_rules`],
//! wherever the situation is recognisable without a model.
//! Every pattern list for that lives in this module and
//! nowhere else: small, explicit, and model-agnostic, so
//! swapping the model stays a config change.

use crate::models::agent::Preset;
use serde_json::Value;

use crate::services::agent_chat_service::{
    BriefUpdates, BudgetUpdate, CategoryUpdate, ChatAction, ChatDecision, ChatError,
};

/// Heads of an affirmation: the visitor agreeing to the
/// agent's last question. A head matches the whole message
/// or a message that continues after a comma — "はい、
/// そういうものほしいです" affirms; "はいそうです" does not.
const AFFIRMATION_HEADS: &[&str] = &[
    "はい",
    "お願いします",
    "おねがいします",
    "それでいいです",
    "それでいい",
    "それでお願いします",
    "そうして",
    "そうしてください",
    "うん",
    "いいです",
    "いいよ",
    "オーケー",
    "OK",
    "ok",
];

/// Phrases that close the conversation: thanks or a clear
/// sign-off. A closing turn never searches.
const CLOSING_PHRASES: &[&str] = &["ありがとう", "助かりました", "助かった", "以上です"];

/// Phrases that abandon the conversation: the visitor asks
/// to start over or to stop, which is the reset button in
/// words.
const RESET_PHRASES: &[&str] = &["最初から", "やめる", "リセット"];

/// Search promises: a message saying a search is under way.
/// Said without a search decision, the turn has stalled —
/// the promise rescue in `enforce_rules` turns the words
/// into the search they promised.
const SEARCH_PROMISES: &[&str] = &[
    "探しています",
    "お探しします",
    "お探しいたします",
    "検索します",
    "お調べします",
    "調べます",
    "探します",
];

/// "Well-rated / popular" phrasing: the best-rated preset in
/// words. Checked before the cheapest list — when a message
/// names both ("安くて評価の高い"), the rating phrase is the
/// more specific signal and wins.
const BEST_RATED_PHRASES: &[&str] = &[
    "評価の高い",
    "評価が高い",
    "高評価",
    "人気の",
    "人気が",
    "人気商品",
];

/// "Cheap" phrasing: the cheapest preset in words.
const CHEAPEST_PHRASES: &[&str] = &[
    "もっと安い",
    "もっと安く",
    "安くして",
    "安い順",
    "安いもの",
    "安くて",
    "安い",
];

/// Trims a message and drops trailing sentence punctuation,
/// so the affirmation heads compare against the words
/// themselves.
fn normalise(text: &str) -> &str {
    text.trim()
        .trim_end_matches(['。', '！', '!', '？', '?', ' '])
}

/// Returns whether the message affirms the agent's previous
/// question: an affirmation head as the whole message, or
/// heading a sentence that continues after a comma.
pub fn is_affirmation(text: &str) -> bool {
    let text = normalise(text);
    AFFIRMATION_HEADS.iter().any(|head| {
        text == *head
            || text
                .strip_prefix(head)
                .is_some_and(|rest| rest.starts_with(['、', '，', ',', ' ', '　']))
    })
}

/// Returns whether the message closes the conversation
/// (thanks or a sign-off).
pub fn is_closing(text: &str) -> bool {
    CLOSING_PHRASES.iter().any(|phrase| text.contains(phrase))
}

/// Returns whether the message asks to abandon the
/// conversation and start over.
pub fn is_reset_request(text: &str) -> bool {
    RESET_PHRASES.iter().any(|phrase| text.contains(phrase))
}

/// Returns whether an agent message promises a search.
pub fn promises_search(message: &str) -> bool {
    SEARCH_PROMISES
        .iter()
        .any(|phrase| message.contains(phrase))
}

/// The deterministic brief adjustments one message states
/// outright: a preset in words, a yen amount. `None` fields
/// mean the message did not mention them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FollowUpTransform {
    /// Preset the message asked for, in words.
    pub preset: Option<Preset>,
    /// Budget the message stated, in yen.
    pub budget_yen: Option<i64>,
}

/// Reads the deterministic adjustments out of a message:
/// "better rated" beats "cheaper" when both are named (see
/// `BEST_RATED_PHRASES`), and a yen amount — `N円` or
/// `N万円`, ASCII or full-width digits — reads as the budget,
/// which is what an amount means in a shopping conversation.
pub fn follow_up_transform(text: &str) -> Option<FollowUpTransform> {
    let preset = if BEST_RATED_PHRASES.iter().any(|p| text.contains(p)) {
        Some(Preset::BestRated)
    } else if CHEAPEST_PHRASES.iter().any(|p| text.contains(p)) {
        Some(Preset::Cheapest)
    } else {
        None
    };
    let budget_yen = yen_amount(text);
    if preset.is_none() && budget_yen.is_none() {
        return None;
    }
    Some(FollowUpTransform { preset, budget_yen })
}

/// Finds the first yen amount in a message: a digit run
/// (ASCII or full-width, commas ignored) followed by `円`,
/// with an optional `万` before the `円` multiplying by ten
/// thousand. Amounts are budgets in this conversation, so
/// only a positive amount counts.
fn yen_amount(text: &str) -> Option<i64> {
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let mut digits = String::new();
        let mut j = i;
        while j < chars.len() {
            let c = chars[j];
            if c.is_ascii_digit() {
                digits.push(c);
            } else if ('０'..='９').contains(&c) {
                let ascii = char::from_u32(c as u32 - '０' as u32 + '0' as u32)?;
                digits.push(ascii);
            } else if c == ',' || c == '，' {
                if digits.is_empty() {
                    break;
                }
            } else {
                break;
            }
            j += 1;
        }
        if digits.is_empty() {
            i += 1;
            continue;
        }
        let mut multiplier = 1_i64;
        let mut k = j;
        if chars.get(k) == Some(&'万') {
            multiplier = 10_000;
            k += 1;
        }
        if chars.get(k) == Some(&'円')
            && let Ok(amount) = digits.parse::<i64>()
            && amount > 0
        {
            // Checked: a visitor can type an amount whose 万
            // multiplication overflows i64
            // ("1000000000000000万円"). An overflowing amount
            // is no amount — keep scanning.
            if let Some(yen) = amount.checked_mul(multiplier) {
                return Some(yen);
            }
        }
        i = j.max(i + 1);
    }
    None
}

/// Reads one budget value: a positive number (or numeric
/// string, which small models emit) sets the cap, zero clears
/// it, anything else is ignored.
fn parse_budget(raw: &Value) -> Option<BudgetUpdate> {
    let value = match raw {
        Value::Number(number) => number.as_i64(),
        Value::String(text) => text.trim().parse::<i64>().ok(),
        _ => None,
    }?;
    match value {
        0 => Some(BudgetUpdate::Clear),
        yen if yen > 0 => Some(BudgetUpdate::Set(yen)),
        _ => None,
    }
}

/// Reads one needs list: an array replaces, anything else is
/// ignored. Items are trimmed, empties dropped, and the list
/// is capped so a runaway answer cannot stuff the brief.
fn parse_needs(raw: &Value) -> Option<Vec<String>> {
    let items = raw.as_array()?;
    Some(
        items
            .iter()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|need| !need.is_empty())
            .take(10)
            .map(|need| need.chars().take(50).collect())
            .collect(),
    )
}

/// Drops a category update naming a category the store does
/// not have: the prompt restricts the model to the real list,
/// and this is the enforcement behind the instruction — an
/// unknown name keeps the accumulated category instead of
/// searching an empty pool.
pub(crate) fn sanitise_updates(updates: &mut BriefUpdates, categories: &[String]) {
    if let Some(CategoryUpdate::Set(name)) = &updates.category
        && !categories.iter().any(|real| real == name)
    {
        updates.category = None;
    }
}

/// Parses and validates one decision body. Strict about the
/// envelope (action, and a message for ask/reply), lenient
/// inside the brief — a wrong-typed field is ignored rather
/// than failing the whole turn, because the chat text is the
/// part the visitor sees.
///
/// # Errors
///
/// Returns [`ChatError::BadOutput`] when the content is not
/// the contracted JSON decision.
pub fn parse_decision(content: &str) -> Result<ChatDecision, ChatError> {
    let value: Value = serde_json::from_str(strip_fences(content))
        .map_err(|_| ChatError::BadOutput("decision was not JSON".to_string()))?;
    let object = value
        .as_object()
        .ok_or_else(|| ChatError::BadOutput("decision was not an object".to_string()))?;
    let action = match object.get("action").and_then(Value::as_str) {
        Some("ask") => ChatAction::Ask,
        Some("search") => ChatAction::Search,
        Some("reply") => ChatAction::Reply,
        _ => {
            return Err(ChatError::BadOutput(
                "decision had no usable action".to_string(),
            ));
        }
    };
    let message = object
        .get("message")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
        .chars()
        .take(1000)
        .collect::<String>();
    let message = if message.is_empty() {
        match action {
            // A search still needs its lead-in line; the
            // default carries no facts, so it cannot mislead.
            ChatAction::Search => "承知しました。お探しします。".to_string(),
            _ => {
                return Err(ChatError::BadOutput("decision had no message".to_string()));
            }
        }
    } else {
        message
    };
    Ok(ChatDecision {
        action,
        message,
        updates: parse_updates(object.get("brief")),
    })
}

/// Strips a markdown code fence a model wrapped its JSON in
/// despite the instruction not to.
pub(crate) fn strip_fences(content: &str) -> &str {
    let trimmed = content.trim();
    let inner = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .unwrap_or(trimmed);
    inner.strip_suffix("```").unwrap_or(inner).trim()
}

/// Reads the brief updates out of the decision's `brief`
/// member. Absent or null means "nothing was mentioned", per
/// field; see [`BriefUpdates`] for the keep/set/clear split.
pub(crate) fn parse_updates(value: Option<&Value>) -> BriefUpdates {
    let mut updates = BriefUpdates::default();
    let Some(object) = value.and_then(Value::as_object) else {
        return updates;
    };
    if let Some(raw) = object.get("budget_yen") {
        updates.budget_yen = parse_budget(raw);
    }
    if let Some(raw) = object.get("category") {
        updates.category = match raw.as_str() {
            Some(name) if !name.trim().is_empty() => {
                Some(CategoryUpdate::Set(name.trim().to_string()))
            }
            Some(_) => Some(CategoryUpdate::Clear),
            None => None,
        };
    }
    if let Some(raw) = object.get("must_have") {
        updates.must_have = parse_needs(raw);
    }
    if let Some(raw) = object.get("nice_to_have") {
        updates.nice_to_have = parse_needs(raw);
    }
    if let Some(raw) = object.get("preset").and_then(Value::as_str) {
        updates.preset = Preset::from_form_value(raw);
    }
    updates
}

#[cfg(test)]
mod tests {
    use super::{
        follow_up_transform, is_affirmation, is_closing, is_reset_request, promises_search,
    };
    use crate::models::agent::Preset;

    #[test]
    fn affirmations_match_heads_and_comma_continuations() {
        assert!(is_affirmation("はい"));
        assert!(is_affirmation("はい。"));
        // The exact sentence that stalled the live chat.
        assert!(is_affirmation("はい、そういうものほしいです。"));
        assert!(is_affirmation("お願いします"));
        assert!(is_affirmation("そうして"));
        assert!(is_affirmation("OK"));
        assert!(is_affirmation("うん、いいよ"));
    }

    #[test]
    fn affirmations_refuse_lookalikes_and_other_speech() {
        assert!(!is_affirmation("はいそうです"));
        assert!(!is_affirmation("探して"));
        assert!(!is_affirmation("もっと安いのありますか？"));
        assert!(!is_affirmation(""));
    }

    #[test]
    fn closings_and_resets_match_their_phrases() {
        assert!(is_closing("ありがとうございました"));
        assert!(is_closing("助かりました"));
        assert!(is_closing("以上です"));
        assert!(!is_closing("ありがたいけど、もっと安いのある？"));
        assert!(is_reset_request("最初からやり直したい"));
        assert!(is_reset_request("やっぱりやめる"));
        assert!(!is_reset_request("やっぱり別のカテゴリーで"));
    }

    #[test]
    fn search_promises_match_the_stall_wording() {
        // The exact reply that stalled the live chat.
        assert!(promises_search("軽くて評価の高いものを探しています。"));
        assert!(promises_search("承知しました。お探しします。"));
        assert!(promises_search("検索します"));
        assert!(!promises_search("ご予算はどのくらいですか？"));
        assert!(!promises_search("かしこまりました。"));
    }

    #[test]
    fn transforms_read_presets_with_rating_first() {
        let both = follow_up_transform("安くて評価の高いものを教えてください").expect("transform");
        assert_eq!(both.preset, Some(Preset::BestRated));
        let cheap = follow_up_transform("もっと安いのありますか？").expect("transform");
        assert_eq!(cheap.preset, Some(Preset::Cheapest));
        let rated = follow_up_transform("人気のものが見たい").expect("transform");
        assert_eq!(rated.preset, Some(Preset::BestRated));
        assert_eq!(follow_up_transform("こんにちは"), None);
    }

    #[test]
    fn transforms_read_yen_amounts_as_budgets() {
        assert_eq!(
            follow_up_transform("予算は5000円までで").and_then(|t| t.budget_yen),
            Some(5_000)
        );
        assert_eq!(
            follow_up_transform("1万円までなら出せます").and_then(|t| t.budget_yen),
            Some(10_000)
        );
        assert_eq!(
            follow_up_transform("５０００円くらい").and_then(|t| t.budget_yen),
            Some(5_000)
        );
        assert_eq!(
            follow_up_transform("3,000円のもの").and_then(|t| t.budget_yen),
            Some(3_000)
        );
        assert_eq!(
            follow_up_transform("来週までにほしいです").and_then(|t| t.budget_yen),
            None
        );
    }
}

// ---- Decision enforcement and parsing (moved from agent_chat_service) ----
