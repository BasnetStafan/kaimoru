//! View structs for Kaimoru.
//! Each struct is the typed data one Askama template receives.

pub mod addresses;
pub mod agent;
pub mod agent_settlement;
pub mod blog;
pub mod blog_embed;
pub mod cart;
pub mod category_index;
pub mod checkout;
pub mod contact;
pub mod content_page;
pub mod errors;
pub mod favorites;
pub mod header;
pub mod home;
pub mod login;
pub mod mcp_draft;
pub mod mypage;
pub mod order_lookup;
pub mod password_reset;
pub mod payment;
pub mod points;
pub mod product;
pub mod product_card;
pub mod receipt;
pub mod restock;
pub mod reviews;
pub mod search;
pub mod section;
pub mod signup;
pub mod sns;
pub mod special;
pub mod totp;
pub mod unsubscribe;
pub mod withdrawn;

/// The 47 prefectures, in the standard order used by Japanese forms.
pub const PREFECTURES: [&str; 47] = [
    "北海道",
    "青森県",
    "岩手県",
    "宮城県",
    "秋田県",
    "山形県",
    "福島県",
    "茨城県",
    "栃木県",
    "群馬県",
    "埼玉県",
    "千葉県",
    "東京都",
    "神奈川県",
    "新潟県",
    "富山県",
    "石川県",
    "福井県",
    "山梨県",
    "長野県",
    "岐阜県",
    "静岡県",
    "愛知県",
    "三重県",
    "滋賀県",
    "京都府",
    "大阪府",
    "兵庫県",
    "奈良県",
    "和歌山県",
    "鳥取県",
    "島根県",
    "岡山県",
    "広島県",
    "山口県",
    "徳島県",
    "香川県",
    "愛媛県",
    "高知県",
    "福岡県",
    "佐賀県",
    "長崎県",
    "熊本県",
    "大分県",
    "宮崎県",
    "鹿児島県",
    "沖縄県",
];

/// Masks an email for display: first two local characters visible.
/// Example: ta***@example.com. Used by the sign-up and email
/// change flows to show where a code was sent without printing
/// the full address on the page.
pub fn mask_email(email: &str) -> String {
    match email.split_once('@') {
        Some((local, domain)) => {
            let head: String = local.chars().take(2).collect();
            format!("{head}***@{domain}")
        }
        None => "***".to_string(),
    }
}
