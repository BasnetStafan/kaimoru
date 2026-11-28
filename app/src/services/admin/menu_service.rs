//! Admin menu rules (CMS spec C-4).
//!
//! The operator edits the layout's two link lists — the
//! header menu and the footer menu — in one form. The save
//! validates every row, then replaces each *changed*
//! menu's rows in a single audited transaction; a submit
//! that changes nothing writes nothing (the settings
//! rule). Labels and URLs never reach a log line — only
//! the action name does.

use sqlx::PgPool;

use crate::error::AppError;
use crate::messages::{LINK_URL_MESSAGE, SORT_ORDER_NUMERIC_MESSAGE, label_max_length_message};

use crate::models::{FOOTER_KEY, HEADER_KEY, MENU_KEYS, MenuItem};
use crate::repositories::menu_repository;
use crate::services::menu_service::Menus;
use crate::services::{admin::audit_service, menu_service};
use crate::validation::link_url_allowed;

/// Link label limit, in characters.
const LABEL_MAX_CHARS: usize = 30;

/// Blank rows the editor renders under each menu. One row
/// is the no-script fallback (a filled blank appends, an
/// untouched one is ignored); the editor's リンクを追加
/// button clones more rows client-side under the same field
/// contract, so no save behaviour depends on the script.
pub const BLANK_ROWS: usize = 1;

/// One menu row as submitted: every value a string, exactly
/// as typed, except the switches. A row whose id is empty
/// is a new row; a fully blank new row (no label, no URL)
/// is the editor's add slot, not a row at all, and the
/// save ignores it.
#[derive(Clone, Default)]
pub struct MenuItemInput {
    /// Stored row id; empty for a new row.
    pub id: String,
    /// Link text as typed.
    pub label: String,
    /// Link URL as typed.
    pub url: String,
    /// Visibility switch.
    pub visible: bool,
    /// Sort position as typed.
    pub sort_order: String,
    /// Delete switch: the save drops the row.
    pub delete: bool,
}

/// The raw menu form as submitted: each menu's rows in
/// submitted order.
#[derive(Clone, Default)]
pub struct MenusInput {
    /// The header menu's rows.
    pub header: Vec<MenuItemInput>,
    /// The footer menu's rows.
    pub footer: Vec<MenuItemInput>,
}

impl MenusInput {
    /// Returns one menu's rows by its key.
    pub fn rows(&self, menu_key: &str) -> &[MenuItemInput] {
        match menu_key {
            HEADER_KEY => &self.header,
            FOOTER_KEY => &self.footer,
            _ => &[],
        }
    }
}

impl From<&Menus> for MenusInput {
    /// The editor's initial values: the stored rows, then
    /// [`BLANK_ROWS`] empty add rows under each menu. An add
    /// row starts visible with the next sort position
    /// filled in — a link the operator bothers to type is
    /// meant to appear, after the ones already there.
    fn from(menus: &Menus) -> Self {
        fn rows(items: &[MenuItem]) -> Vec<MenuItemInput> {
            items
                .iter()
                .map(|item| MenuItemInput {
                    id: item.id.to_string(),
                    label: item.label.clone(),
                    url: item.url.clone(),
                    visible: item.is_visible,
                    sort_order: item.sort_order.to_string(),
                    delete: false,
                })
                .chain((0..BLANK_ROWS).map(|blank| MenuItemInput {
                    sort_order: ((items.len() + blank + 1) * 10).to_string(),
                    visible: true,
                    ..MenuItemInput::default()
                }))
                .collect()
        }
        Self {
            header: rows(&menus.header),
            footer: rows(&menus.footer),
        }
    }
}

/// The menus after validation, ready to store: each menu's
/// kept rows in submitted order. New rows carry id 0 — the
/// save replaces a changed menu's rows wholesale, and
/// nothing references menu item ids, so identity carries
/// no meaning across a save.
pub struct ValidatedMenus {
    /// The header menu's kept rows.
    pub header: Vec<MenuItem>,
    /// The footer menu's kept rows.
    pub footer: Vec<MenuItem>,
}

impl ValidatedMenus {
    /// Returns one menu's rows by its key.
    pub fn rows(&self, menu_key: &str) -> &[MenuItem] {
        match menu_key {
            HEADER_KEY => &self.header,
            FOOTER_KEY => &self.footer,
            _ => &[],
        }
    }
}

/// One submitted row's validation messages, by part; a part
/// is `None` when it passed.
#[derive(Clone, Default)]
pub struct MenuRowErrors {
    /// Label message.
    pub label: Option<String>,
    /// URL message.
    pub url: Option<String>,
    /// Sort position message.
    pub sort: Option<String>,
}

/// The menu form's validation failure: one message per
/// failing field, addressed by position — the messages sit
/// parallel to the submitted rows, plus one message per
/// menu as a whole. (`FieldError`'s static field names
/// cannot name dynamic rows; the editor's rows are the
/// address space here, so the shape mirrors them.)
#[derive(Default)]
pub struct MenuErrors {
    /// The header menu's whole-menu message (the
    /// last-visible-item guard, a broken row id).
    pub header_menu: Option<String>,
    /// The footer menu's whole-menu message.
    pub footer_menu: Option<String>,
    /// Per-row messages for the header, parallel to the
    /// submitted header rows.
    pub header_rows: Vec<MenuRowErrors>,
    /// Per-row messages for the footer, parallel to the
    /// submitted footer rows.
    pub footer_rows: Vec<MenuRowErrors>,
}

impl MenuErrors {
    /// Whether no field failed: validation passed.
    fn is_empty(&self) -> bool {
        self.header_menu.is_none()
            && self.footer_menu.is_none()
            && self
                .header_rows
                .iter()
                .chain(self.footer_rows.iter())
                .all(|row| row.label.is_none() && row.url.is_none() && row.sort.is_none())
    }
}

/// Whether a submitted row is an untouched add slot: a new
/// row with neither label nor URL. The pre-filled sort
/// position does not make it a row.
fn is_blank(row: &MenuItemInput) -> bool {
    row.id.trim().is_empty() && row.label.trim().is_empty() && row.url.trim().is_empty()
}

/// Validates the menu form against the stored menus,
/// reporting every failing field at once. Values are
/// trimmed before they are checked and stored.
pub fn validate_menus(input: &MenusInput, current: &Menus) -> Result<ValidatedMenus, MenuErrors> {
    let mut errors = MenuErrors::default();
    let header = validate_menu(
        HEADER_KEY,
        input.rows(HEADER_KEY),
        current.items(HEADER_KEY),
        &mut errors.header_menu,
        &mut errors.header_rows,
    );
    let footer = validate_menu(
        FOOTER_KEY,
        input.rows(FOOTER_KEY),
        current.items(FOOTER_KEY),
        &mut errors.footer_menu,
        &mut errors.footer_rows,
    );
    if !errors.is_empty() {
        return Err(errors);
    }
    Ok(ValidatedMenus { header, footer })
}

/// Validates one menu's submitted rows against its stored
/// rows, filling the row-parallel messages and the
/// whole-menu message, and returns the kept rows (deleted
/// and blank rows dropped) in submitted order. Rows that
/// failed a check are still returned — the save is refused
/// on any error, so they never reach the database.
fn validate_menu(
    menu_key: &str,
    rows: &[MenuItemInput],
    stored: &[MenuItem],
    menu_error: &mut Option<String>,
    row_errors: &mut Vec<MenuRowErrors>,
) -> Vec<MenuItem> {
    let mut kept: Vec<MenuItem> = Vec::new();
    for (index, row) in rows.iter().enumerate() {
        row_errors.push(MenuRowErrors::default());
        if row.delete || is_blank(row) {
            continue;
        }
        let mut messages = MenuRowErrors::default();
        // A stored row's id must name a row of this menu: the
        // form renders the ids, so an unknown one means the
        // submit is stale or tampered with — refuse the menu
        // as a whole rather than guessing which row it meant.
        let id = if row.id.trim().is_empty() {
            0
        } else {
            match row.id.trim().parse::<i64>() {
                Ok(id) if stored.iter().any(|item| item.id == id) => id,
                _ => {
                    if menu_error.is_none() {
                        *menu_error = Some(
                            "送信されたデータが正しくありません。ページを開き直してから保存してください。"
                                .to_string(),
                        );
                    }
                    continue;
                }
            }
        };
        let label = row.label.trim().to_string();
        if label.is_empty() {
            messages.label = Some("ラベルを入力してください。".to_string());
        } else if label.chars().count() > LABEL_MAX_CHARS {
            messages.label = Some(label_max_length_message(LABEL_MAX_CHARS));
        }
        let url = row.url.trim().to_string();
        if url.is_empty() {
            messages.url = Some("URLを入力してください。".to_string());
        } else if !link_url_allowed(&url) {
            messages.url = Some(LINK_URL_MESSAGE.to_string());
        }
        let sort_order = match row.sort_order.trim().parse::<i32>() {
            Ok(value) => value,
            Err(_) => {
                messages.sort = Some(SORT_ORDER_NUMERIC_MESSAGE.to_string());
                0
            }
        };
        row_errors[index] = messages;
        kept.push(MenuItem {
            id,
            menu_key: menu_key.to_string(),
            label,
            url,
            is_visible: row.visible,
            sort_order,
        });
    }
    // The last-visible-item guard covers the
    // footer only: the footer is the store's guaranteed
    // navigation, so it must never drop from some visible
    // links to none by one misclick. The header menu is
    // optional chrome — it renders only while it has a
    // visible link and was seeded empty — so a save that
    // leaves it with zero is legitimate there.
    let stored_visible = stored.iter().filter(|item| item.is_visible).count();
    let kept_visible = kept.iter().filter(|item| item.is_visible).count();
    if menu_key == FOOTER_KEY && stored_visible >= 1 && kept_visible == 0 && menu_error.is_none() {
        *menu_error = Some(
            "表示するリンクが1件もなくなります。最低1件は表示のままにしてください。".to_string(),
        );
    }
    kept
}

/// Returns the stored menus for the editor, read fresh: an
/// admin must see the stored truth, never the storefront's
/// brief cache.
pub async fn get_menus(pool: &PgPool) -> Result<Menus, AppError> {
    let items = menu_repository::list_all(pool).await?;
    Ok(menu_service::assemble(items))
}

/// Whether two item lists carry the same content in the
/// same order (a save replaces a menu wholesale, so order
/// is content).
fn items_eq(a: &[MenuItem], b: &[MenuItem]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b.iter()).all(|(stored, next)| {
            stored.id == next.id
                && stored.label == next.label
                && stored.url == next.url
                && stored.is_visible == next.is_visible
                && stored.sort_order == next.sort_order
        })
}

/// Stores validated menus and audits each changed menu in
/// the same transaction, then drops the storefront cache
/// so the new links are live on the next page. A submit
/// that changes nothing writes nothing.
/// The form-ticket stamp for the menus editor: a
/// fingerprint of the full item set the form was rendered
/// from (a save replaces each changed menu wholesale, so
/// the set is the record). Unlike the single-record
/// guards, the comparison runs on the read at the top of
/// `update_menus`, before its transaction — it narrows
/// the race to the save's own duration rather than
/// eliminating it, proportionate for a whole-set replace
/// (locking every menu row to compare would be the only
/// exact form).
pub fn fingerprint(menus: &Menus) -> i64 {
    let mut stamp = crate::services::form_ticket_service::SetStamp::new();
    for key in MENU_KEYS {
        stamp.text(key);
        for item in menus.items(key) {
            stamp.int(item.id);
            stamp.text(&item.label);
            stamp.text(&item.url);
            stamp.flag(item.is_visible);
            stamp.int(i64::from(item.sort_order));
        }
    }
    stamp.finish()
}

pub async fn update_menus(
    pool: &PgPool,
    admin_id: i64,
    validated: &ValidatedMenus,
    expected: i64,
) -> Result<(), AppError> {
    let current = get_menus(pool).await?;
    if fingerprint(&current) != expected {
        return Err(crate::services::form_ticket_service::stale_conflict());
    }
    let changed: Vec<&str> = MENU_KEYS
        .iter()
        .copied()
        .filter(|key| !items_eq(current.items(key), validated.rows(key)))
        .collect();
    if changed.is_empty() {
        return Ok(());
    }
    let mut tx = pool.begin().await.map_err(|e| AppError::db("begin", e))?;
    for key in &changed {
        menu_repository::delete_menu(&mut tx, key).await?;
        for item in validated.rows(key) {
            menu_repository::insert_item(&mut tx, item).await?;
        }
        audit_service::record(&mut tx, admin_id, "menu.update", "menu", None).await?;
    }
    tx.commit().await.map_err(|e| AppError::db("commit", e))?;
    // Storefront reads go through the menu cache; drop it so
    // the save is live on the next page.
    menu_service::invalidate_cache();
    tracing::info!(admin_id, "menu.update: menus changed");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::fingerprint;
    use crate::models::{FOOTER_KEY, MenuItem};
    use crate::services::menu_service::Menus;

    /// One footer item with the given label.
    fn item(label: &str) -> MenuItem {
        MenuItem {
            id: 1,
            menu_key: FOOTER_KEY.to_string(),
            label: label.to_string(),
            url: "/guide".to_string(),
            is_visible: true,
            sort_order: 0,
        }
    }

    #[test]
    fn menu_fingerprint_is_stable_and_reacts_to_edits() {
        let menus = Menus {
            header: Vec::new(),
            footer: vec![item("ご利用ガイド")],
        };
        assert_eq!(fingerprint(&menus), fingerprint(&menus));

        let relabelled = Menus {
            header: Vec::new(),
            footer: vec![item("送料について")],
        };
        assert_ne!(fingerprint(&menus), fingerprint(&relabelled));

        let mut hidden = Menus {
            header: Vec::new(),
            footer: vec![item("ご利用ガイド")],
        };
        hidden.footer[0].is_visible = false;
        assert_ne!(fingerprint(&menus), fingerprint(&hidden));
    }
}
