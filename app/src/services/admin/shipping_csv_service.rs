//! Carrier label CSV export and tracking-number CSV import.
//!
//! The daily shipping run's paper half: unshipped orders go
//! out in the layout each carrier's label tool imports
//! (ヤマト B2クラウド, 佐川 e飛伝III, 日本郵便
//! ゆうパックプリントR), and the printed labels' tracking
//! numbers come back as one small CSV whose rows ship their
//! orders through the same guarded transition as the detail
//! form. Sender (ご依頼主) fields carry the shop name only:
//! the shop settings hold no postal address, and the carrier
//! tools fall back to the sender registered on the operator's
//! carrier account for the rest — inventing an address here
//! would print wrong labels.

use sqlx::PgPool;

use crate::error::AppError;
use crate::messages::{
    CARRIER_INVALID_MESSAGE, ORDER_NOT_FOUND_MESSAGE, SHIPMENT_BLOCKED_UNPAID_MESSAGE,
    SHIPPING_CSV_LOAD_FAILED_MESSAGE, SHIPPING_CSV_UPDATE_FAILED_MESSAGE,
    STATUS_ALREADY_CHANGED_MESSAGE, STATUS_CHANGE_NOT_ALLOWED_MESSAGE,
};
use crate::util::csv_transfer::{LineError, parse_rows};
use std::collections::HashMap;

use crate::models::Order;
use crate::models::carrier::carrier_for;
use crate::repositories::{order_repository, payment_repository};
use crate::services::admin::order_service::{self, StatusChangeError};

/// Most orders one label export may carry.
const MAX_LABEL_ORDERS: usize = 500;

/// The tracking import's header row, checked exactly.
const IMPORT_HEADERS: [&str; 3] = ["注文番号", "配送会社", "送り状番号"];

/// One exported label file, ready to download.
pub struct LabelExport {
    /// Download file name.
    pub filename: String,
    /// Shift_JIS CSV bytes (the carrier tools' native encoding).
    pub bytes: Vec<u8>,
    /// Orders written to the file.
    pub order_count: usize,
    /// Unshipped orders left out because their KOMOJU payment
    /// has not settled (they may not ship yet).
    pub skipped_unsettled: usize,
}

/// Builds the label CSV for one carrier. With `ids`, only
/// those orders are considered; without, every unshipped
/// order. Only orders that may actually ship are written:
/// received/preparing, with a settled payment or cash on
/// delivery — the same guard the ship transition enforces.
///
/// # Errors
///
/// Returns [`AppError`] on a database failure, or
/// [`AppError::NotFound`] for a carrier code outside the
/// catalogue (the controller turns it into a form error).
pub async fn export_labels(
    pool: &PgPool,
    carrier_code: &str,
    ids: Option<&[i64]>,
    shop_name: &str,
    today: chrono::NaiveDate,
) -> Result<LabelExport, AppError> {
    if carrier_for(carrier_code).is_none() || carrier_code == "other" {
        return Err(AppError::NotFound);
    }
    let candidates: Vec<Order> = match ids {
        Some(ids) => {
            // One bulk load, then the operator's selection
            // order re-imposed: the file lists the orders in
            // the order they were picked. A repeated id names
            // its order once — a parcel never gets two labels.
            let found = order_repository::find_by_ids(pool, ids)
                .await
                .map_err(|e| AppError::db("label export order load", e))?;
            let mut by_id: HashMap<i64, Order> = found.into_iter().map(|o| (o.id, o)).collect();
            ids.iter().filter_map(|id| by_id.remove(id)).collect()
        }
        None => order_repository::find_by_statuses(
            pool,
            &["received", "preparing"],
            MAX_LABEL_ORDERS as i64,
        )
        .await
        .map_err(|e| AppError::db("label export order list", e))?,
    };
    let shippable: Vec<Order> = candidates
        .into_iter()
        .filter(|o| o.status == "received" || o.status == "preparing")
        .collect();
    // The payments and lines of every shippable order load
    // in one query each: the per-order reads this replaces
    // cost two round trips per order on a 500-label run.
    let order_ids: Vec<i64> = shippable.iter().map(|o| o.id).collect();
    let payments: HashMap<i64, crate::models::Payment> =
        payment_repository::find_by_order_ids(pool, &order_ids)
            .await
            .map_err(|e| AppError::db("label export payment load", e))?
            .into_iter()
            .map(|p| (p.order_id, p))
            .collect();
    let mut lines_by_order: HashMap<i64, Vec<crate::models::OrderLine>> = HashMap::new();
    for (order_id, line) in order_repository::lines_for_orders(pool, &order_ids)
        .await
        .map_err(|e| AppError::db("label export lines", e))?
    {
        lines_by_order.entry(order_id).or_default().push(line);
    }
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut skipped_unsettled = 0usize;
    for order in shippable {
        let payment = payments.get(&order.id);
        let is_cod = payment.is_some_and(|p| p.provider == "cod");
        let settled = payment.is_some_and(|p| p.is_paid());
        if !is_cod && !settled {
            skipped_unsettled += 1;
            continue;
        }
        let lines = lines_by_order
            .get(&order.id)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let first_item = lines
            .first()
            .map(|l| l.product_name.clone())
            .unwrap_or_default();
        let item_note = if lines.len() > 1 {
            format!("{first_item} ほか{}点", lines.len() - 1)
        } else {
            first_item
        };
        let cod_amount = if is_cod { order.total_yen } else { 0 };
        rows.push(label_row(
            carrier_code,
            &order,
            shop_name,
            today,
            &item_note,
            cod_amount,
        ));
        if rows.len() >= MAX_LABEL_ORDERS {
            break;
        }
    }
    let headers = label_headers(carrier_code);
    let text = crate::util::csv_transfer::write_csv_text(&headers, &rows);
    // The carrier tools read Shift_JIS; encoding_rs replaces
    // the rare unmappable character rather than failing the
    // whole run.
    let (bytes, _, _) = encoding_rs::SHIFT_JIS.encode(&text);
    Ok(LabelExport {
        filename: format!("labels_{carrier_code}_{}.csv", today.format("%Y%m%d")),
        bytes: bytes.into_owned(),
        order_count: rows.len(),
        skipped_unsettled,
    })
}

/// The carriers that have a label layout: the export's
/// universe (the catalogue's `other` has no layout to print).
pub const LABEL_CARRIER_CODES: [&str; 3] = ["yamato", "sagawa", "japan_post"];

/// The header row of one carrier's label layout.
fn label_headers(carrier_code: &str) -> Vec<&'static str> {
    match carrier_code {
        "yamato" => vec![
            "お客様管理番号",
            "送り状種類",
            "クール区分",
            "伝票番号",
            "出荷予定日",
            "お届け予定日",
            "配達時間帯",
            "お届け先電話番号",
            "お届け先郵便番号",
            "お届け先住所",
            "お届け先アパートマンション名",
            "お届け先名",
            "ご依頼主電話番号",
            "ご依頼主郵便番号",
            "ご依頼主住所",
            "ご依頼主名",
            "品名１",
            "品名２",
            "代金引換額",
        ],
        "sagawa" => vec![
            "お客様管理番号",
            "出荷日",
            "送り状番号",
            "配送指定日",
            "配達指定時間帯",
            "お届け先郵便番号",
            "お届け先住所１",
            "お届け先住所２",
            "お届け先住所３",
            "お届け先名称１",
            "お届け先電話番号",
            "ご依頼主郵便番号",
            "ご依頼主住所１",
            "ご依頼主名称１",
            "品名１",
            "代引金額",
        ],
        _ => vec![
            "お客様管理番号",
            "お届け先郵便番号",
            "お届け先住所",
            "お届け先氏名",
            "お届け先電話番号",
            "品名",
            "配達希望日",
            "配達時間帯",
            "代金引換額",
            "ご依頼主郵便番号",
            "ご依頼主住所",
            "ご依頼主氏名",
            "ご依頼主電話番号",
        ],
    }
}

/// One label row in the carrier's column order. The recipient
/// is the order's delivery address; the sender is the shop
/// name with address fields blank (see the module note).
fn label_row(
    carrier_code: &str,
    order: &Order,
    shop_name: &str,
    today: chrono::NaiveDate,
    item_note: &str,
    cod_amount: i64,
) -> Vec<String> {
    let recipient = format!("{} {}", order.family_name, order.given_name);
    let postal = order.postal_code.replace('-', "");
    let address = format!(
        "{}{}{}{}",
        order.prefecture,
        order.city,
        order.address_line,
        order.building.clone().unwrap_or_default()
    );
    let ship_date = today.format("%Y/%m/%d").to_string();
    let delivery_date = order
        .delivery_date
        .map(|d| d.format("%Y/%m/%d").to_string())
        .unwrap_or_default();
    let cod = if cod_amount > 0 {
        cod_amount.to_string()
    } else {
        String::new()
    };
    match carrier_code {
        "yamato" => vec![
            order.order_number(),
            "0".to_string(), // 宅急便
            "0".to_string(), // 通常（クールでない）
            String::new(),   // 伝票番号: printed by B2
            ship_date,
            delivery_date,
            yamato_slot_code(&order.delivery_slot).to_string(),
            order.phone.clone(),
            postal,
            format!("{}{}{}", order.prefecture, order.city, order.address_line),
            order.building.clone().unwrap_or_default(),
            recipient,
            String::new(),
            String::new(),
            String::new(),
            shop_name.to_string(),
            item_note.to_string(),
            String::new(),
            cod,
        ],
        "sagawa" => vec![
            order.order_number(),
            ship_date,
            String::new(), // 送り状番号: printed by e飛伝
            delivery_date,
            sagawa_slot_code(&order.delivery_slot).to_string(),
            postal,
            order.prefecture.clone(),
            order.city.clone(),
            format!(
                "{}{}",
                order.address_line,
                order.building.clone().unwrap_or_default()
            ),
            recipient,
            order.phone.clone(),
            String::new(),
            String::new(),
            shop_name.to_string(),
            item_note.to_string(),
            cod,
        ],
        _ => vec![
            order.order_number(),
            postal,
            address,
            recipient,
            order.phone.clone(),
            item_note.to_string(),
            delivery_date,
            japan_post_slot_label(&order.delivery_slot).to_string(),
            cod,
            String::new(),
            String::new(),
            shop_name.to_string(),
            String::new(),
        ],
    }
}

/// The B2クラウド delivery-band code for one slot: the
/// carrier's own band numbering (午前中 is 0812, the evening
/// bands are their start-end hours).
fn yamato_slot_code(slot: &str) -> &'static str {
    match slot {
        "am" => "0812",
        "t1416" => "1416",
        "t1618" => "1618",
        "t1820" => "1820",
        "t1921" => "1921",
        _ => "",
    }
}

/// The e飛伝III delivery-band code for one slot (the band's
/// opening hour; 午前中 is 01).
fn sagawa_slot_code(slot: &str) -> &'static str {
    match slot {
        "am" => "01",
        "t1416" => "14",
        "t1618" => "16",
        "t1820" => "18",
        "t1921" => "19",
        _ => "",
    }
}

/// The ゆうパックプリントR delivery-band label for one slot.
fn japan_post_slot_label(slot: &str) -> &'static str {
    match slot {
        "am" => "午前中",
        "t1416" => "14時-16時",
        "t1618" => "16時-18時",
        "t1820" => "18時-20時",
        "t1921" => "19時-21時",
        _ => "希望なし",
    }
}

/// One tracking import row's outcome.
pub struct TrackingRowOutcome {
    /// File line the row came from (header is line 1).
    pub line: usize,
    /// The posted order number, verbatim.
    pub number: String,
    /// Whether the row was applied.
    pub applied: bool,
    /// What happened, for the results page.
    pub message: String,
}

/// The tracking import's whole outcome: one entry per row.
pub struct TrackingImportReport {
    /// Per-row outcomes, in file order.
    pub rows: Vec<TrackingRowOutcome>,
}

impl TrackingImportReport {
    /// How many rows were applied.
    pub fn applied_count(&self) -> usize {
        self.rows.iter().filter(|r| r.applied).count()
    }
}

/// Applies a tracking-number CSV: each row ships its order
/// (or records the number on an already-shipped order that
/// has none), through the same state checks as the detail
/// form, with the shipping mail queued rather than sent from
/// this request. A received order walks the flow via
/// preparing as two audited transitions, and an unsettled
/// KOMOJU order is refused before anything moves. Rows are independent: a bad row is reported
/// and never half-applies, and never blocks its neighbours.
///
/// # Errors
///
/// Returns a [`LineError`] at line 1 when the file's header
/// does not match the documented format, and [`AppError`]
/// wrapped outcomes are per-row — a database failure on one
/// row is reported on that row, not raised.
pub async fn import_tracking(
    pool: &PgPool,
    admin_id: i64,
    text: &str,
) -> Result<TrackingImportReport, LineError> {
    let parsed = parse_rows(text, &IMPORT_HEADERS)?;
    let mut rows = Vec::with_capacity(parsed.len());
    for row in parsed {
        let outcome =
            apply_tracking_row(pool, admin_id, row.line, row.get(0), row.get(1), row.get(2)).await;
        rows.push(outcome);
    }
    Ok(TrackingImportReport { rows })
}

/// Maps a posted carrier cell (code or Japanese label) to the
/// catalogue code.
fn carrier_code_for_cell(cell: &str) -> Option<&'static str> {
    match cell.trim() {
        "yamato" | "ヤマト運輸" => Some("yamato"),
        "sagawa" | "佐川急便" => Some("sagawa"),
        "japan_post" | "日本郵便" | "日本郵便（ゆうパック）" | "ゆうパック" => {
            Some("japan_post")
        }
        "other" | "その他" => Some("other"),
        _ => None,
    }
}

/// Applies one import row. Every refusal is a message, never
/// an error: the row set is the unit of reporting.
async fn apply_tracking_row(
    pool: &PgPool,
    admin_id: i64,
    line: usize,
    number_cell: &str,
    carrier_cell: &str,
    tracking_cell: &str,
) -> TrackingRowOutcome {
    let number = number_cell.trim().to_string();
    let fail = |message: String| TrackingRowOutcome {
        line,
        number: number.clone(),
        applied: false,
        message,
    };
    // The order number's tail is the order ID; the full
    // number is then re-derived from the stored order and
    // compared, so a typo in the date part cannot ship the
    // wrong order.
    let Some(id) = number
        .rsplit('-')
        .next()
        .and_then(|tail| tail.trim().parse::<i64>().ok())
    else {
        return fail("注文番号の形式が正しくありません".to_string());
    };
    let order = match order_repository::find_by_id(pool, id).await {
        Ok(Some(order)) => order,
        Ok(None) => return fail(ORDER_NOT_FOUND_MESSAGE.to_string()),
        Err(_) => return fail(SHIPPING_CSV_LOAD_FAILED_MESSAGE.to_string()),
    };
    if order.order_number() != number {
        return fail(ORDER_NOT_FOUND_MESSAGE.to_string());
    }
    let Some(carrier_code) = carrier_code_for_cell(carrier_cell) else {
        return fail(CARRIER_INVALID_MESSAGE.to_string());
    };
    if let Err(e) = crate::models::delivery::validate_shipping_details(carrier_code, tracking_cell)
    {
        return fail(e.message().to_string());
    }
    let tracking = tracking_cell.trim().to_string();
    // C5: an order with an open parcel takes the row on
    // that parcel — the split flow's ship path, with its
    // own guard and per-parcel mail.
    let open = match crate::repositories::admin::shipment_repository::open_shipment_for_order(
        pool, order.id,
    )
    .await
    {
        Ok(open) => open,
        Err(_) => return fail("出荷情報の確認に失敗しました".to_string()),
    };
    if let Some(open) = open {
        return match crate::services::admin::shipment_service::ship_shipment(
            pool,
            admin_id,
            order.id,
            open.id,
            carrier_code,
            &tracking,
        )
        .await
        {
            Ok(completed) => TrackingRowOutcome {
                line,
                number: number.clone(),
                applied: true,
                message: if completed {
                    "分割出荷を発送しました（注文は発送済みになりました）".to_string()
                } else {
                    "分割出荷を発送しました".to_string()
                },
            },
            Err(e) => fail(match &e {
                AppError::Conflict(m) => m.clone(),
                _ => "発送に失敗しました".to_string(),
            }),
        };
    }
    match order.status.as_str() {
        "received" | "preparing" => {
            // The payment guard is checked before anything
            // moves: an unsettled hosted order keeps its exact
            // state and the row reports the one reason.
            let payment = match payment_repository::find_by_order_id(pool, order.id).await {
                Ok(p) => p,
                Err(_) => return fail(SHIPPING_CSV_LOAD_FAILED_MESSAGE.to_string()),
            };
            if payment
                .as_ref()
                .is_some_and(|p| p.is_hosted() && !p.is_paid())
            {
                return fail(SHIPMENT_BLOCKED_UNPAID_MESSAGE.to_string());
            }
            // A received order walks the flow's two steps
            // (preparing, then shipped) as two audited
            // transitions — the same moves the operator would
            // make from the detail page, never a jump the flow
            // forbids.
            if order.status == "received" {
                match order_service::change_status_queued_mail(
                    pool,
                    admin_id,
                    order.id,
                    "preparing",
                    Some("received"),
                    "",
                    "",
                )
                .await
                {
                    Ok(()) => {}
                    Err(StatusChangeError::Stale) | Err(StatusChangeError::IllegalTransition) => {
                        return fail(STATUS_ALREADY_CHANGED_MESSAGE.to_string());
                    }
                    Err(_) => return fail(SHIPPING_CSV_UPDATE_FAILED_MESSAGE.to_string()),
                }
            }
            match order_service::change_status_queued_mail(
                pool,
                admin_id,
                order.id,
                "shipped",
                Some("preparing"),
                carrier_code,
                &tracking,
            )
            .await
            {
                Ok(()) => TrackingRowOutcome {
                    line,
                    number,
                    applied: true,
                    message: "発送済みにしました（発送メールは順次送信されます）".to_string(),
                },
                Err(StatusChangeError::PaymentNotSettled) => {
                    fail(SHIPMENT_BLOCKED_UNPAID_MESSAGE.to_string())
                }
                Err(StatusChangeError::Stale) | Err(StatusChangeError::IllegalTransition) => {
                    fail(STATUS_CHANGE_NOT_ALLOWED_MESSAGE.to_string())
                }
                Err(StatusChangeError::InvalidShipping(message)) => fail(message),
                Err(_) => fail(SHIPPING_CSV_UPDATE_FAILED_MESSAGE.to_string()),
            }
        }
        "shipped" | "delivered" if order.tracking_number.is_none() => {
            // The number alone is missing (the order shipped
            // through a path that had none): record it, with
            // its own audit action — no status moves and no
            // second mail.
            let recorded =
                record_tracking_only(pool, admin_id, order.id, carrier_code, &tracking).await;
            match recorded {
                Ok(()) => TrackingRowOutcome {
                    line,
                    number,
                    applied: true,
                    message: "伝票番号を登録しました".to_string(),
                },
                Err(_) => fail("登録に失敗しました".to_string()),
            }
        }
        "shipped" | "delivered" => fail("すでに伝票番号が登録されています".to_string()),
        _ => fail("キャンセル済みの注文です".to_string()),
    }
}

/// Records a tracking number on an already-shipped order, in
/// one transaction with its audit row (`order.shipping_details`).
async fn record_tracking_only(
    pool: &PgPool,
    admin_id: i64,
    order_id: i64,
    carrier_code: &str,
    tracking: &str,
) -> Result<(), AppError> {
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| AppError::db("begin tracking record", e))?;
    order_repository::set_shipping_details(&mut tx, order_id, Some(carrier_code), Some(tracking))
        .await
        .map_err(|e| AppError::db("record tracking number", e))?;
    crate::services::admin::audit_service::record(
        &mut tx,
        admin_id,
        "order.shipping_details",
        "order",
        Some(order_id),
    )
    .await?;
    tx.commit()
        .await
        .map_err(|e| AppError::db("commit tracking record", e))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slot_codes_follow_each_carriers_bands() {
        assert_eq!(yamato_slot_code("am"), "0812");
        assert_eq!(yamato_slot_code("t1921"), "1921");
        assert_eq!(yamato_slot_code("none"), "");
        assert_eq!(sagawa_slot_code("am"), "01");
        assert_eq!(sagawa_slot_code("t1618"), "16");
        assert_eq!(japan_post_slot_label("t1820"), "18時-20時");
        assert_eq!(japan_post_slot_label("none"), "希望なし");
    }

    #[test]
    fn carrier_cells_accept_codes_and_labels() {
        assert_eq!(carrier_code_for_cell("yamato"), Some("yamato"));
        assert_eq!(carrier_code_for_cell(" 佐川急便 "), Some("sagawa"));
        assert_eq!(carrier_code_for_cell("ゆうパック"), Some("japan_post"));
        assert_eq!(carrier_code_for_cell("dhl"), None);
    }
}
