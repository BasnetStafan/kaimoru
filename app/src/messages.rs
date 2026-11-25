//! Shared user-facing messages: every notice, error and
//! validation text that more than one place shows lives here
//! exactly once, so the same situation always reads the same
//! way and a wording change lands everywhere at once.
//!
//! Plain texts are constants; texts with values in them are
//! small builder functions (a constant cannot serve as a
//! format string). A message used in only one place stays
//! where it is used.

// ---- Links ---------------------------------------------------------------

/// The refusal every link field shows when
/// [`crate::validation::link_url_allowed`] rejects its value;
/// one wording for the home slots, the hero slides and the
/// menus alike.
pub const LINK_URL_MESSAGE: &str = "リンク先は「/」で始まるパス、https://で始まるURL、または「#」で始まるページ内リンクを入力してください。";

// ---- Checkout and orders -------------------------------------------------

/// The notice asking the shopper to pick a delivery address
/// again, when the saved address behind the checkout flow is
/// gone or unreadable: shared by the storefront checkout and
/// the agent settlement, which resume the same flow.
pub const RESELECT_DELIVERY_MESSAGE: &str = "お届け先を選択し直してください。";

/// Builds the shopper-facing sentence for a payment-method
/// limit refusal (today only the konbini ceiling): one
/// builder for the placement error and the agent's selection
/// check, so the wording cannot drift between the step that
/// warns and the step that refuses.
pub fn payment_method_limit_message(method_label: &str, max_yen: i64) -> String {
    format!(
        "{}でのお支払いは合計{}円までです。お支払い方法を変更してください。",
        method_label,
        crate::pricing::display_price(max_yen)
    )
}

// ---- Shopping agent --------------------------------------------------------

/// The notice shown when an agent settlement placement fails
/// for a reason the shopper cannot act on (a coupon or price
/// refusal the settlement should have caught, or a database
/// failure): one wording for every such branch.
pub const ORDER_PROCESSING_FAILED_NOTICE: &str = "注文処理に失敗しました。もう一度お試しください。";

/// The refusal when a form rendered from an older copy of a
/// record is submitted after the record was already updated
/// elsewhere (another tab, window, or staff member): the
/// stale save is refused and the page shows the current
/// values. One wording for every form-ticket guard (member
/// profile, addresses, reviews, and the admin record edits).
pub const STALE_FORM_MESSAGE: &str =
    "この内容は既に更新されています。最新の内容を確認してから保存してください。";

/// The agent-chat reply when a proposal run fails: the same
/// wording on the search and question paths.
pub const EXECUTION_FAILED_NOTICE: &str =
    "実行に失敗しました。しばらく時間をおいてから再度お試しください。";

// ---- Accounts, sign-in and verification ------------------------------------
//
// One wording per situation across the member flows, the admin
// sign-in and the services that validate the same fields.

/// The refusal when an email address is malformed: the same
/// wording in every form and service that validates one.
pub const EMAIL_FORMAT_MESSAGE: &str = "メールアドレスの形式が正しくありません。";

/// The refusal when the email field is empty.
pub const EMAIL_REQUIRED_MESSAGE: &str = "メールアドレスを入力してください。";

/// The refusal when a sign-in fails: it never says which half
/// was wrong.
pub const LOGIN_FAILED_MESSAGE: &str = "メールアドレスまたはパスワードが正しくありません。";

/// The refusal when the password field is empty.
pub const PASSWORD_REQUIRED_MESSAGE: &str = "パスワードを入力してください";

/// The refusal when the current password does not match.
pub const CURRENT_PASSWORD_MISMATCH_MESSAGE: &str = "現在のパスワードが正しくありません。";

/// The refusal when the new password equals the current one.
pub const PASSWORD_SAME_AS_CURRENT_MESSAGE: &str = "新しいパスワードが現在のパスワードと同じです。";

/// The refusal when the password confirmation differs.
pub const PASSWORD_CONFIRM_MISMATCH_MESSAGE: &str = "新しいパスワード(確認)が一致しません。";

/// The error when a password cannot be hashed or verified.
pub const PASSWORD_PROCESSING_FAILED_MESSAGE: &str = "パスワードを処理できませんでした。";

/// The refusal when the address is already registered.
pub const EMAIL_ALREADY_REGISTERED_MESSAGE: &str = "このメールアドレスは登録済みです。";

/// The refusal at sign-up when the address is already
/// registered, pointing at sign-in instead.
pub const EMAIL_ALREADY_REGISTERED_LOGIN_MESSAGE: &str =
    "このメールアドレスは登録済みです。ログインしてください。";

/// The error when account creation fails outright.
pub const ACCOUNT_CREATE_FAILED_MESSAGE: &str = "アカウントを作成できませんでした。";

/// The refusal when the registration session is incomplete.
pub const REGISTRATION_INCOMPLETE_MESSAGE: &str =
    "登録情報が不足しています。はじめからやり直してください。";

/// The notice shown when a verification code is resent.
pub const VERIFICATION_CODE_RESENT_NOTICE: &str = "認証コードを再送しました。";

/// The refusal when a verification code does not match.
pub const VERIFICATION_CODE_MISMATCH_MESSAGE: &str = "認証コードが正しくありません。";

/// Shown when a two-factor proof (login challenge aside)
/// exhausts its attempt budget for this session.
pub const TOTP_PROOF_ATTEMPTS_EXCEEDED_MESSAGE: &str =
    "認証コードの試行回数が上限に達しました。しばらくしてから再度お試しください。";

/// The refusal when a verification code has expired.
pub const VERIFICATION_CODE_EXPIRED_MESSAGE: &str =
    "認証コードの有効期限が切れています。再送してください。";

/// The refusal when the code entry attempts are exhausted.
pub const VERIFICATION_CODE_ATTEMPTS_EXHAUSTED_MESSAGE: &str =
    "入力回数の上限に達しました。コードを再送してください。";

/// The refusal when the resend limit is reached.
pub const VERIFICATION_CODE_RESEND_LIMIT_MESSAGE: &str =
    "再送の上限に達しました。はじめからやり直してください。";

/// The refusal when an action needs the code verified first.
pub const VERIFICATION_CODE_INCOMPLETE_MESSAGE: &str = "先に認証コードの確認を完了してください。";

/// The notice when verification is already complete.
pub const ALREADY_VERIFIED_MESSAGE: &str = "すでに認証が完了しています。";

/// The error when a mail cannot be sent.
pub const MAIL_SEND_FAILED_MESSAGE: &str =
    "メールを送信できませんでした。時間をおいてもう一度お試しください。";

/// The refusal when requests come too fast.
pub const TOO_MANY_REQUESTS_MESSAGE: &str =
    "リクエストが多すぎます。時間をおいてもう一度お試しください。";

/// The generic failure shown when nothing more specific fits.
pub const GENERIC_FAILURE_MESSAGE: &str =
    "処理に失敗しました。時間をおいてもう一度お試しください。";

/// The refusal when the display name is empty.
pub const DISPLAY_NAME_REQUIRED_MESSAGE: &str = "表示名を入力してください。";

/// The refusal when the family name is empty.
pub const FAMILY_NAME_REQUIRED_MESSAGE: &str = "姓を入力してください。";

/// The refusal when the given name is empty.
pub const GIVEN_NAME_REQUIRED_MESSAGE: &str = "名を入力してください。";

/// The refusal when the postal code is not seven digits.
pub const POSTAL_CODE_FORMAT_MESSAGE: &str = "郵便番号は7桁の数字で入力してください。";

/// The refusal when no prefecture is chosen.
pub const PREFECTURE_REQUIRED_MESSAGE: &str = "都道府県を選択してください。";

/// The refusal when the city field is empty.
pub const CITY_REQUIRED_MESSAGE: &str = "市区町村を入力してください。";

/// The refusal when the street address field is empty.
pub const ADDRESS_LINE_REQUIRED_MESSAGE: &str = "番地を入力してください。";

// ---- Parameterized messages --------------------------------------------------
//
// Texts with values in them are builder functions: a constant
// cannot serve as a format string, so the wording lives in
// one function and every caller passes only its values.

/// Builds the refusal when a CSV file exceeds the row limit.
pub fn csv_row_limit_message(max_rows: i64) -> String {
    format!("行数が上限（{max_rows}行）を超えています")
}

/// Builds the flash shown when a CSV import batch is applied.
pub fn csv_import_applied_message(created: usize, updated: usize) -> String {
    format!("CSV取込を適用しました（新規{created}件・更新{updated}件）")
}

/// Builds the refusal when a field exceeds its character cap.
pub fn max_length_message(label: &str, max: usize) -> String {
    format!("{label}は{max}文字以内で入力してください。")
}

/// Builds the refusal when an address or menu label exceeds
/// its cap; the cap itself is a per-feature rule and stays
/// with the caller.
pub fn label_max_length_message(max: usize) -> String {
    format!("ラベルは{max}文字以内で入力してください。")
}

/// Builds the refusal when a verification code is resent
/// before the cooldown ends.
pub fn verification_code_resend_wait_message(seconds: i64) -> String {
    format!("コードの再送は{seconds}秒後から可能です。")
}

/// Builds the refusal when a verification code does not
/// match, with the attempts left.
pub fn verification_code_mismatch_attempts_message(remaining: i32) -> String {
    format!("認証コードが正しくありません。あと{remaining}回まで入力できます。")
}

/// Builds the notice shown when an SNS account is linked.
pub fn sns_account_linked_message(provider_name: &str) -> String {
    format!("{provider_name}アカウントを連携しました。")
}

/// Builds the agent-approval refusal that appends its reason.
pub fn order_failed_message(reason: &str) -> String {
    format!("注文できませんでした。{reason}")
}

/// Builds the paid suffix on the completion page's method
/// label.
pub fn paid_method_label(method_label: &str) -> String {
    format!("（{method_label}・お支払い済み）")
}

// ---- Admin: orders and shipping ---------------------------------------------

/// The refusal when an admin order lookup finds nothing.
pub const ORDER_NOT_FOUND_MESSAGE: &str = "注文が見つかりません";

/// The refusal when an order has no payment row to act on.
pub const PAYMENT_INFO_MISSING_MESSAGE: &str = "支払い情報がありません。";

/// The refusal when a bulk action runs with no order chosen.
pub const NO_ORDERS_SELECTED_MESSAGE: &str = "注文が選択されていません。";

/// The refusal when shipping is attempted before payment is
/// confirmed (the service-layer wording).
pub const SHIPMENT_BLOCKED_UNPAID_MESSAGE: &str = "入金が確認できていないため発送できません";

/// The same refusal on the admin order screens, with its
/// sentence-final period; the two wordings are kept as found.
pub const SHIPMENT_BLOCKED_UNPAID_NOTICE: &str = "入金が確認できていないため発送できません。";

/// The refusal when an order cannot move to the chosen
/// status.
pub const ORDER_STATUS_CHANGE_DENIED_MESSAGE: &str = "この注文では、そのステータスへ変更できません";

/// The refusal when part of a split shipment is shipped
/// whole.
pub const PARTIAL_SHIPMENT_UNIT_MESSAGE: &str =
    "分割出荷中の注文です。出荷単位で発送してください。";

/// The refusal when a bulk status change names an invalid
/// target status.
pub const BULK_STATUS_INVALID_MESSAGE: &str = "一括変更の対象ステータスが正しくありません。";

/// The refusal when the order's status already moved.
pub const STATUS_ALREADY_CHANGED_MESSAGE: &str = "ステータスが既に変更されています";

/// The refusal when the current status allows no change.
pub const STATUS_CHANGE_NOT_ALLOWED_MESSAGE: &str = "現在のステータスからは変更できません";

/// The refusal when the chosen carrier is invalid.
pub const CARRIER_SELECTION_INVALID_MESSAGE: &str = "配送会社の選択が正しくありません。";

/// The refusal when a shipment names an unknown carrier.
pub const CARRIER_INVALID_MESSAGE: &str = "配送会社が正しくありません";

/// The fallback failure on the admin order actions.
pub const ADMIN_PROCESSING_FAILED_MESSAGE: &str = "処理に失敗しました";

// ---- Admin: CSV import -------------------------------------------------------

/// The refusal when a CSV import runs with no file chosen.
pub const CSV_FILE_REQUIRED_MESSAGE: &str = "CSVファイルを選択してください";

/// The refusal when a CSV file exceeds the size limit.
pub const CSV_FILE_TOO_LARGE_MESSAGE: &str = "CSVファイルが上限サイズを超えています";

/// The flash shown when a CSV import preview is discarded.
pub const CSV_IMPORT_DISCARDED_NOTICE: &str = "CSV取込を破棄しました";

/// The failure when a shipping CSV cannot be read.
pub const SHIPPING_CSV_LOAD_FAILED_MESSAGE: &str = "読み込みに失敗しました";

/// The failure when a shipping CSV update fails.
pub const SHIPPING_CSV_UPDATE_FAILED_MESSAGE: &str = "更新に失敗しました";

// ---- SNS and sign-up ----------------------------------------------------------

/// The failure when an SNS link attempt fails.
pub const SNS_LINK_FAILED_MESSAGE: &str =
    "連携に失敗しました。時間をおいてもう一度お試しください。";

/// The failure when an SNS unlink attempt fails.
pub const SNS_UNLINK_FAILED_MESSAGE: &str =
    "解除に失敗しました。時間をおいてもう一度お試しください。";

/// The refusal when the SNS account belongs to another
/// member.
pub const SNS_ACCOUNT_USED_MESSAGE: &str =
    "このSNSアカウントは、すでに別のアカウントに連携されています。";

/// The refusal when the terms agreement is missing at
/// sign-up.
pub const TERMS_AGREEMENT_REQUIRED_MESSAGE: &str =
    "利用規約とプライバシーポリシーへの同意が必要です。";

/// The notice shown when registration completes, naming the
/// welcome coupon it issues.
pub const SIGNUP_COMPLETED_NOTICE: &str =
    "会員登録が完了しました。オープン記念クーポン「WELCOME500」を発行しました。";

// ---- Storefront -----------------------------------------------------------------

/// The refusal when a coupon code cannot be applied.
pub const COUPON_INVALID_MESSAGE: &str =
    "クーポンコードが正しくないか、利用条件を満たしていません。";

/// The title shown for a missing page.
pub const PAGE_NOT_FOUND_MESSAGE: &str = "ページが見つかりませんでした";

/// The title shown for a server error.
pub const SERVER_ERROR_MESSAGE: &str = "サーバーエラーが発生しました";

/// The notice shown when a review is deleted.
pub const REVIEW_DELETED_NOTICE: &str = "レビューを削除しました";

/// The refusal when review posts come too fast (kept
/// without its sentence-final period, as found).
pub const REVIEW_POSTING_BUSY_MESSAGE: &str =
    "投稿が集中しています。しばらく時間をおいてからお試しください";

/// The refusal when an admin inquiry lookup finds nothing.
pub const INQUIRY_NOT_FOUND_MESSAGE: &str = "お問い合わせが見つかりませんでした。";

// ---- Validation in the admin services ---------------------------------------------

/// The refusal when a value must be a non-negative integer.
pub const NON_NEGATIVE_INTEGER_MESSAGE: &str = "0以上の整数で入力してください。";

/// The refusal when a sort order is not numeric.
pub const SORT_ORDER_NUMERIC_MESSAGE: &str = "並び順は数値で入力してください。";

/// The refusal when no tax class is chosen.
pub const TAX_CLASS_REQUIRED_MESSAGE: &str = "消費税区分を選択してください";

/// The refusal when an image upload has no file.
pub const IMAGE_FILE_REQUIRED_MESSAGE: &str = "画像ファイルを選択してください";

/// The failure when an uploaded image cannot be read.
pub const IMAGE_LOAD_FAILED_MESSAGE: &str = "画像の読み込みに失敗しました";

/// The refusal when a variant specification is invalid.
pub const VARIANT_SPEC_INVALID_MESSAGE: &str = "バリエーションの指定が正しくありません";

/// The refusal when a datetime field is malformed (the
/// coupon and home-content wording).
pub const DATETIME_FORMAT_MESSAGE: &str = "日時の形式が正しくありません。";

/// The refusal when a datetime field is malformed (the mail
/// and product wording, without a sentence-final period);
/// the two wordings are kept as found.
pub const DATETIME_FORMAT_MESSAGE_BARE: &str = "日時形式が正しくありません";

/// The refusal when a slug is empty.
pub const SLUG_REQUIRED_MESSAGE: &str = "スラッグを入力してください";

/// The refusal when a slug breaks the character rule.
pub const SLUG_FORMAT_MESSAGE: &str =
    "スラッグは半角英小文字・数字・ハイフンで、50文字以内で入力してください";

/// The refusal when a slug is already registered.
pub const SLUG_TAKEN_MESSAGE: &str = "このスラッグは既に登録されています";

/// The failure when a slug cannot be checked.
pub const SLUG_CHECK_FAILED_MESSAGE: &str = "スラッグを確認できませんでした";

/// The refusal when a description exceeds 500 characters.
pub const DESCRIPTION_MAX_MESSAGE: &str = "説明は500文字以内で入力してください";

/// The failure when a parent category cannot be verified.
pub const PARENT_CATEGORY_CHECK_FAILED_MESSAGE: &str = "親カテゴリーを確認できませんでした";

/// The refusal when a product-scoped coupon has no target
/// product.
pub const COUPON_TARGET_PRODUCT_REQUIRED_MESSAGE: &str = "対象商品を選択してください。";

/// The refusal when a variant-scoped coupon has no target
/// variant.
pub const COUPON_TARGET_VARIANT_REQUIRED_MESSAGE: &str = "対象バリエーションを選択してください。";

/// The refusal when a used coupon's amount is edited.
pub const COUPON_AMOUNT_LOCKED_MESSAGE: &str = "利用済みのため金額は変更できません。";

/// The refusal when a chosen mail segment no longer exists.
pub const SEGMENT_NOT_FOUND_MESSAGE: &str = "選択したセグメントが見つかりません";

/// The refusal when a subject field is empty.
pub const SUBJECT_REQUIRED_MESSAGE: &str = "件名を入力してください。";

/// The refusal when a body field is empty.
pub const BODY_REQUIRED_MESSAGE: &str = "本文を入力してください。";

// ---- Mail templates -------------------------------------------------------------

/// The display name of the sign-up verification-code mail:
/// one name for the template catalog and the admin mail
/// menu alike.
pub const MAIL_NAME_VERIFICATION_CODE: &str = "会員登録の認証コード";

/// The display name of the password-reset code mail.
pub const MAIL_NAME_PASSWORD_RESET_CODE: &str = "パスワード再設定の認証コード";

/// The display name of the email-change code mail.
pub const MAIL_NAME_EMAIL_CHANGE_CODE: &str = "メールアドレス変更の認証コード";

/// The display name of the withdrawal code mail.
pub const MAIL_NAME_WITHDRAWAL_CODE: &str = "退会手続きの認証コード";

/// The display name of the order confirmation mail.
pub const MAIL_NAME_ORDER_CONFIRMATION: &str = "注文確認メール";

/// The display name of the payment confirmation mail.
pub const MAIL_NAME_PAYMENT_CONFIRMATION: &str = "支払い完了のお知らせ";

/// The display name of the konbini instructions mail.
pub const MAIL_NAME_KONBINI_INSTRUCTIONS: &str = "コンビニ支払いのご案内";

/// The display name of the shipping notification mail.
pub const MAIL_NAME_SHIPPING_NOTIFICATION: &str = "商品発送のお知らせ";

/// The display name of the order cancellation mail.
pub const MAIL_NAME_ORDER_CANCELLATION: &str = "注文キャンセルのお知らせ";

/// The display name of the inquiry acknowledgement mail.
pub const MAIL_NAME_INQUIRY_ACKNOWLEDGEMENT: &str = "お問い合わせ受付メール";

/// The display name of the withdrawal-completed mail.
pub const MAIL_NAME_WITHDRAWAL_COMPLETED: &str = "退会完了のお知らせ";

/// The cancellation-mail sentence stating the refund went
/// back to the payment method: shared by the built-in mail
/// and the template defaults.
pub const REFUND_METHOD_NOTE_MESSAGE: &str =
    "お支払い済みの金額は、ご利用のお支払い方法へ返金済みです。";
