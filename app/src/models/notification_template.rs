//! Notification (transactional) mail templates.
//!
//! The catalog of the fixed system mails an operator may
//! reword from the mail template menu, each with its code
//! default (subject and body, rendering byte-identically to
//! the hand-built mails they replace), its closed tag set,
//! and the structural tags a saved body may not drop.
//! Overrides live in the `notification_templates` table; a
//! mail with no row renders from the default here.

use crate::messages::{
    MAIL_NAME_EMAIL_CHANGE_CODE, MAIL_NAME_INQUIRY_ACKNOWLEDGEMENT, MAIL_NAME_KONBINI_INSTRUCTIONS,
    MAIL_NAME_ORDER_CANCELLATION, MAIL_NAME_ORDER_CONFIRMATION, MAIL_NAME_PASSWORD_RESET_CODE,
    MAIL_NAME_PAYMENT_CONFIRMATION, MAIL_NAME_SHIPPING_NOTIFICATION, MAIL_NAME_VERIFICATION_CODE,
    MAIL_NAME_WITHDRAWAL_CODE, MAIL_NAME_WITHDRAWAL_COMPLETED,
};
use std::collections::HashMap;

/// One stored override row from `notification_templates`.
#[derive(Clone, sqlx::FromRow)]
pub struct NotificationTemplate {
    /// The mail's stable key (a `mail.rs` sender name).
    pub mail_key: String,
    /// The operator's subject template.
    pub subject: String,
    /// The operator's body template.
    pub body: String,
    /// When the override was last saved.
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// One usable `{tag}` in a template: its name, what it
/// inserts, and whether it is a system-generated block
/// (inserted whole) rather than a single value.
pub struct TagDef {
    /// Tag name inside the braces, e.g. `order_number`.
    pub name: &'static str,
    /// One-line Japanese description for the editor.
    pub label: &'static str,
    /// Whether the tag expands to a generated section.
    pub block: bool,
}

/// A named group of tags, shown together in the editor.
pub struct TagGroup {
    /// Group heading shown in the editor.
    pub label: &'static str,
    /// The group's tags.
    pub tags: &'static [TagDef],
}

/// The shop identity tags, from the shop settings.
pub const SHOP_TAGS: &[TagDef] = &[
    TagDef {
        name: "shop_name",
        label: "店舗名",
        block: false,
    },
    TagDef {
        name: "signature",
        label: "署名（未設定のときは店舗名）",
        block: false,
    },
    TagDef {
        name: "contact_email",
        label: "問い合わせメールアドレス",
        block: false,
    },
    TagDef {
        name: "shop_url",
        label: "店舗URL（未設定のときは空）",
        block: false,
    },
];

/// The recipient tags: the person the mail is about.
pub const RECIPIENT_TAGS: &[TagDef] = &[
    TagDef {
        name: "customer_name",
        label: "お客様の氏名",
        block: false,
    },
    TagDef {
        name: "customer_name_kana",
        label: "お客様のフリガナ（注文メールでは空）",
        block: false,
    },
    TagDef {
        name: "customer_email",
        label: "宛先メールアドレス",
        block: false,
    },
    TagDef {
        name: "customer_phone",
        label: "お客様の電話番号",
        block: false,
    },
];

/// The order tags, from the order record.
pub const ORDER_TAGS: &[TagDef] = &[
    TagDef {
        name: "order_number",
        label: "注文番号",
        block: false,
    },
    TagDef {
        name: "order_date",
        label: "注文日時",
        block: false,
    },
    TagDef {
        name: "order_status",
        label: "注文ステータス",
        block: false,
    },
    TagDef {
        name: "item_count",
        label: "商品点数",
        block: false,
    },
    TagDef {
        name: "subtotal",
        label: "小計",
        block: false,
    },
    TagDef {
        name: "discount",
        label: "クーポン割引額（0円のときは空）",
        block: false,
    },
    TagDef {
        name: "shipping_fee",
        label: "送料",
        block: false,
    },
    TagDef {
        name: "cod_fee",
        label: "代引き手数料（0円のときは空）",
        block: false,
    },
    TagDef {
        name: "total",
        label: "合計金額",
        block: false,
    },
    TagDef {
        name: "payment_method",
        label: "お支払い方法",
        block: false,
    },
    TagDef {
        name: "coupon_code",
        label: "利用クーポンコード（ないときは空）",
        block: false,
    },
];

/// The delivery tags, from the order's delivery snapshot.
pub const DELIVERY_TAGS: &[TagDef] = &[
    TagDef {
        name: "delivery_name",
        label: "お届け先氏名",
        block: false,
    },
    TagDef {
        name: "delivery_postal",
        label: "お届け先の郵便番号",
        block: false,
    },
    TagDef {
        name: "delivery_address",
        label: "お届け先住所",
        block: false,
    },
    TagDef {
        name: "delivery_phone",
        label: "お届け先の電話番号",
        block: false,
    },
    TagDef {
        name: "delivery_date",
        label: "お届け希望日（ないときは空）",
        block: false,
    },
    TagDef {
        name: "delivery_slot",
        label: "お届け時間帯",
        block: false,
    },
    TagDef {
        name: "carrier",
        label: "配送会社（未指定のときは空）",
        block: false,
    },
    TagDef {
        name: "tracking_number",
        label: "伝票番号（ないときは空）",
        block: false,
    },
    TagDef {
        name: "tracking_url",
        label: "追跡URL（ないときは空）",
        block: false,
    },
];

/// The generated blocks shared by the three order mails.
pub const ORDER_BLOCK_TAGS: &[TagDef] = &[
    TagDef {
        name: "items_block",
        label: "商品明細（自動生成）",
        block: true,
    },
    TagDef {
        name: "tax_block",
        label: "税率ごとの内訳（自動生成）",
        block: true,
    },
    TagDef {
        name: "delivery_block",
        label: "お届け希望日時（自動生成）",
        block: true,
    },
    TagDef {
        name: "gift_block",
        label: "ギフト（のし・メッセージ、自動生成）",
        block: true,
    },
    TagDef {
        name: "address_block",
        label: "お届け先（自動生成）",
        block: true,
    },
];

/// The generated blocks of the shipping notification.
pub const SHIPPING_BLOCK_TAGS: &[TagDef] = &[
    TagDef {
        name: "items_block",
        label: "商品明細（自動生成）",
        block: true,
    },
    TagDef {
        name: "tax_block",
        label: "税率ごとの内訳（自動生成）",
        block: true,
    },
    TagDef {
        name: "address_block",
        label: "お届け先（自動生成）",
        block: true,
    },
    TagDef {
        name: "shipped_date_block",
        label: "発送日（自動生成）",
        block: true,
    },
    TagDef {
        name: "cod_block",
        label: "代金引換の案内（自動生成）",
        block: true,
    },
    TagDef {
        name: "delivery_request_block",
        label: "お届け希望（自動生成）",
        block: true,
    },
    TagDef {
        name: "tracking_block",
        label: "配送情報（自動生成）",
        block: true,
    },
    TagDef {
        name: "contact_block",
        label: "問い合わせ先（自動生成）",
        block: true,
    },
];

const SHOP_GROUP: TagGroup = TagGroup {
    label: "店舗情報",
    tags: SHOP_TAGS,
};
const RECIPIENT_GROUP: TagGroup = TagGroup {
    label: "お客様情報",
    tags: RECIPIENT_TAGS,
};
const ORDER_GROUP: TagGroup = TagGroup {
    label: "注文情報",
    tags: ORDER_TAGS,
};
const DELIVERY_GROUP: TagGroup = TagGroup {
    label: "配送情報",
    tags: DELIVERY_TAGS,
};
const ORDER_BLOCK_GROUP: TagGroup = TagGroup {
    label: "自動生成ブロック",
    tags: ORDER_BLOCK_TAGS,
};
const SHIPPING_BLOCK_GROUP: TagGroup = TagGroup {
    label: "自動生成ブロック",
    tags: SHIPPING_BLOCK_TAGS,
};

/// One catalog entry: a fixed system mail and everything
/// the editor and renderer need to know about it.
pub struct NotificationMailDef {
    /// Stable key, matching the `mail.rs` sender name and
    /// the `notification_templates` primary key.
    pub key: &'static str,
    /// Japanese display name (the dashboard inventory name).
    pub name: &'static str,
    /// Tag groups this mail may use, in editor order.
    pub groups: &'static [TagGroup],
    /// Mail-specific value tags beyond the groups.
    pub specific_tags: &'static [TagDef],
    /// Tags a saved body must still contain: the mail's
    /// structural content an edit may move but not delete.
    pub required: &'static [&'static str],
    /// The code default subject template.
    pub default_subject: &'static str,
    /// The code default body template.
    pub default_body: &'static str,
}

impl NotificationMailDef {
    /// Returns every tag this mail accepts, groups first.
    pub fn allowed_tags(&self) -> Vec<&TagDef> {
        let mut tags: Vec<&TagDef> = Vec::new();
        for group in self.groups {
            tags.extend(group.tags.iter());
        }
        tags.extend(self.specific_tags.iter());
        tags
    }

    /// Whether `name` is one of this mail's tags.
    pub fn allows(&self, name: &str) -> bool {
        self.allowed_tags().iter().any(|t| t.name == name)
    }
}

const CODE_TAGS: &[TagDef] = &[TagDef {
    name: "code",
    label: "認証コード（6桁）",
    block: false,
}];

const CODE_GROUPS: &[TagGroup] = &[SHOP_GROUP, RECIPIENT_GROUP];
const ORDER_GROUPS: &[TagGroup] = &[
    SHOP_GROUP,
    RECIPIENT_GROUP,
    ORDER_GROUP,
    DELIVERY_GROUP,
    ORDER_BLOCK_GROUP,
];
const ORDER_REQUIRED: &[&str] = &[
    "items_block",
    "tax_block",
    "delivery_block",
    "address_block",
];

/// The editable notification mails, in dashboard order.
/// The inquiry reply is absent on purpose: it is free text
/// the operator writes per reply, not a fixed template.
pub const NOTIFICATION_MAILS: &[NotificationMailDef] = &[
    NotificationMailDef {
        key: "verification_code",
        name: MAIL_NAME_VERIFICATION_CODE,
        groups: CODE_GROUPS,
        specific_tags: CODE_TAGS,
        required: &["code"],
        default_subject: "認証コードのお知らせ",
        default_body: concat!(
            "AGENTIC STORE をご利用いただきありがとうございます。\n\n",
            "会員登録の認証コードは次のとおりです。\n\n",
            "認証コード:{code}",
            "\n\nこのコードの有効期限は10分間です。\n心当たりがない場合は、このメールを破棄してください。\n",
        ),
    },
    NotificationMailDef {
        key: "password_reset_code",
        name: MAIL_NAME_PASSWORD_RESET_CODE,
        groups: CODE_GROUPS,
        specific_tags: CODE_TAGS,
        required: &["code"],
        default_subject: MAIL_NAME_PASSWORD_RESET_CODE,
        default_body: concat!(
            "AGENTIC STORE をご利用いただきありがとうございます。\n\n",
            "パスワード再設定の認証コードは次のとおりです。\n\n",
            "認証コード:{code}",
            "\n\nこのコードの有効期限は10分間です。\n心当たりがない場合は、このメールを破棄してください。\n",
        ),
    },
    NotificationMailDef {
        key: "email_change_code",
        name: MAIL_NAME_EMAIL_CHANGE_CODE,
        groups: CODE_GROUPS,
        specific_tags: CODE_TAGS,
        required: &["code"],
        default_subject: MAIL_NAME_EMAIL_CHANGE_CODE,
        default_body: concat!(
            "AGENTIC STORE をご利用いただきありがとうございます。\n\n",
            "メールアドレス変更の認証コードは次のとおりです。\n\n",
            "認証コード:{code}",
            "\n\nこのコードの有効期限は10分間です。\n心当たりがない場合は、このメールを破棄してください。\n",
        ),
    },
    NotificationMailDef {
        key: "withdrawal_code",
        name: MAIL_NAME_WITHDRAWAL_CODE,
        groups: CODE_GROUPS,
        specific_tags: CODE_TAGS,
        required: &["code"],
        default_subject: MAIL_NAME_WITHDRAWAL_CODE,
        default_body: concat!(
            "AGENTIC STORE をご利用いただきありがとうございます。\n\n",
            "退会手続きの認証コードは次のとおりです。\n\n",
            "認証コード:{code}",
            "\n\nこのコードの有効期限は10分間です。\n心当たりがない場合は、このメールを破棄してください。\n",
        ),
    },
    NotificationMailDef {
        key: "order_confirmation",
        name: MAIL_NAME_ORDER_CONFIRMATION,
        groups: ORDER_GROUPS,
        specific_tags: &[],
        required: ORDER_REQUIRED,
        default_subject: "ご注文ありがとうございます",
        default_body: concat!(
            "AGENTIC STORE をご利用いただきありがとうございます。\n",
            "ご注文を受け付けました。",
            "\n\n注文番号:{order_number}\n\n\n{items_block}{tax_block}\n合計:{total}",
            "（お届け時に現金でお支払いください）",
            "\n\n{delivery_block}{gift_block}{address_block}",
        ),
    },
    NotificationMailDef {
        key: "payment_confirmation",
        name: MAIL_NAME_PAYMENT_CONFIRMATION,
        groups: ORDER_GROUPS,
        specific_tags: &[],
        required: ORDER_REQUIRED,
        default_subject: "お支払いが完了しました",
        default_body: concat!(
            "AGENTIC STORE をご利用いただきありがとうございます。\n",
            "お支払いが完了しました。",
            "\n\n注文番号:{order_number}\n\n\n{items_block}{tax_block}\n合計:{total}",
            "（お支払い済み）",
            "\n\n商品の発送準備を進めています。\n{delivery_block}{gift_block}{address_block}",
        ),
    },
    NotificationMailDef {
        key: "konbini_instructions",
        name: MAIL_NAME_KONBINI_INSTRUCTIONS,
        groups: ORDER_GROUPS,
        specific_tags: &[TagDef {
            name: "konbini_block",
            label: "コンビニ支払い情報（自動生成）",
            block: true,
        }],
        required: &[
            "items_block",
            "tax_block",
            "delivery_block",
            "address_block",
            "konbini_block",
        ],
        default_subject: MAIL_NAME_KONBINI_INSTRUCTIONS,
        default_body: concat!(
            "AGENTIC STORE をご利用いただきありがとうございます。\n",
            "コンビニ支払いのご案内です。",
            "\n\n注文番号:{order_number}\n\n\n{items_block}{tax_block}\n合計:{total}",
            "（コンビニでお支払いください）",
            "\n\n{konbini_block}{delivery_block}{gift_block}{address_block}",
        ),
    },
    NotificationMailDef {
        key: "shipping_notification",
        name: MAIL_NAME_SHIPPING_NOTIFICATION,
        groups: &[
            SHOP_GROUP,
            RECIPIENT_GROUP,
            ORDER_GROUP,
            DELIVERY_GROUP,
            SHIPPING_BLOCK_GROUP,
        ],
        specific_tags: &[TagDef {
            name: "shipped_date",
            label: "発送日（再送時は空）",
            block: false,
        }],
        required: &[
            "items_block",
            "tax_block",
            "address_block",
            "shipped_date_block",
            "cod_block",
            "delivery_request_block",
            "tracking_block",
            "contact_block",
        ],
        default_subject: "商品発送のお知らせ（ご注文番号：{order_number}）",
        default_body: concat!(
            "{customer_name} 様\n\n",
            "いつも{shop_name}をご利用いただき、誠にありがとうございます。\n",
            "ご注文いただいた商品を発送いたしましたので、お知らせいたします。\n\n",
            "■ ご注文番号\n{order_number}\n{shipped_date_block}\n",
            "■ ご注文内容\n{items_block}合計：{total}（税込）\n{tax_block}\n",
            "■ お届け先\n{address_block}{cod_block}{delivery_request_block}{tracking_block}\n",
            "商品がお手元に届くまで、今しばらくお待ちくださいますようお願いいたします。\n",
            "お届け内容に相違がある場合や、商品の破損などがございましたら、\n",
            "お手数ですが下記までご連絡ください。\n",
            "{contact_block}──────────────\n{signature}\n{shop_url}\n",
        ),
    },
    NotificationMailDef {
        key: "order_cancellation",
        name: MAIL_NAME_ORDER_CANCELLATION,
        groups: ORDER_GROUPS,
        specific_tags: &[TagDef {
            name: "money_block",
            label: "返金に関する案内（状態により自動選択）",
            block: true,
        }],
        required: &["money_block"],
        default_subject: "注文キャンセルのお知らせ（ご注文番号：{order_number}）",
        default_body: concat!(
            "{shop_name} をご利用いただき、ありがとうございます。",
            "ご注文のキャンセルを受け付けました。\n\n",
            "ご注文番号：{order_number}\n",
            "お支払い合計：{total}\n\n",
            "{money_block}\n\n",
            "ご不明な点がございましたら、お気軽にお問い合わせください。\n",
            "{signature}\n",
        ),
    },
    NotificationMailDef {
        key: "inquiry_acknowledgement",
        name: MAIL_NAME_INQUIRY_ACKNOWLEDGEMENT,
        groups: &[SHOP_GROUP, RECIPIENT_GROUP],
        specific_tags: &[
            TagDef {
                name: "reference",
                label: "受付番号",
                block: false,
            },
            TagDef {
                name: "category",
                label: "お問い合わせ種別",
                block: false,
            },
            TagDef {
                name: "inquiry_body",
                label: "お問い合わせ内容（お客様の入力）",
                block: true,
            },
        ],
        required: &["inquiry_body"],
        default_subject: "お問い合わせを受け付けました（受付番号：{reference}）",
        default_body: concat!(
            "{shop_name}へのお問い合わせありがとうございます。\n",
            "以下の内容で受け付けました。\n\n",
            "■ 受付番号\n{reference}\n\n",
            "■ お問い合わせ種別\n{category}\n\n",
            "■ お問い合わせ内容\n{inquiry_body}\n\n",
            "内容を確認のうえ、通常2営業日以内にメールでご返信いたします。\n\n",
            "— {shop_name}\n",
        ),
    },
    NotificationMailDef {
        key: "restock_notification",
        name: "再入荷のお知らせ",
        groups: &[SHOP_GROUP, RECIPIENT_GROUP],
        specific_tags: &[
            TagDef {
                name: "product_name",
                label: "商品名",
                block: false,
            },
            TagDef {
                name: "variant_label",
                label: "バリエーション",
                block: false,
            },
            TagDef {
                name: "product_url",
                label: "商品ページURL",
                block: false,
            },
        ],
        required: &["product_name", "product_url"],
        default_subject: "再入荷のお知らせ：{product_name}",
        default_body: concat!(
            "{product_name} の再入荷をお知らせします。\n\n",
            "お客様が再入荷通知に登録された商品が入荷しました。\n\n",
            "■ 商品\n{product_name} {variant_label}\n\n",
            "■ 商品ページ\n{product_url}\n\n",
            "在庫には限りがあります。売り切れの節はご容赦ください。\n\n",
            "このメールは、再入荷通知にご登録いただいた方に一度だけお送りしています。\n\n",
            "{signature}\n",
        ),
    },
    NotificationMailDef {
        key: "cart_reminder",
        name: "カートお知らせ",
        groups: &[SHOP_GROUP, RECIPIENT_GROUP],
        specific_tags: &[
            TagDef {
                name: "cart_block",
                label: "カートの商品一覧",
                block: true,
            },
            TagDef {
                name: "cart_url",
                label: "カートページURL",
                block: false,
            },
            TagDef {
                name: "unsubscribe_url",
                label: "配信停止URL",
                block: false,
            },
        ],
        required: &["cart_block", "cart_url"],
        default_subject: "カートに商品が残っています",
        default_body: concat!(
            "いつも{shop_name}をご利用いただきありがとうございます。\n\n",
            "お客様のカートに、以下の商品が残っています。\n\n",
            "{cart_block}\n",
            "■ カートを見る\n{cart_url}\n\n",
            "在庫には限りがあります。売り切れの節はご容赦ください。\n",
            "すでにご注文いただいた場合は、このメールと行き違いですのでご放念ください。\n\n",
            "配信停止はこちら：{unsubscribe_url}\n\n",
            "{signature}\n",
        ),
    },
    NotificationMailDef {
        key: "low_stock_alert",
        name: "在庫わずかアラート（運営者向け）",
        groups: &[SHOP_GROUP],
        specific_tags: &[TagDef {
            name: "products_block",
            label: "在庫わずかの商品一覧",
            block: true,
        }],
        required: &["products_block"],
        default_subject: "在庫わずかのお知らせ（{shop_name}）",
        default_body: concat!(
            "{shop_name} の在庫アラートです。\n\n",
            "以下の商品が在庫わずかになっています。\n\n",
            "{products_block}\n",
            "管理画面の商品編集から在庫を確認してください。\n",
        ),
    },
    NotificationMailDef {
        key: "withdrawal_completed",
        name: MAIL_NAME_WITHDRAWAL_COMPLETED,
        groups: &[SHOP_GROUP, RECIPIENT_GROUP],
        specific_tags: &[TagDef {
            name: "withdrawal_date",
            label: "退会日",
            block: false,
        }],
        required: &[],
        default_subject: "退会手続き完了のお知らせ",
        default_body: concat!(
            "{shop_name} をご利用いただき、ありがとうございました。",
            "退会手続きが完了しました。\n\n",
            "退会日：{withdrawal_date}\n\n",
            "同じメールアドレスで再度ご登録いただくことも可能です。",
            "またのご利用をお待ちしております。\n\n",
            "{signature}\n",
        ),
    },
];

/// Returns the catalog entry for a mail key, when it is an
/// editable notification mail.
pub fn find_def(key: &str) -> Option<&'static NotificationMailDef> {
    NOTIFICATION_MAILS.iter().find(|def| def.key == key)
}

/// The values one send substitutes into a template: tag
/// name → rendered text. Tags a context does not fill
/// render empty.
pub type NotificationContext = HashMap<&'static str, String>;

/// An effective template: the operator's override when one
/// is stored, otherwise the code default.
pub struct EffectiveTemplate {
    /// Subject template.
    pub subject: String,
    /// Body template.
    pub body: String,
    /// Whether an override row produced this template.
    pub customized: bool,
}

impl NotificationMailDef {
    /// Returns this mail's code default as an effective
    /// template.
    pub fn default_template(&self) -> EffectiveTemplate {
        EffectiveTemplate {
            subject: self.default_subject.to_string(),
            body: self.default_body.to_string(),
            customized: false,
        }
    }
}
