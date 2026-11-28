//! Shopping-agent controller.
//!
//! The agent's one face: a conversation with the salesman
//! agent — each message is decided by the translator in
//! [`crate::services::agent_chat_service`], and a search
//! decision runs the engine into the visitor's own cart —
//! guest session or member, the
//! same owner rule as the cart itself. The add-anyway
//! endpoint is the one place a visitor may deliberately
//! cross a run's budget, and only for a variant that run
//! actually refused.
//!
//! The settlement also lives here: the
//! server-driven step machine that turns an agent-stocked
//! cart into an order — policy agreement, delivery address,
//! payment method, final confirmation, pay — rendered as
//! cards under the thread and advanced only by the step
//! buttons' form posts. Free-text chat never touches it;
//! the chat decision enum has no settlement action, by
//! design.

use axum::{
    Form, Router,
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
};
use serde::Deserialize;
use sqlx::PgPool;
use tower_sessions::Session;

use super::{header_for, product_cards, render};
use crate::messages::{
    EXECUTION_FAILED_NOTICE, ORDER_PROCESSING_FAILED_NOTICE, RESELECT_DELIVERY_MESSAGE,
};
use crate::models::agent::Preset;
use crate::models::{MemberAddress, PaymentMethod};
use crate::repositories::{product_repository, variant_repository};
use crate::services::agent_approval_service::{
    self, AgentSettlement, CreateSettlement, SettlementStage,
};
use crate::services::agent_chat_service::{ChatAction, ChatEntry, ChatResultSnapshot, ChatState};
use crate::services::cart_service::{self, AddOutcome, CartOwner};
use crate::services::order_service::{self, CheckoutFlow, PlaceError};

use crate::services::{
    agent_chat_rules, agent_chat_service, agent_run_service, member_address_service,
    payment_service,
};
use crate::session::{
    SESSION_AGENT_APPLIED, SESSION_AGENT_APPROVAL_NOTICE, SESSION_AGENT_CHAT,
    SESSION_AGENT_OVER_BUDGET, SESSION_AGENT_SETTLEMENT, SESSION_AGENT_SETTLEMENT_DISMISSED,
    SESSION_LAST_ORDER,
};
use crate::views::storefront::agent::{
    AgentChatFragmentView, AgentChatView, AgentPresetsView, AgentResultView, ChatEntryView,
    brief_chips,
};
use crate::views::storefront::agent_settlement::{
    AgentSettlementFragmentView, AgentSettlementView, SavedAddressView, SettlementAddressErrors,
    SettlementMethodView,
};
use crate::views::storefront::payment::PaymentErrorView;
use crate::views::storefront::product_card::ProductCard;

/// The agent routes.
pub fn router() -> Router<PgPool> {
    Router::new()
        .route("/agent", get(show_agent))
        .route("/agent/chat", post(chat_message))
        .route("/agent/preset", post(chat_preset))
        .route("/agent/brief/clear", post(chat_brief_clear))
        .route("/agent/reset", post(chat_reset))
        .route("/agent/add-anyway", post(add_anyway))
        .route("/agent/apply-pick", post(apply_pick))
        .route("/agent/apply-all", post(apply_all))
        .route("/agent/approve/void", post(void_approval))
        .route("/agent/settle/start", post(settle_start))
        .route("/agent/settle/continue", post(settle_continue))
        .route("/agent/settle/agree", post(settle_agree))
        .route("/agent/settle/cancel", post(settle_cancel))
        .route("/agent/settle/address", post(settle_address))
        .route("/agent/settle/method", post(settle_method))
        .route("/agent/settle/proceed", post(settle_proceed))
        .route("/agent/settle/back", post(settle_back))
        .route("/agent/settle/pay", post(settle_pay))
}

/// The settlement slot's contents for this visitor, if
/// any: the walk's current step on the chat
/// surface, its progress line on the cart surface, or the
/// offer to start settling when the cart holds agent
/// lines. Public to the cart controller, which renders the
/// same state read-only on /cart. A failed load is logged
/// and the slot is simply absent.
pub(crate) async fn load_settlement_view(
    pool: &PgPool,
    session: &Session,
    in_chat: bool,
) -> Option<AgentSettlementView> {
    let owner = cart_service::owner_for_read(pool, session).await;
    let (lines, totals) = match agent_approval_service::cart_state(pool, &owner).await {
        Ok(state) => state,
        Err(e) => {
            tracing::error!("agent settlement cart load failed: {e}");
            return None;
        }
    };
    let mut walk = load_walk(session).await;
    if !agent_approval_service::has_agent_line(&lines) {
        // Nothing for the agent path: any walk is over.
        if walk.is_some() {
            clear_walk(session).await;
        }
        return None;
    }
    let lines_fp = agent_approval_service::fingerprint_for(&lines, &totals);
    let snapshot = agent_approval_service::snapshot_for(&lines, &totals);
    let resolved = match agent_approval_service::resolve_latest(pool, &owner, &lines_fp).await {
        Ok(resolved) => resolved,
        Err(e) => {
            tracing::error!("agent settlement resolve failed: {e}");
            return None;
        }
    };
    let mut notice = resolved.notice;
    if notice.is_none() {
        notice = take_notice(session).await;
    }
    let user = crate::session::current_user(pool, session).await;

    if let Some(active) = &walk {
        match &resolved.live {
            Some(approval) if approval.id == active.approval_id => {
                if !in_chat {
                    // The cart surface shows progress
                    // only; the steps live in chat.
                    let mut view = AgentSettlementView::new("progress", false, &snapshot);
                    view.expires_label = Some(format!(
                        "{}",
                        approval
                            .expires_at
                            .with_timezone(
                                &chrono::FixedOffset::east_opt(9 * 3600)
                                    .expect("the JST offset is valid")
                            )
                            .format("%H:%M")
                    ));
                    return Some(view);
                }
                let mut view =
                    build_step_view(pool, user.as_ref(), active, approval, &lines, &snapshot).await;
                if notice.is_some() {
                    view.notice = notice;
                }
                return Some(view);
            }
            _ => {
                // The walk's record died (expiry, a cart
                // change, a void from /cart): the walk is
                // over and the offer comes back.
                clear_walk(session).await;
                walk = None;
            }
        }
    }

    if let Ok(Some(dismissed)) = session
        .get::<String>(SESSION_AGENT_SETTLEMENT_DISMISSED)
        .await
        && dismissed == lines_fp
    {
        return None;
    }
    let _ = walk;
    let mut view = AgentSettlementView::new("offer", in_chat, &snapshot);
    view.notice = notice;
    view.is_member = user.is_some();
    Some(view)
}

/// Renders the walk's current step from server state. The
/// cart surface never shows steps — it shows the progress
/// variant, because the settlement itself lives in chat.
async fn build_step_view(
    pool: &PgPool,
    user: Option<&crate::models::User>,
    walk: &AgentSettlement,
    approval: &crate::models::agent_approval::AgentApproval,
    lines: &[crate::models::CartLine],
    snapshot: &crate::models::agent_approval::ApprovalSnapshot,
) -> AgentSettlementView {
    let expires = format!(
        "{}",
        approval
            .expires_at
            .with_timezone(
                &chrono::FixedOffset::east_opt(9 * 3600).expect("the JST offset is valid")
            )
            .format("%H:%M")
    );
    let in_chat = true;
    match walk.stage {
        SettlementStage::Policy => {
            let mut view = AgentSettlementView::new("policy", in_chat, snapshot);
            view.expires_label = Some(expires);
            view
        }
        SettlementStage::Address => {
            let mut view = AgentSettlementView::new("address", in_chat, snapshot);
            view.is_member = user.is_some();
            view.prefectures = &crate::views::storefront::checkout::PREFECTURES;
            view.errors = SettlementAddressErrors::from_pairs(&walk.address_errors);
            if let Some(member) = user
                && let Ok(addresses) = member_address_service::list(pool, member.id).await
            {
                view.saved_addresses = addresses
                    .iter()
                    .map(|address| SavedAddressView {
                        id: address.id,
                        summary: saved_summary(address),
                        is_default: address.is_default,
                    })
                    .collect();
                if let Some(default) = addresses.iter().find(|address| address.is_default) {
                    view.default_address_id = Some(default.id);
                    view.default_address_summary = Some(saved_summary(default));
                }
            }
            // The typed fields show the walk's latest
            // address state: the chosen address after a
            // back-step, the kept draft after a failed
            // validation, the member profile otherwise.
            let draft = walk
                .delivery
                .clone()
                .or_else(|| user.map(CheckoutFlow::from_user));
            if let Some(flow) = draft {
                view.v_email = flow.email;
                view.v_family_name = flow.family_name;
                view.v_given_name = flow.given_name;
                view.v_postal_code = flow.postal_code;
                view.v_prefecture = flow.prefecture;
                view.v_city = flow.city;
                view.v_address_line = flow.address_line;
                view.v_building = flow.building;
                view.v_phone = flow.phone;
            }
            view
        }
        SettlementStage::Payment => {
            let mut view = AgentSettlementView::new("payment", in_chat, snapshot);
            // The chosen method (the delivery state's own
            // chosen flag, the human checkout's idiom) prices
            // this step in place: the same totals computation
            // the confirmation uses, so the two cards can
            // never disagree. Unchosen, the step shows the
            // no-method totals — no COD fee yet.
            let chosen = walk
                .delivery
                .as_ref()
                .filter(|delivery| delivery.payment_method_chosen)
                .map(|delivery| delivery.payment_method);
            let fees = crate::services::shop_settings_service::load(pool)
                .await
                .fee_settings();
            view.methods = payment_service::offered_methods()
                .iter()
                .map(|method| SettlementMethodView {
                    slug: method.slug(),
                    label: method.label(),
                    note: method.note(fees, payment_service::active_gateway_name()),
                    selected: chosen == Some(*method),
                })
                .collect();
            // Both method families are priced up front:
            // the step switches the displayed total between
            // them on the client when the shopper picks a
            // method (the human checkout's in-place idiom),
            // so a pick never waits on a round trip. The
            // confirmation recomputes by the recorded method
            // as before, so the two cards cannot disagree.
            let tax = crate::services::tax_service::params_today_or_fallback(pool).await;
            let fees = crate::services::shop_settings_service::load(pool)
                .await
                .fee_settings();
            let online =
                cart_service::totals(lines, 0, 0, Some(PaymentMethod::CreditCard), tax, fees);
            let cod = cart_service::totals(lines, 0, 0, Some(PaymentMethod::Cod), tax, fees);
            view.fill_payment_totals(&online, &cod, chosen);
            view.method_chosen = chosen.is_some();
            view
        }
        SettlementStage::Confirm => {
            let mut view = AgentSettlementView::new("confirm", in_chat, snapshot);
            view.expires_label = Some(expires);
            if let Some(delivery) = &walk.delivery {
                let method = delivery.payment_method;
                // The agreed totals are the method's: the
                // COD fee enters here and nowhere earlier.
                let tax = crate::services::tax_service::params_today_or_fallback(pool).await;
                let fees = crate::services::shop_settings_service::load(pool)
                    .await
                    .fee_settings();
                let totals = cart_service::totals(lines, 0, 0, Some(method), tax, fees);
                view.fill_totals(&totals);
                view.method_label = Some(method.label().to_string());
                view.delivery_name = Some(format!(
                    "{} {} 様",
                    delivery.family_name, delivery.given_name
                ));
                let mut address = format!(
                    "〒{} {} {} {}",
                    delivery.postal_code, delivery.prefecture, delivery.city, delivery.address_line
                );
                if !delivery.building.is_empty() {
                    address.push(' ');
                    address.push_str(&delivery.building);
                }
                view.delivery_address = Some(address);
                view.delivery_phone = Some(delivery.phone.clone());
                view.delivery_email = Some(delivery.email.clone());
            }
            view
        }
    }
}

/// One saved address as a single summary line.
fn saved_summary(address: &MemberAddress) -> String {
    let mut summary = format!(
        "{} {} 様　〒{} {} {} {}",
        address.family_name,
        address.given_name,
        address.postal_code,
        address.prefecture,
        address.city,
        address.address_line
    );
    if let Some(building) = &address.building
        && !building.is_empty()
    {
        summary.push(' ');
        summary.push_str(building);
    }
    summary
}

/// Loads the session's settlement walk, if any.
async fn load_walk(session: &Session) -> Option<AgentSettlement> {
    session
        .get::<AgentSettlement>(SESSION_AGENT_SETTLEMENT)
        .await
        .ok()
        .flatten()
}

async fn save_walk(session: &Session, walk: &AgentSettlement) {
    if let Err(e) = session.insert(SESSION_AGENT_SETTLEMENT, walk).await {
        tracing::error!("failed to save agent settlement: {e}");
    }
}

async fn clear_walk(session: &Session) {
    let _ = session
        .remove::<AgentSettlement>(SESSION_AGENT_SETTLEMENT)
        .await;
}

/// The one-shot settlement notice, taken (read + cleared).
async fn take_notice(session: &Session) -> Option<String> {
    match session
        .remove::<String>(SESSION_AGENT_APPROVAL_NOTICE)
        .await
    {
        Ok(notice) => notice,
        Err(e) => {
            tracing::error!("failed to take agent settlement notice: {e}");
            None
        }
    }
}

async fn set_notice(session: &Session, message: &str) {
    if let Err(e) = session
        .insert(SESSION_AGENT_APPROVAL_NOTICE, message.to_string())
        .await
    {
        tracing::error!("failed to record agent settlement notice: {e}");
    }
}

/// The walk in the session together with its approval,
/// verified live: the record must stand, belong to this
/// visitor and still match the cart. A dead record clears
/// the walk and leaves its notice for the next render.
async fn live_walk(
    pool: &PgPool,
    session: &Session,
) -> Option<(
    AgentSettlement,
    crate::models::agent_approval::AgentApproval,
)> {
    let walk = load_walk(session).await?;
    let owner = cart_service::owner_for_read(pool, session).await;
    let (lines, totals) = match agent_approval_service::cart_state(pool, &owner).await {
        Ok(state) => state,
        Err(e) => {
            tracing::error!("agent settlement cart load failed: {e}");
            return None;
        }
    };
    let lines_fp = agent_approval_service::fingerprint_for(&lines, &totals);
    let resolved = match agent_approval_service::resolve_latest(pool, &owner, &lines_fp).await {
        Ok(resolved) => resolved,
        Err(e) => {
            tracing::error!("agent settlement resolve failed: {e}");
            return None;
        }
    };
    match resolved.live {
        Some(approval) if approval.id == walk.approval_id => Some((walk, approval)),
        _ => {
            clear_walk(session).await;
            if let Some(notice) = resolved.notice {
                set_notice(session, &notice).await;
            }
            None
        }
    }
}

/// How a step post answers: the settlement fragment when
/// htmx drove it (the slot re-renders in place), a redirect
/// back to /agent otherwise — the page re-renders from
/// server state, which is also what makes the walk resume
/// after a reload.
async fn respond_settle(pool: &PgPool, session: &Session, headers: &HeaderMap) -> Response {
    if is_htmx(headers) {
        let view = AgentSettlementFragmentView {
            settlement: load_settlement_view(pool, session, true).await,
        };
        return render(view);
    }
    Redirect::to("/agent").into_response()
}

/// この内容で注文する — opens the settlement: the standing
/// approval for this exact cart (created now if none) plus
/// a fresh walk at the policy step.
async fn settle_start(
    State(pool): State<PgPool>,
    session: Session,
    headers: HeaderMap,
) -> Response {
    let owner = cart_service::owner_for(&pool, &session).await;
    let key = agent_approval_service::limit_key(&owner);
    match agent_approval_service::create_settlement_approval(&pool, &owner, &key).await {
        Ok(CreateSettlement::Created(approval)) => {
            save_walk(
                &session,
                &AgentSettlement {
                    approval_id: approval.id,
                    stage: SettlementStage::Policy,
                    delivery: None,
                    address_errors: Vec::new(),
                },
            )
            .await;
            let _ = session
                .remove::<String>(SESSION_AGENT_SETTLEMENT_DISMISSED)
                .await;
        }
        Ok(CreateSettlement::Capped) => {
            set_notice(&session, agent_approval_service::CAP_MESSAGE).await;
        }
        Ok(CreateSettlement::NoAgentLines) => {
            return Redirect::to("/cart").into_response();
        }
        Err(e) => {
            tracing::error!("agent settlement create failed: {e}");
            return Redirect::to("/cart").into_response();
        }
    }
    respond_settle(&pool, &session, &headers).await
}

/// 買い物を続ける — dismisses the offer for the cart as it
/// stands; it returns the moment the agent adds again or
/// the cart otherwise changes.
async fn settle_continue(
    State(pool): State<PgPool>,
    session: Session,
    headers: HeaderMap,
) -> Response {
    let owner = cart_service::owner_for_read(&pool, &session).await;
    if let Ok((lines, totals)) = agent_approval_service::cart_state(&pool, &owner).await {
        let fp = agent_approval_service::fingerprint_for(&lines, &totals);
        if let Err(e) = session.insert(SESSION_AGENT_SETTLEMENT_DISMISSED, fp).await {
            tracing::error!("failed to record agent settlement dismissal: {e}");
        }
    }
    if is_htmx(&headers) {
        let view = AgentSettlementFragmentView { settlement: None };
        return render(view);
    }
    Redirect::to(&back_path(&headers)).into_response()
}

/// 同意する (policy step) — the agreement is the step
/// itself; the walk advances to the address step.
async fn settle_agree(
    State(pool): State<PgPool>,
    session: Session,
    headers: HeaderMap,
) -> Response {
    if let Some((mut walk, _approval)) = live_walk(&pool, &session).await
        && walk.stage == SettlementStage::Policy
    {
        walk.stage = SettlementStage::Address;
        save_walk(&session, &walk).await;
    }
    respond_settle(&pool, &session, &headers).await
}

/// やめる / 手続きをやめる — voids the standing approval
/// and ends the walk; the offer comes back.
async fn settle_cancel(
    State(pool): State<PgPool>,
    session: Session,
    headers: HeaderMap,
) -> Response {
    let owner = cart_service::owner_for(&pool, &session).await;
    if let Err(e) = agent_approval_service::void_all_own(&pool, &owner).await {
        tracing::error!("agent settlement void failed: {e}");
    }
    clear_walk(&session).await;
    respond_settle(&pool, &session, &headers).await
}

/// `POST /agent/approve/void`: 手続きを取り消す from /cart —
/// revokes the settlement's approval and ends the walk.
/// The owner returns to the page they revoked from (a
/// path-only referer; anything else lands on the cart).
async fn void_approval(
    State(pool): State<PgPool>,
    session: Session,
    headers: HeaderMap,
) -> Response {
    let owner = cart_service::owner_for(&pool, &session).await;
    if let Err(e) = agent_approval_service::void_all_own(&pool, &owner).await {
        tracing::error!("agent approval void failed: {e}");
    }
    clear_walk(&session).await;
    Redirect::to(&back_path(&headers)).into_response()
}

/// Where a plain post returns to: the path-only referer,
/// restricted to the two settlement surfaces.
fn back_path(headers: &HeaderMap) -> String {
    headers
        .get("referer")
        .and_then(|value| value.to_str().ok())
        .and_then(|referer| referer.parse::<axum::http::Uri>().ok())
        .map(|uri| uri.path().to_string())
        .filter(|path| path == "/agent" || path == "/cart")
        .unwrap_or_else(|| "/cart".to_string())
}

/// The address step's post: either a saved address (the
/// member's quick pick or a picker choice) or the typed
/// form, validated by the checkout's own delivery rules.
/// A valid address advances to the payment step; an
/// invalid one stays on the step, keeping the draft and
/// the field errors.
async fn settle_address(
    State(pool): State<PgPool>,
    session: Session,
    headers: HeaderMap,
    Form(form): Form<SettleAddressForm>,
) -> Response {
    let Some((mut walk, _approval)) = live_walk(&pool, &session).await else {
        return respond_settle(&pool, &session, &headers).await;
    };
    if walk.stage != SettlementStage::Address {
        return respond_settle(&pool, &session, &headers).await;
    }
    let user = crate::session::current_user(&pool, &session).await;
    let choice = form.delivery_choice.trim().to_string();
    let flow = match (&user, choice.as_str()) {
        (Some(member), picked) if !picked.is_empty() && picked != "new" => {
            let saved = match picked.parse::<i64>() {
                Ok(id) => member_address_service::find_owned(&pool, member.id, id)
                    .await
                    .ok()
                    .flatten(),
                Err(_) => None,
            };
            match saved {
                Some(address) => flow_from_saved(&address, &member.email),
                None => {
                    set_notice(&session, RESELECT_DELIVERY_MESSAGE).await;
                    return respond_settle(&pool, &session, &headers).await;
                }
            }
        }
        _ => form.to_flow(user.as_ref()),
    };
    let settings = crate::services::shop_settings_service::load(&pool).await;
    let holidays = crate::services::holiday_service::upcoming(&pool).await;
    let errors = order_service::validate_delivery(&flow, user.is_some(), &settings, &holidays);
    walk.delivery = Some(flow);
    if errors.is_empty() {
        walk.address_errors.clear();
        walk.stage = SettlementStage::Payment;
    } else {
        walk.address_errors = errors
            .iter()
            .map(|error| (error.field.to_string(), error.message.clone()))
            .collect();
    }
    save_walk(&session, &walk).await;
    respond_settle(&pool, &session, &headers).await
}

/// A checkout flow pre-filled from a saved address.
fn flow_from_saved(address: &MemberAddress, email: &str) -> CheckoutFlow {
    CheckoutFlow {
        email: email.to_string(),
        family_name: address.family_name.clone(),
        given_name: address.given_name.clone(),
        postal_code: address.postal_code.clone(),
        prefecture: address.prefecture.clone(),
        city: address.city.clone(),
        address_line: address.address_line.clone(),
        building: address.building.clone().unwrap_or_default(),
        phone: address.phone.clone(),
        delivery_date: String::new(),
        delivery_slot: "none".to_string(),
        coupon_codes: Vec::new(),
        payment_method: PaymentMethod::Cod,
        payment_method_chosen: false,
        save_address: false,
        noshi: String::new(),
        noshi_name: String::new(),
        gift_message: String::new(),
        points_to_use: 0,
    }
}

/// The payment step's post: one method button. The chosen
/// method is recorded server-side and the walk STAYS on the
/// payment step, whose totals re-render in place for that
/// method (the COD fee enters and leaves with the choice);
/// 確認画面へ進む (settle_proceed) is what advances.
async fn settle_method(
    State(pool): State<PgPool>,
    session: Session,
    headers: HeaderMap,
    Form(form): Form<SettleMethodForm>,
) -> Response {
    let Some((mut walk, _approval)) = live_walk(&pool, &session).await else {
        return respond_settle(&pool, &session, &headers).await;
    };
    if walk.stage != SettlementStage::Payment {
        return respond_settle(&pool, &session, &headers).await;
    }
    if walk.delivery.is_none() {
        walk.stage = SettlementStage::Address;
        save_walk(&session, &walk).await;
        return respond_settle(&pool, &session, &headers).await;
    }
    let Some(method) = PaymentMethod::from_slug(&form.payment_method) else {
        return respond_settle(&pool, &session, &headers).await;
    };
    if !payment_service::offered_methods().contains(&method) {
        return respond_settle(&pool, &session, &headers).await;
    }
    if let Some(delivery) = walk.delivery.as_mut() {
        delivery.payment_method = method;
        delivery.payment_method_chosen = true;
    }
    save_walk(&session, &walk).await;
    respond_settle(&pool, &session, &headers).await
}

/// 確認画面へ進む — the payment step's way on: advances to
/// the final confirmation once a method stands chosen. With
/// none chosen the step simply re-renders (the button is
/// disabled in that state; this is the server-side guard).
async fn settle_proceed(
    State(pool): State<PgPool>,
    session: Session,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    // The proceed form carries the method picked on the
    // client (the payment step now switches methods in
    // place, with no post per pick): record it before the
    // advance check, through the same validation the
    // method endpoint applies. A body-less post — the
    // shape older clients and the test suites use — simply
    // keeps the method the walk already recorded.
    let picked = serde_urlencoded::from_bytes::<SettleMethodForm>(&body)
        .ok()
        .and_then(|form| PaymentMethod::from_slug(&form.payment_method))
        .filter(|method| payment_service::offered_methods().contains(method));
    if let Some((mut walk, _approval)) = live_walk(&pool, &session).await
        && walk.stage == SettlementStage::Payment
    {
        if let Some(method) = picked
            && let Some(delivery) = walk.delivery.as_mut()
        {
            delivery.payment_method = method;
            delivery.payment_method_chosen = true;
        }
        match &walk.delivery {
            Some(delivery) if delivery.payment_method_chosen => {
                walk.stage = SettlementStage::Confirm;
                save_walk(&session, &walk).await;
            }
            Some(_) => {}
            None => {
                walk.stage = SettlementStage::Address;
                save_walk(&session, &walk).await;
            }
        }
    }
    respond_settle(&pool, &session, &headers).await
}

/// The step-back post (住所を変更する / お支払い方法を変更する).
async fn settle_back(
    State(pool): State<PgPool>,
    session: Session,
    headers: HeaderMap,
    Form(form): Form<SettleBackForm>,
) -> Response {
    if let Some((mut walk, _approval)) = live_walk(&pool, &session).await {
        walk.stage = match form.to.as_str() {
            "address" => SettlementStage::Address,
            "payment" if walk.delivery.is_some() => SettlementStage::Payment,
            _ => walk.stage,
        };
        save_walk(&session, &walk).await;
    }
    respond_settle(&pool, &session, &headers).await
}

/// 支払う — the pay click. Verifies the approval (live,
/// owned, still bound to the cart's lines), re-binds it to
/// the final state agreed in the thread — the lines as
/// they stand right now, the method-dependent totals, the
/// method, the delivery address — then places the order
/// through the storefront's own machinery: KOMOJU methods
/// go out to the hosted page exactly like a human checkout,
/// COD completes here. The approval is consumed onto the
/// order in the same commitment; no payment session is
/// ever created without one.
async fn settle_pay(State(pool): State<PgPool>, session: Session) -> Response {
    let Some(walk) = load_walk(&session).await else {
        let placed: Option<i64> = session.get(SESSION_LAST_ORDER).await.ok().flatten();
        return if placed.is_some() {
            Redirect::to("/checkout/complete").into_response()
        } else {
            Redirect::to("/agent").into_response()
        };
    };
    if walk.stage != SettlementStage::Confirm {
        return Redirect::to("/agent").into_response();
    }
    let owner = cart_service::owner_for(&pool, &session).await;
    let approval =
        match agent_approval_service::verify_for_pay(&pool, &owner, walk.approval_id).await {
            Ok(approval) => approval,
            Err(refusal) => {
                clear_walk(&session).await;
                set_notice(&session, &refusal.message).await;
                return Redirect::to("/agent").into_response();
            }
        };
    let Some(delivery) = walk.delivery.clone() else {
        clear_walk(&session).await;
        return Redirect::to("/agent").into_response();
    };
    let method = delivery.payment_method;
    let (lines, _standing) = match agent_approval_service::cart_state(&pool, &owner).await {
        Ok(state) => state,
        Err(e) => {
            tracing::error!("agent settlement cart load failed: {e}");
            return Redirect::to("/agent").into_response();
        }
    };
    let tax = crate::services::tax_service::params_today_or_fallback(&pool).await;
    let fees = crate::services::shop_settings_service::load(&pool)
        .await
        .fee_settings();
    let totals = cart_service::totals(&lines, 0, 0, Some(method), tax, fees);
    if let Some(max) = method.max_total_yen()
        && totals.total_yen > max
    {
        set_notice(
            &session,
            &crate::messages::payment_method_limit_message(method.label(), max),
        )
        .await;
        return Redirect::to("/agent").into_response();
    }
    match agent_approval_service::bind_final(&pool, &approval, &lines, &totals, method, &delivery)
        .await
    {
        Ok(true) => {}
        Ok(false) => {
            clear_walk(&session).await;
            set_notice(
                &session,
                "この承認は無効になっています。お手数ですが、もう一度はじめからお試しください。",
            )
            .await;
            return Redirect::to("/agent").into_response();
        }
        Err(e) => {
            tracing::error!("agent settlement bind failed: {e}");
            set_notice(&session, "処理に失敗しました。もう一度お試しください。").await;
            return Redirect::to("/agent").into_response();
        }
    }
    // The bound figures double as the placement's expected
    // fingerprint: bind_final read the
    // cart unlocked, so any drift between that read and
    // placement's locked read — a price move, a quantity
    // change from another tab — refuses with PriceChanged
    // instead of placing under a stale approval.
    let bound_fingerprint = agent_approval_service::fingerprint_for(&lines, &totals);
    match order_service::place_order(
        &pool,
        &owner,
        &delivery,
        crate::models::order::PlacedVia::Agent,
        Some(&bound_fingerprint),
    )
    .await
    {
        Ok(order) => {
            match agent_approval_service::consume(&pool, approval.id, order.id).await {
                Ok(true) => {}
                Ok(false) => {
                    tracing::error!(
                        "agent approval {} did not consume onto order {}",
                        approval.id,
                        order.id
                    );
                }
                Err(e) => {
                    tracing::error!("agent approval consume failed: {e}");
                }
            }
            clear_walk(&session).await;
            let _ = session
                .remove::<String>(SESSION_AGENT_SETTLEMENT_DISMISSED)
                .await;
            if let Err(e) = session.insert(SESSION_LAST_ORDER, order.id).await {
                tracing::error!("failed to record last order: {e}");
            }
            if !method.settles_without_session(order.total_yen) {
                match payment_service::start_session(&pool, order.id).await {
                    Ok(url) => Redirect::to(&url).into_response(),
                    Err(e) => {
                        tracing::error!("agent KOMOJU session start failed: {e}");
                        let user = crate::session::current_user(&pool, &session).await;
                        render(PaymentErrorView {
                            header: header_for(&pool, &session, false).await,
                            order_number: order.order_number(),
                            is_member: user.is_some(),
                        })
                    }
                }
            } else {
                order_service::spawn_confirmation_mail(pool.clone(), order.id);
                Redirect::to("/checkout/complete").into_response()
            }
        }
        Err(PlaceError::StockShortage) => {
            tracing::warn!("agent settlement stock shortage");
            set_notice(
                &session,
                "在庫が不足している商品があります。カートの内容をご確認ください。",
            )
            .await;
            Redirect::to("/agent").into_response()
        }
        Err(PlaceError::Undeliverable(name)) => {
            set_notice(
                &session,
                &format!("「{name}」は現在お届けできない商品のため、注文を完了できませんでした。"),
            )
            .await;
            Redirect::to("/agent").into_response()
        }
        Err(PlaceError::DeliveryInvalid(message)) => {
            if let Some(mut walk) = load_walk(&session).await {
                walk.stage = SettlementStage::Address;
                save_walk(&session, &walk).await;
            }
            set_notice(&session, &message).await;
            Redirect::to("/agent").into_response()
        }
        Err(PlaceError::EmptyCart) => {
            let placed: Option<i64> = session.get(SESSION_LAST_ORDER).await.ok().flatten();
            if placed.is_some() {
                Redirect::to("/checkout/complete").into_response()
            } else {
                Redirect::to("/cart").into_response()
            }
        }
        Err(PlaceError::CouponInvalid) => {
            // The settlement never carries a coupon, so this
            // is unreachable in practice; it still explains
            // itself instead of placing anything.
            tracing::error!("agent settlement placement hit a coupon refusal");
            set_notice(&session, ORDER_PROCESSING_FAILED_NOTICE).await;
            Redirect::to("/agent").into_response()
        }
        Err(PlaceError::PaymentMethodLimit(message)) => {
            // The settlement checks the ceiling before binding
            // the approval, but placement re-checks against the
            // locked figures; a price move in between lands
            // here with the shopper-facing reason.
            set_notice(&session, &message).await;
            Redirect::to("/agent").into_response()
        }
        Err(PlaceError::PriceChanged) => {
            // The figures drifted between the approval's
            // binding read and placement's locked read
            // nothing was placed and the approval
            // stays unconsumed, so the shopper can re-walk
            // the settlement at the current figures.
            tracing::error!("agent settlement placement hit a price-change refusal");
            set_notice(&session, ORDER_PROCESSING_FAILED_NOTICE).await;
            Redirect::to("/agent").into_response()
        }
        Err(PlaceError::Db(e)) => {
            tracing::error!("agent settlement placement failed: {e}");
            set_notice(&session, ORDER_PROCESSING_FAILED_NOTICE).await;
            Redirect::to("/agent").into_response()
        }
    }
}

/// The address step's form.
#[derive(Debug, Deserialize, Default)]
pub struct SettleAddressForm {
    #[serde(default)]
    pub delivery_choice: String,
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub family_name: String,
    #[serde(default)]
    pub given_name: String,
    #[serde(default)]
    pub postal_code: String,
    #[serde(default)]
    pub prefecture: String,
    #[serde(default)]
    pub city: String,
    #[serde(default)]
    pub address_line: String,
    #[serde(default)]
    pub building: String,
    #[serde(default)]
    pub phone: String,
}

impl SettleAddressForm {
    /// The typed form as a checkout flow. A member's email
    /// is their account's; everything else is as typed.
    fn to_flow(&self, user: Option<&crate::models::User>) -> CheckoutFlow {
        CheckoutFlow {
            email: user
                .map(|member| member.email.clone())
                .unwrap_or_else(|| self.email.trim().to_string()),
            family_name: self.family_name.trim().to_string(),
            given_name: self.given_name.trim().to_string(),
            postal_code: self.postal_code.trim().to_string(),
            prefecture: self.prefecture.trim().to_string(),
            city: self.city.trim().to_string(),
            address_line: self.address_line.trim().to_string(),
            building: self.building.trim().to_string(),
            phone: self.phone.trim().to_string(),
            delivery_date: String::new(),
            delivery_slot: "none".to_string(),
            coupon_codes: Vec::new(),
            payment_method: PaymentMethod::Cod,
            payment_method_chosen: false,
            save_address: false,
            noshi: String::new(),
            noshi_name: String::new(),
            gift_message: String::new(),
            points_to_use: 0,
        }
    }
}

/// The payment step's form (one method button).
#[derive(Debug, Deserialize, Default)]
pub struct SettleMethodForm {
    #[serde(default)]
    pub payment_method: String,
}

/// The step-back form.
#[derive(Debug, Deserialize, Default)]
pub struct SettleBackForm {
    #[serde(default)]
    pub to: String,
}

/// `GET /agent`: the chat face of the shopping agent. (The
/// pre-chat brief form that used to share this path was
/// retired with its config flag; the chat is the only
/// surface now.)
async fn show_agent(State(pool): State<PgPool>, session: Session) -> Response {
    let state = load_chat_state(&session).await;
    render_chat_page(&pool, &session, &state, None).await
}

/// The run limiter's key for this visitor: the session
/// fingerprint — the same truncated hash the request log
/// carries, never the raw session ID. A session whose ID is
/// not assigned yet (its very first request) falls back to
/// the cart owner's own token, which lives in that same
/// session and so scopes the limit identically.
fn run_key(session: &Session, owner: &CartOwner) -> String {
    if let Some(id) = session.id() {
        return crate::util::logging::session_fingerprint(&id.to_string());
    }
    match owner {
        CartOwner::User(id) => crate::util::logging::session_fingerprint(&format!("user:{id}")),
        CartOwner::Session(token) => crate::util::logging::session_fingerprint(token),
    }
}

/// Loads a run's product cards (picks and refused picks) in
/// one query, with ratings and the viewer's favourite marks
/// attached by the shared card builder, exactly as the
/// catalog pages do. A failure degrades the cards to
/// name-only entries — the run itself already succeeded —
/// so it is logged, never raised.
async fn result_cards(pool: &PgPool, session: &Session, product_ids: &[i64]) -> Vec<ProductCard> {
    match product_repository::find_cards_by_ids(pool, product_ids).await {
        Ok(products) => {
            let viewer = crate::session::current_user(pool, session)
                .await
                .map(|user| user.id);
            product_cards(pool, viewer, products).await
        }
        Err(e) => {
            tracing::error!("agent result cards failed: {e}");
            Vec::new()
        }
    }
}

// ----------------------------------------------------------------
// The chat face.

/// Loads the conversation state from the session, or a fresh
/// one when this visitor has not chatted yet.
async fn load_chat_state(session: &Session) -> ChatState {
    session
        .get::<ChatState>(SESSION_AGENT_CHAT)
        .await
        .ok()
        .flatten()
        .unwrap_or_default()
}

/// Persists the conversation state. A store failure is
/// logged, never raised: the page still renders the turn.
async fn save_chat_state(session: &Session, state: &ChatState) {
    if let Err(e) = session.insert(SESSION_AGENT_CHAT, state.clone()).await {
        tracing::error!("agent chat state could not be stored: {e}");
    }
}

/// Converts stored entries to render entries: a result
/// entry re-reads its product cards (images, ratings) at
/// render time, while its run facts come from the snapshot.
async fn entry_views(
    pool: &PgPool,
    session: &Session,
    entries: &[ChatEntry],
) -> Vec<ChatEntryView> {
    // The applied set and the latest-proposal marker steer
    // the pick cards' apply buttons (see AgentResultView):
    // a pick already applied renders as in-cart, and only
    // the newest proposal offers apply-all.
    let applied: std::collections::HashSet<i64> = session
        .get::<Vec<i64>>(SESSION_AGENT_APPLIED)
        .await
        .ok()
        .flatten()
        .unwrap_or_default()
        .into_iter()
        .collect();
    let last_result = entries
        .iter()
        .rposition(|entry| matches!(entry, ChatEntry::Result { .. }));
    let mut views = Vec::with_capacity(entries.len());
    for entry in entries {
        match entry {
            ChatEntry::User { text } => views.push(ChatEntryView {
                is_user: true,
                is_result: false,
                text: text.clone(),
                result: None,
            }),
            ChatEntry::Agent { text } => views.push(ChatEntryView {
                is_user: false,
                is_result: false,
                text: text.clone(),
                result: None,
            }),
            ChatEntry::Result { message, snapshot } => {
                let card_ids: Vec<i64> = snapshot
                    .picks
                    .iter()
                    .chain(snapshot.over_budget.iter().map(|entry| &entry.pick))
                    .map(|pick| pick.product_id)
                    .collect();
                let cards = result_cards(pool, session, &card_ids).await;
                let is_latest = last_result == Some(views.len());
                views.push(ChatEntryView {
                    is_user: false,
                    is_result: true,
                    text: message.clone(),
                    result: Some(AgentResultView::from_snapshot(
                        snapshot, cards, &applied, is_latest,
                    )),
                });
            }
        }
    }
    views
}

/// Whether the request came from htmx (the composer, the
/// example chips, the preset chips): those answers are
/// fragments, never full pages.
fn is_htmx(headers: &HeaderMap) -> bool {
    headers
        .get("hx-request")
        .is_some_and(|value| value == "true")
}

/// Answers a chat turn. A plain post gets the full chat
/// page, as before. An htmx post gets only what the turn
/// changed: the entries it appended (a fragment for the
/// thread's tail), the note slot and the header cart badge
/// as out-of-band updates. The composer never moves, so
/// its CSRF token — one per session, rotated only on a
/// credential change — stays valid across any number of
/// swaps; the forms inside a result fragment are re-armed
/// by the CSRF middleware like any other response's.
/// `from` is the thread length before this turn ran.
async fn respond_chat(
    pool: &PgPool,
    session: &Session,
    state: &ChatState,
    note: Option<String>,
    from: usize,
    hx: bool,
) -> Response {
    if !hx {
        return render_chat_page(pool, session, state, note).await;
    }
    let tail = state.entries.get(from..).unwrap_or(&[]);
    let entries = entry_views(pool, session, tail).await;
    let user = crate::session::current_user(pool, session).await;
    let count = cart_service::count_for_header(pool, session, user.as_ref()).await;
    let settlement = load_settlement_view(pool, session, true).await;
    render(AgentChatFragmentView {
        entries,
        note,
        count,
        oob: true,
        settlement,
        brief_chips: brief_chips(&state.brief),
    })
}

/// Renders the chat page from a conversation state: result
/// entries re-read their product cards (images, ratings) at
/// render time, while the run facts come from the snapshot.
async fn render_chat_page(
    pool: &PgPool,
    session: &Session,
    state: &ChatState,
    note: Option<String>,
) -> Response {
    let entries = entry_views(pool, session, &state.entries).await;
    let settlement = load_settlement_view(pool, session, true).await;
    render(AgentChatView {
        header: header_for(pool, session, false).await,
        greeting: agent_chat_service::GREETING.to_string(),
        examples: agent_chat_service::EXAMPLE_PROMPTS
            .iter()
            .map(ToString::to_string)
            .collect(),
        entries,
        preset: state.brief.preset.form_value().to_string(),
        brief_chips: brief_chips(&state.brief),
        note,
        settlement,
    })
}

/// The posted chat message.
#[derive(Deserialize)]
pub struct ChatForm {
    /// The message text.
    #[serde(default)]
    pub message: String,
}

/// `POST /agent/chat`: one conversation turn. Words that
/// abandon the conversation (「最初から」「やめる」) are the
/// reset button in text form: they clear the state exactly
/// like `POST /agent/reset`, with no translator call at all.
/// Otherwise the translator decides and the deterministic
/// rules in `agent_chat_service::enforce_rules` have the last
/// word; only a `search` decision touches the engine, and it
/// calls ONLY the read-only proposal operation — a chat turn
/// can never write to the cart (the design rule of
/// 2026-11-09: replies propose, the shopper applies). Only
/// a search consumes a run from the hourly cap. A failed
/// translator call still records the visitor's message, with
/// the agent's fallback bubble after it — the brief and the
/// cart are untouched.
async fn chat_message(
    State(pool): State<PgPool>,
    session: Session,
    headers: HeaderMap,
    Form(form): Form<ChatForm>,
) -> Response {
    let mut state = load_chat_state(&session).await;
    let from = state.entries.len();
    let hx = is_htmx(&headers);
    let text = form.message.trim().to_string();
    if text.is_empty() {
        return respond_chat(
            &pool,
            &session,
            &state,
            Some("メッセージを入力してください。".to_string()),
            from,
            hx,
        )
        .await;
    }
    if agent_chat_rules::is_reset_request(&text) {
        let _ = session.remove::<ChatState>(SESSION_AGENT_CHAT).await;
        let _ = session.remove::<Vec<i64>>(SESSION_AGENT_OVER_BUDGET).await;
        let _ = session.remove::<Vec<i64>>(SESSION_AGENT_APPLIED).await;
        if hx {
            // htmx would swap a followed redirect's page
            // into the thread; the HX-Redirect header asks
            // it for a real navigation to the fresh page.
            return (StatusCode::NO_CONTENT, [("HX-Redirect", "/agent")]).into_response();
        }
        return Redirect::to("/agent").into_response();
    }
    let decision = match agent_chat_service::decide(&pool, &state, &text).await {
        Ok(decision) => decision,
        Err(e) => {
            // The reason only: never the prompt, the response
            // body, or the visitor's text (logging rules).
            tracing::error!("agent chat decision failed: {e}");
            state.last_turn_was_question = false;
            state.push(ChatEntry::User { text });
            state.push(ChatEntry::Agent {
                text: agent_chat_service::FALLBACK_MESSAGE.to_string(),
            });
            save_chat_state(&session, &state).await;
            return respond_chat(&pool, &session, &state, None, from, hx).await;
        }
    };
    state.push(ChatEntry::User { text });
    agent_chat_service::merge_brief(&mut state.brief, &decision.updates);
    let mut ran_search = false;
    match decision.action {
        ChatAction::Ask | ChatAction::Reply => {
            state.push(ChatEntry::Agent {
                text: decision.message,
            });
        }
        ChatAction::Search => {
            let owner = cart_service::owner_for(&pool, &session).await;
            // The cap counts searches, not conversation: an
            // ask or a reply above never reaches this line.
            if !agent_run_service::try_begin_run(&run_key(&session, &owner)) {
                state.push(ChatEntry::Agent {
                    text: agent_run_service::RUN_LIMIT_MESSAGE.to_string(),
                });
            } else {
                let brief = state.brief.to_brief();
                match agent_run_service::propose(&pool, &owner, &brief).await {
                    Ok(proposal) => {
                        ran_search = true;
                        // The add-anyway allow-list, exactly
                        // as the form flow writes it: this
                        // proposal's refused variants, this
                        // session.
                        let refused: Vec<i64> = proposal
                            .over_budget
                            .iter()
                            .map(|entry| entry.pick.variant_id)
                            .collect();
                        let _ = session.insert(SESSION_AGENT_OVER_BUDGET, refused).await;
                        // A new proposal starts with nothing
                        // applied: the applied set describes
                        // this proposal only.
                        let _ = session
                            .insert(SESSION_AGENT_APPLIED, Vec::<i64>::new())
                            .await;
                        let snapshot =
                            ChatResultSnapshot::from_proposal(&proposal, brief.budget_yen);
                        state.push(ChatEntry::Result {
                            message: decision.message,
                            snapshot,
                        });
                    }
                    Err(e) => {
                        tracing::error!("agent chat proposal failed: {e}");
                        state.push(ChatEntry::Agent {
                            text: EXECUTION_FAILED_NOTICE.to_string(),
                        });
                    }
                }
            }
        }
    }
    // The rules' bookkeeping for the next turn: an asked
    // question spends from the question budget and arms the
    // confirmation rule; a search that actually ran refills
    // the budget; anything else ends the question.
    match decision.action {
        ChatAction::Ask => {
            state.asks_since_search = state.asks_since_search.saturating_add(1);
            state.last_turn_was_question = true;
        }
        ChatAction::Search if ran_search => {
            state.asks_since_search = 0;
            state.last_turn_was_question = false;
        }
        _ => {
            state.last_turn_was_question = false;
        }
    }
    save_chat_state(&session, &state).await;
    respond_chat(&pool, &session, &state, None, from, hx).await
}

/// The posted preset chip.
#[derive(Deserialize)]
pub struct PresetForm {
    /// Preset form value.
    #[serde(default)]
    pub preset: String,
}

/// `POST /agent/preset`: sets the conversation's scoring
/// preset from a chip tap, then returns to the chat (an
/// htmx tap is answered with just the chips). An
/// unknown value is ignored — the chips are the only writers
/// in practice, and a forged post must not break the thread.
async fn chat_preset(
    session: Session,
    headers: HeaderMap,
    Form(form): Form<PresetForm>,
) -> Response {
    if let Some(preset) = Preset::from_form_value(form.preset.trim()) {
        let mut state = load_chat_state(&session).await;
        state.brief.preset = preset;
        state.brief.preset_explicit = true;
        save_chat_state(&session, &state).await;
        if is_htmx(&headers) {
            // Just the chips back, with the new active
            // one: the thread and the composer stay put.
            return render(AgentPresetsView {
                preset: preset.form_value().to_string(),
            });
        }
    }
    Redirect::to("/agent").into_response()
}

/// The posted brief chip clear: which brief slot, and for
/// a list slot which need.
#[derive(Deserialize)]
pub struct BriefClearForm {
    /// The brief slot ("budget", "category", "must_have",
    /// "nice_to_have").
    #[serde(default)]
    pub field: String,
    /// The need text for a list slot.
    #[serde(default)]
    pub value: String,
}

/// `POST /agent/brief/clear`: removes one accumulated brief
/// condition at the visitor's tap — no translator call: the
/// brief mutates deterministically, and when a target still
/// remains, the search re-runs at once under the relaxed
/// brief and lands in the thread like a search turn's
/// result. A stale or forged post changes nothing and
/// simply re-renders.
async fn chat_brief_clear(
    State(pool): State<PgPool>,
    session: Session,
    headers: HeaderMap,
    Form(form): Form<BriefClearForm>,
) -> Response {
    let mut state = load_chat_state(&session).await;
    let from = state.entries.len();
    let hx = is_htmx(&headers);
    let Some(removed) =
        agent_chat_service::clear_brief_item(&mut state.brief, &form.field, &form.value)
    else {
        return respond_chat(&pool, &session, &state, None, from, hx).await;
    };
    if agent_chat_service::has_signal(&state.brief) {
        let owner = cart_service::owner_for(&pool, &session).await;
        if !agent_run_service::try_begin_run(&run_key(&session, &owner)) {
            state.push(ChatEntry::Agent {
                text: agent_run_service::RUN_LIMIT_MESSAGE.to_string(),
            });
            state.last_turn_was_question = false;
        } else {
            let brief = state.brief.to_brief();
            match agent_run_service::propose(&pool, &owner, &brief).await {
                Ok(proposal) => {
                    let refused: Vec<i64> = proposal
                        .over_budget
                        .iter()
                        .map(|entry| entry.pick.variant_id)
                        .collect();
                    let _ = session.insert(SESSION_AGENT_OVER_BUDGET, refused).await;
                    let _ = session
                        .insert(SESSION_AGENT_APPLIED, Vec::<i64>::new())
                        .await;
                    let snapshot = ChatResultSnapshot::from_proposal(&proposal, brief.budget_yen);
                    state.push(ChatEntry::Result {
                        message: format!("{removed}を外して、探し直しました。"),
                        snapshot,
                    });
                    state.asks_since_search = 0;
                    state.last_turn_was_question = false;
                }
                Err(e) => {
                    tracing::error!("agent chat proposal failed: {e}");
                    state.push(ChatEntry::Agent {
                        text: EXECUTION_FAILED_NOTICE.to_string(),
                    });
                    state.last_turn_was_question = false;
                }
            }
        }
    } else {
        state.push(ChatEntry::Agent {
            text: format!("{removed}を外しました。"),
        });
        state.last_turn_was_question = false;
    }
    save_chat_state(&session, &state).await;
    respond_chat(&pool, &session, &state, None, from, hx).await
}

/// `POST /agent/reset`: clears the conversation — thread and
/// accumulated brief, plus the add-anyway allow-list the last
/// search wrote. The settlement lifecycle ends with it too:
/// any live approval is voided, the walk and its notice are
/// dropped, and the order offer is dismissed for the cart as
/// it stands — the same dismissal `settle_continue` records —
/// so the old consultation's proposal cannot re-render from
/// the cart alone. The cart itself is untouched: clearing a
/// conversation is not clearing a cart, and the offer returns
/// if the cart changes.
async fn chat_reset(State(pool): State<PgPool>, session: Session) -> Response {
    let _ = session.remove::<ChatState>(SESSION_AGENT_CHAT).await;
    let _ = session.remove::<Vec<i64>>(SESSION_AGENT_OVER_BUDGET).await;
    let _ = session.remove::<Vec<i64>>(SESSION_AGENT_APPLIED).await;
    let owner = cart_service::owner_for_read(&pool, &session).await;
    if let Err(e) = agent_approval_service::void_all_own(&pool, &owner).await {
        tracing::error!("agent approval void on chat reset failed: {e}");
    }
    clear_walk(&session).await;
    let _ = session
        .remove::<String>(SESSION_AGENT_APPROVAL_NOTICE)
        .await;
    if let Ok((lines, totals)) = agent_approval_service::cart_state(&pool, &owner).await
        && agent_approval_service::has_agent_line(&lines)
    {
        let fp = agent_approval_service::fingerprint_for(&lines, &totals);
        if let Err(e) = session.insert(SESSION_AGENT_SETTLEMENT_DISMISSED, fp).await {
            tracing::error!("failed to record agent settlement dismissal: {e}");
        }
    }
    Redirect::to("/agent").into_response()
}

/// Form posted by an over-budget entry's add-anyway button:
/// the variant the run refused.
#[derive(Deserialize)]
pub struct AddAnywayForm {
    /// Variant id as typed.
    #[serde(default)]
    pub variant_id: String,
}

/// `POST /agent/add-anyway`: adds one budget-refused variant
/// as an agent line, at the visitor's explicit instruction,
/// then sends them to the cart where the line is visible.
///
/// Only a variant on this session's allow-list — written by
/// the visitor's own latest run — may be added this way: the
/// budget is the agent's rule, and a crafted post must not
/// mint agent lines for arbitrary items. Anything else is
/// refused back to /agent.
async fn add_anyway(
    State(pool): State<PgPool>,
    session: Session,
    Form(form): Form<AddAnywayForm>,
) -> Response {
    let Ok(variant_id) = form.variant_id.parse::<i64>() else {
        return (StatusCode::NOT_FOUND, "Not Found").into_response();
    };
    let mut allowed: Vec<i64> = session
        .get(SESSION_AGENT_OVER_BUDGET)
        .await
        .ok()
        .flatten()
        .unwrap_or_default();
    if !allowed.contains(&variant_id) {
        return Redirect::to("/agent").into_response();
    }
    let variant = match variant_repository::find_variant(&pool, variant_id).await {
        Ok(Some(variant)) => variant,
        Ok(None) => return (StatusCode::NOT_FOUND, "Not Found").into_response(),
        Err(e) => {
            tracing::error!(variant_id, "add-anyway variant lookup failed: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error").into_response();
        }
    };
    let owner = cart_service::owner_for(&pool, &session).await;
    match cart_service::add_to_cart_as_agent(&pool, &owner, variant.product_id, variant_id, 1).await
    {
        Ok(AddOutcome::Added(_)) | Ok(AddOutcome::Capped(_)) | Ok(AddOutcome::SoldOut) => {
            // One deliberate add consumes the refusal: the
            // same variant cannot be re-added from a stale
            // results page.
            allowed.retain(|id| *id != variant_id);
            let _ = session.insert(SESSION_AGENT_OVER_BUDGET, allowed).await;
            Redirect::to("/cart").into_response()
        }
        Ok(AddOutcome::NotFound) => (StatusCode::NOT_FOUND, "Not Found").into_response(),
        Err(e) => {
            tracing::error!("agent add-anyway failed: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error").into_response()
        }
    }
}

/// The picks of the thread's latest proposal: the only
/// picks an apply action may write. Older results are
/// history — their per-pick buttons still post, and resolve
/// against this list, so a pick the newest proposal did not
/// repeat is refused.
fn latest_proposal_picks(state: &ChatState) -> Vec<crate::services::agent_run_service::AgentPick> {
    state
        .entries
        .iter()
        .rev()
        .find_map(|entry| match entry {
            ChatEntry::Result { snapshot, .. } => Some(snapshot.picks.clone()),
            _ => None,
        })
        .unwrap_or_default()
}

/// `POST /agent/apply-pick`: applies ONE pick of the
/// visitor's latest chat proposal — the per-pick
/// 「カートに入れる」 button on a pick card. A deliberate
/// shopper action carrying the pick it applies, so it may
/// call the engine's apply operation; the pick lands
/// alongside the existing lines (no replace). A variant the
/// latest proposal did not offer is refused back to /agent:
/// a crafted post must not mint agent lines for arbitrary
/// items.
async fn apply_pick(
    State(pool): State<PgPool>,
    session: Session,
    Form(form): Form<AddAnywayForm>,
) -> Response {
    let Ok(variant_id) = form.variant_id.parse::<i64>() else {
        return (StatusCode::NOT_FOUND, "Not Found").into_response();
    };
    let state = load_chat_state(&session).await;
    let picks = latest_proposal_picks(&state);
    let Some(pick) = picks.iter().find(|pick| pick.variant_id == variant_id) else {
        return Redirect::to("/agent").into_response();
    };
    let owner = cart_service::owner_for(&pool, &session).await;
    match agent_run_service::apply(&pool, &owner, std::slice::from_ref(pick), false).await {
        Ok(outcome) => {
            if !outcome.landed.is_empty() {
                let mut applied: Vec<i64> = session
                    .get(SESSION_AGENT_APPLIED)
                    .await
                    .ok()
                    .flatten()
                    .unwrap_or_default();
                if !applied.contains(&variant_id) {
                    applied.push(variant_id);
                    let _ = session.insert(SESSION_AGENT_APPLIED, applied).await;
                }
            }
            Redirect::to("/agent").into_response()
        }
        Err(e) => {
            tracing::error!("agent apply-pick failed: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error").into_response()
        }
    }
}

/// `POST /agent/apply-all`: applies the visitor's latest
/// chat proposal in full — the explicit 「すべてカートに入れる」
/// action under the newest result. A deliberate shopper
/// action carrying the whole proposal, so it may call the
/// engine's apply operation with the replace rule: the
/// applied proposal supersedes the owner's previous agent
/// lines. With no proposal, or a proposal with no picks,
/// it changes nothing.
async fn apply_all(State(pool): State<PgPool>, session: Session) -> Response {
    let state = load_chat_state(&session).await;
    let picks = latest_proposal_picks(&state);
    if picks.is_empty() {
        return Redirect::to("/agent").into_response();
    }
    let owner = cart_service::owner_for(&pool, &session).await;
    match agent_run_service::apply(&pool, &owner, &picks, true).await {
        Ok(outcome) => {
            let applied: Vec<i64> = outcome.landed.iter().map(|pick| pick.variant_id).collect();
            let _ = session.insert(SESSION_AGENT_APPLIED, applied).await;
            Redirect::to("/agent").into_response()
        }
        Err(e) => {
            tracing::error!("agent apply-all failed: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error").into_response()
        }
    }
}
