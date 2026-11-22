//! Audit log viewer view. Read-only: there is no edit or delete
//! path for audit rows anywhere in the app.

use std::borrow::Cow;

use askama::Template;

use crate::controllers::admin::AdminLayout;
use crate::views::admin::Page;

/// One filter option (target type or action).
pub struct TargetOption {
    /// Filter value (empty = all).
    pub value: &'static str,
    /// Japanese label.
    pub label: String,
    /// Whether it is the active filter.
    pub selected: bool,
}

/// One audit row, display-ready.
pub struct AuditRow {
    /// When it happened, `YYYY/MM/DD HH:mm:ss` JST.
    pub at: String,
    /// The acting admin's display name.
    pub admin_name: String,
    /// Japanese action label.
    pub action_label: String,
    /// Target display: a link when a screen exists for the
    /// target, plain text otherwise. Precomputed `href`.
    pub target_href: Option<String>,
    /// Target text, e.g. `商品 #12`.
    pub target_text: String,
}

/// The audit viewer page.
#[derive(Template)]
#[template(path = "admin/pages/audit/list.html")]
pub struct AuditListView {
    /// Sidebar layout state.
    pub layout: AdminLayout,
    /// Active navigation key.
    pub nav: &'static str,
    /// Target-type filter options.
    pub targets: Vec<TargetOption>,
    /// Action filter options, built from [`ACTION_CODES`].
    pub actions: Vec<TargetOption>,
    /// The 期間 start input's value (`YYYY-MM-DD` or empty).
    pub from_value: String,
    /// The 期間 end input's value (`YYYY-MM-DD` or empty).
    pub to_value: String,
    /// The rows of this page.
    pub logs: Vec<AuditRow>,
    /// Whether any filter is active — selects the empty-state
    /// wording (no rows at all vs none matching).
    pub filtered: bool,
    /// The CSV export URL carrying the current filters: the
    /// export writes exactly the set the viewer is showing.
    pub export_url: String,
    /// Pagination state.
    pub page: Page,
}

/// The action codes the viewer can filter by, in display order:
/// the unified dotted vocabulary. `product.unarchive` and
/// `product.status` are included beyond the spec's table — the
/// writers emit them, and a filter that cannot name them would
/// hide real rows. The select is built from this list, so it can
/// never offer a code the writers no longer write.
pub const ACTION_CODES: [&str; 68] = [
    "admin.login",
    "admin.password",
    "admin.profile",
    "admin.create",
    "admin.update",
    "admin.password_reset",
    "shop_settings.update",
    "order.shipment_create",
    "order.shipment_ship",
    "return.create",
    "return.status",
    "segment.create",
    "segment.delete",
    "shop_holiday.create",
    "shop_holiday.delete",
    "tax_rate.create",
    "content.page.create",
    "content.page.update",
    "content.page.publish",
    "content.page.restore",
    "content.page.delete",
    "home.update",
    "menu.update",
    "product.create",
    "product.update",
    "product.archive",
    "product.unarchive",
    "product.status",
    "product.csv_import",
    "product.image_upload",
    "order.status",
    "order.member_cancel",
    "order.shipping_mail_resend",
    "order.shipping_details",
    "payment.refund_requested",
    "payment.sync",
    "customer.memo",
    "customer.tag",
    "review.delete",
    "review.hide",
    "review.reply",
    "review.unhide",
    "coupon.create",
    "coupon.update",
    "coupon.activate",
    "coupon.deactivate",
    "inquiry.status.update",
    "inquiry.memo.update",
    "inquiry.reply",
    "notice.create",
    "notice.update",
    "notice.delete",
    "category.create",
    "category.update",
    "category.delete",
    "category.csv_import",
    "tag.create",
    "tag.update",
    "tag.delete",
    "tag.csv_import",
    "mail_template.create",
    "mail_template.update",
    "mail_template.delete",
    "notification_template.update",
    "notification_template.reset",
    "mail_campaign.create",
    "mail_campaign.update",
    "mail_campaign.enqueue",
];

/// The Japanese label for an audit action name. Unknown actions
/// (written by a future version) show their raw name rather
/// than vanishing.
pub fn action_label(action: &str) -> Cow<'static, str> {
    match action {
        "admin.login" => Cow::Borrowed("ログイン"),
        "admin.password" => Cow::Borrowed("パスワードを変更"),
        "admin.profile" => Cow::Borrowed("表示名を変更"),
        "admin.create" => Cow::Borrowed("管理者を登録"),
        "admin.update" => Cow::Borrowed("管理者を更新"),
        "admin.password_reset" => Cow::Borrowed("パスワードをリセット"),
        "shop_settings.update" => Cow::Borrowed("店舗設定を更新"),
        "shop_holiday.create" => Cow::Borrowed("休業日を登録"),
        "shop_holiday.delete" => Cow::Borrowed("休業日を削除"),
        "tax_rate.create" => Cow::Borrowed("税率を登録"),
        "content.page.create" => Cow::Borrowed("コンテンツページを登録"),
        "content.page.update" => Cow::Borrowed("コンテンツページを更新"),
        "content.page.publish" => Cow::Borrowed("コンテンツページを公開"),
        "content.page.restore" => Cow::Borrowed("コンテンツページの版を復元"),
        "content.page.delete" => Cow::Borrowed("コンテンツページを削除"),
        "home.update" => Cow::Borrowed("トップページ設定を更新"),
        "menu.update" => Cow::Borrowed("メニュー設定を更新"),
        "product.create" => Cow::Borrowed("商品を登録"),
        "product.update" => Cow::Borrowed("商品を更新"),
        "product.archive" => Cow::Borrowed("商品をアーカイブ"),
        "product.unarchive" => Cow::Borrowed("アーカイブを解除"),
        "product.status" => Cow::Borrowed("商品のステータスを変更"),
        "product.csv_import" => Cow::Borrowed("商品CSV取込"),
        "product.image_upload" => Cow::Borrowed("商品画像をアップロード"),
        "order.status" => Cow::Borrowed("注文ステータスを変更"),
        "order.member_cancel" => Cow::Borrowed("会員によるキャンセル"),
        "order.shipping_mail_resend" => Cow::Borrowed("発送メールを再送信"),
        "order.shipping_details" => Cow::Borrowed("伝票番号を登録"),
        "coupon.create" => Cow::Borrowed("クーポンを登録"),
        "coupon.update" => Cow::Borrowed("クーポンを更新"),
        "coupon.activate" => Cow::Borrowed("クーポンを有効化"),
        "coupon.deactivate" => Cow::Borrowed("クーポンを無効化"),
        "inquiry.status.update" => Cow::Borrowed("お問い合わせの状況を変更"),
        "inquiry.memo.update" => Cow::Borrowed("お問い合わせの管理メモを更新"),
        "inquiry.reply" => Cow::Borrowed("お問い合わせに返信"),
        "payment.refund_requested" => Cow::Borrowed("返金を開始"),
        "payment.sync" => Cow::Borrowed("支払い状況を同期"),
        "customer.memo" => Cow::Borrowed("会員メモを保存"),
        "customer.tag" => Cow::Borrowed("会員タグを変更"),
        "review.delete" => Cow::Borrowed("レビューを削除"),
        "order.shipment_create" => Cow::Borrowed("分割出荷を作成"),
        "order.shipment_ship" => Cow::Borrowed("分割出荷を発送"),
        "return.create" => Cow::Borrowed("返品申請を作成"),
        "return.status" => Cow::Borrowed("返品申請の状態を変更"),
        "segment.create" => Cow::Borrowed("セグメントを作成"),
        "segment.delete" => Cow::Borrowed("セグメントを削除"),
        "review.hide" => Cow::Borrowed("レビューを非表示"),
        "review.reply" => Cow::Borrowed("レビューに店舗返信"),
        "review.unhide" => Cow::Borrowed("レビューの非表示を解除"),
        "notice.create" => Cow::Borrowed("お知らせを登録"),
        "notice.update" => Cow::Borrowed("お知らせを更新"),
        "notice.delete" => Cow::Borrowed("お知らせを削除"),
        "category.create" => Cow::Borrowed("カテゴリーを登録"),
        "category.update" => Cow::Borrowed("カテゴリーを更新"),
        "category.delete" => Cow::Borrowed("カテゴリーを削除"),
        "category.csv_import" => Cow::Borrowed("カテゴリーCSV取込"),
        "tag.create" => Cow::Borrowed("タグを登録"),
        "tag.update" => Cow::Borrowed("タグを更新"),
        "tag.delete" => Cow::Borrowed("タグを削除"),
        "tag.csv_import" => Cow::Borrowed("タグCSV取込"),
        "mail_template.create" => Cow::Borrowed("メールテンプレートを登録"),
        "notification_template.update" => Cow::Borrowed("通知メールを更新"),
        "notification_template.reset" => Cow::Borrowed("通知メールを初期値に戻す"),
        "mail_template.update" => Cow::Borrowed("メールテンプレートを更新"),
        "mail_template.delete" => Cow::Borrowed("メールテンプレートを削除"),
        "mail_campaign.create" => Cow::Borrowed("メールキャンペーンを登録"),
        "mail_campaign.update" => Cow::Borrowed("メールキャンペーンを更新"),
        "mail_campaign.enqueue" => Cow::Borrowed("メールキャンペーンの配信を予約"),
        other => Cow::Owned(other.to_string()),
    }
}

/// The Japanese noun for an audit target type.
pub fn target_label(target_type: &str) -> &'static str {
    match target_type {
        "product" => "商品",
        "product_image" => "商品画像",
        "order" => "注文",
        "coupon" => "クーポン",
        "inquiry" => "お問い合わせ",
        "review" => "レビュー",
        "notice" => "お知らせ",
        "category" => "カテゴリー",
        "tag" => "タグ",
        "mail_template" => "メールテンプレート",
        "notification_template" => "通知メール",
        "mail_campaign" => "メールキャンペーン",
        "admin" => "管理者",
        "shop_settings" => "店舗設定",
        "tax_class" => "消費税区分",
        "content_page" => "コンテンツページ",
        "home_content" => "トップページ設定",
        "menu" => "メニュー設定",
        _ => "対象",
    }
}

/// The admin URL a target links to, when a screen exists for
/// it. Reviews have no detail screen (the row may be deleted
/// anyway), so they stay plain text.
pub fn target_href(target_type: &str, target_id: Option<i64>) -> Option<String> {
    let id = target_id?;
    match target_type {
        "product" => Some(format!("/admin/products/{id}/edit")),
        "order" => Some(format!("/admin/orders/{id}")),
        "notice" => Some(format!("/admin/notices/{id}/edit")),
        "category" => Some(format!("/admin/categories/{id}/edit")),
        "tag" => Some(format!("/admin/tags/{id}/edit")),
        "admin" => Some(format!("/admin/admins/{id}/edit")),
        "shop_settings" => Some("/admin/settings/shop".to_string()),
        "tax_class" => Some("/admin/settings/tax".to_string()),
        "content_page" => Some(format!("/admin/content/pages/{id}")),
        "mail_template" => Some(format!("/admin/mail/templates/{id}/edit")),
        "mail_campaign" => Some(format!("/admin/mail/campaigns/{id}/edit")),
        "home_content" => Some("/admin/content/home".to_string()),
        _ => None,
    }
}
