-- Seed and reference data (baseline).
--
-- The rows a fresh install of the old migration chain
-- produced: catalogue seeds (8 physical products with their
-- variants and images), the WELCOME500 coupon definition,
-- tax classes and rates, shop settings, categories, tags,
-- menu items, home sections/slots, content pages and
-- notices. Extracted from a pristine old-chain database, so
-- the values are exactly what the old seeds created.
-- NOT included: the bootstrap admin (created at boot from
-- ADMIN_EMAIL/ADMIN_PASSWORD) and the session tables.

INSERT INTO public.categories (id, name, slug, description, parent_id, display_order, is_active, created_at, updated_at) VALUES
  ('1', 'アパレル', 'apparel', '', NULL, '10', 't', '2026-11-07 12:48:01.204173+00', '2026-11-07 12:48:01.204173+00'),
  ('2', 'イベント', 'event', '', NULL, '20', 't', '2026-11-07 12:48:01.204173+00', '2026-11-07 12:48:01.204173+00'),
  ('3', 'グッズ', 'goods', '', NULL, '30', 't', '2026-11-07 12:48:01.204173+00', '2026-11-07 12:48:01.204173+00');

INSERT INTO public.content_pages (id, slug, kind, title, body_md, meta_description, status, created_at, updated_at, format) OVERRIDING SYSTEM VALUE VALUES
  ('1', 'guide', 'system', '送料・お支払いについて', '
## 送料

全国一律 550円(税込)です。商品合計が5,000円(税込)以上のご注文は送料無料です。

## お支払い方法

お支払い方法は、クレジットカード、コンビニ決済、PayPay、代金引換(代引き)からお選びいただけます。

クレジットカード:ご注文手続きの途中に表示される決済ページでお支払いください。カード情報は決済代行会社が安全に処理し、当店のサーバーに保存されることはありません。

コンビニ決済:ご注文完了後に表示されるお支払い情報を使い、表示される期限内にコンビニエンスストア(セブン-イレブン、ローソン、ファミリーマート、ミニストップ、セイコーマート、デイリーヤマザキ)の店頭でお支払いください。お支払いの際にコンビニ決済手数料が別途かかります。期限内にお支払いがない場合、ご注文は自動的にキャンセルされます。

PayPay:ご注文手続きの途中にPayPayの画面へ進み、お支払いください。

代金引換(代引き):商品お届け時に、配達員へ現金でお支払いください。代引き手数料として1注文につき330円(税込)を申し受けます。

## 配送について

ご注文確定後、通常2〜5営業日でお届けします。コンビニ決済のご注文は、ご入金の確認後の発送となります。イベント受取商品は、商品ページに記載の受取方法をご確認ください。

ご注文手続きでお届け希望日時(ご注文日から2日後〜14日後の日付と時間帯)をご指定いただけますが、配送希望日時は目安のため、交通事情や天候などによりご希望に添えない場合があります。

## 返品・交換について

商品の不良・誤配送の場合は、到着後7日以内にマイページの注文履歴からお問い合わせください。お客様都合による返品は、未開封の商品に限り承ります。返金はご利用のお支払い方法に応じた方法で行います。クレジットカード・PayPayの場合は決済を通じて返金し、コンビニ決済の場合は銀行振込で返金します(手続きのご案内をメールでお送りします)。

## お問い合わせ

お問い合わせはこちらの[フォーム](/contact)からお願いいたします。
', '', 'published', '2026-11-07 12:48:01.224468+00', '2026-11-07 12:48:01.224468+00', 'markdown'),
  ('2', 'terms', 'system', '利用規約', '
この利用規約(以下「本規約」)は、Agentic Store(以下「当店」)が提供するオンラインストアの利用条件を定めるものです。会員登録・ご注文をもって、本規約に同意したものとみなします。

## 第1条(会員登録)

会員登録は、登録希望者が当店の定める方法により申請し、当店がこれを承認することで完了します。登録情報に変更があった場合は、速やかにマイページから変更してください。

## 第2条(アカウントの管理)

会員は、メールアドレスおよびパスワードを自己の責任で管理するものとし、第三者への譲渡・貸与はできません。アカウントの不正利用により生じた損害について、当店は故意または重過失がある場合を除き責任を負いません。

## 第3条(売買契約)

売買契約は、会員またはゲストが注文を確定し、当店が注文受付の確認メールを送信した時点で成立します。表示価格はすべて税込価格です。

## 第4条(お支払い)

お支払い方法は、クレジットカード、コンビニ決済、PayPay、代金引換(現金)から選択できます。クレジットカード情報は決済代行会社が処理し、当店はこれを取得・保存しません。コンビニ決済は、ご注文後に表示される期限内にお支払いください。期限を過ぎた場合、そのご注文は自動的にキャンセルされます。クレジットカード・PayPay・コンビニ決済のご注文は、お支払いの確認後に発送します。返金は、ご利用のお支払い方法に応じた方法で行い、コンビニ決済の場合は銀行振込となります。

## 第5条(禁止事項)

転売目的の大量購入、虚偽の情報による登録・注文、他の会員や当店への迷惑行為、その他法令に違反する行為を禁止します。

## 第6条(規約の変更)

当店は、必要と判断した場合に本規約を変更できます。変更後の規約は、当店サイトに表示した時点から効力を生じます。
', '', 'published', '2026-11-07 12:48:01.224468+00', '2026-11-07 12:48:01.224468+00', 'markdown'),
  ('3', 'privacy', 'system', 'プライバシーポリシー', '
Agentic Store(以下「当店」)は、お客様の個人情報を以下の方針に基づき取り扱います。

## 取得する情報

会員登録・ご注文の際に、氏名、住所、電話番号、メールアドレス、生年月日等を取得します。パスワードはハッシュ化して保存し、平文では保存しません。クレジットカード情報は決済代行会社のページで直接ご入力いただくため、当店が取得・保存することはありません。

## 利用目的

取得した情報は、商品の配送、代金の請求、ご注文に関する連絡、メールマガジンの配信(同意いただいた方のみ)、およびサービス改善のために利用します。

## 第三者提供

法令に基づく場合を除き、お客様の同意なく個人情報を第三者に提供しません。配送に必要な範囲で配送業者に住所・氏名・電話番号を提供します。また、決済処理のため、決済代行サービス「KOMOJU」を提供する株式会社Degicaに、ご注文金額など決済に必要な情報を提供します。

## 開示・訂正・削除

ご自身の個人情報の確認・訂正はマイページから行えます。会員情報の削除は、マイページの「退会手続き」から行えます。

## 退会時の取り扱い

退会手続きを行うと、氏名・住所・電話番号・メールアドレスなどの会員情報は直ちに削除(匿名化)され、ログインはできなくなります。未使用のクーポンは無効になります。ご注文の記録は、法令上の保存義務および決済・返金対応のため、個人を特定できない形で原則として7年間保存します。投稿いただいたレビューは、表示が匿名であるため、商品の評価集計のためにそのまま保存します。再登録時のクーポン(WELCOME500)の重複付与を防ぐため、退会したメールアドレスのハッシュ値(元のアドレスを復元できない値)のみを保存し、過去に同クーポンを利用したかどうかを確認します。

お問い合わせの際に入力いただいたお名前・メールアドレス・お問い合わせ内容は、お問い合わせへの回答および必要なご連絡のためにのみ利用します。
', '', 'published', '2026-11-07 12:48:01.224468+00', '2026-11-07 12:48:01.224468+00', 'markdown'),
  ('4', 'legal', 'system', '特定商取引法に基づく表記', '
**販売業者**:Agentic Store

**運営責任者**:店舗運営責任者

**所在地**:請求があった場合に遅滞なく開示します

**お問い合わせ**:マイページおよび注文確認メールに記載の窓口までご連絡ください

**販売価格**:各商品ページに表示する価格(消費税込)

**商品代金以外の必要料金**:送料 550円(税込、商品合計5,000円以上で無料)、代引き手数料 330円(税込、代金引換の場合)、コンビニ決済手数料(コンビニ決済の場合、店頭でのお支払い時に発生)

**お支払い方法**:クレジットカード、コンビニ決済、PayPay、代金引換(現金)

**お支払い時期**:クレジットカード・PayPayはご注文確定時、コンビニ決済はご注文後に表示される期限内、代金引換は商品お届け時

**商品の引渡時期**:ご注文確定後、通常2〜5営業日でお届け(コンビニ決済の場合はご入金の確認後)

**返品・交換**:不良・誤配送は到着後7日以内にご連絡ください。お客様都合の返品は未開封品に限ります。返金はご利用のお支払い方法に応じた方法で行い、コンビニ決済の場合は銀行振込となります
', '', 'published', '2026-11-07 12:48:01.224468+00', '2026-11-07 12:48:01.224468+00', 'markdown'),
  ('5', 'faq', 'system', 'よくある質問', '
## 配送について

## お支払いについて

## 返品・交換について
', '', 'draft', '2026-11-07 12:48:01.224468+00', '2026-11-07 12:48:01.224468+00', 'markdown');

INSERT INTO public.coupons (id, code, discount_yen, min_subtotal_yen, active, description, starts_at, ends_at, stripe_featured) VALUES
  ('1', 'WELCOME500', '500', '0', 't', 'オープン記念クーポン', NULL, NULL, 't');

INSERT INTO public.home_sections (section_key, heading, is_visible, sort_order, updated_at) VALUES
  ('categories', '', 't', '10', '2026-11-07 12:48:01.244959+00'),
  ('new_arrivals', '', 't', '20', '2026-11-07 12:48:01.244959+00'),
  ('ranking', '', 't', '30', '2026-11-07 12:48:01.244959+00'),
  ('featured', '', 't', '40', '2026-11-07 12:48:01.244959+00'),
  ('notices', '', 't', '50', '2026-11-07 12:48:01.244959+00');

INSERT INTO public.home_slots (key, headline, body, cta_label, cta_url, is_enabled, starts_at, ends_at, updated_at, image_path) VALUES
  ('announcement', '', '', '', '', 'f', NULL, NULL, '2026-11-07 12:48:01.244959+00', ''),
  ('hero', '好きなものを、もっと手軽に。', 'ふだんの買い物はそのままに。AIエージェントにも頼める、新しいECストア。', '新着商品を見る', '#new', 't', NULL, NULL, '2026-11-07 12:48:01.244959+00', '');

INSERT INTO public.menu_items (id, menu_key, label, url, is_visible, sort_order) OVERRIDING SYSTEM VALUE VALUES
  ('1', 'footer', '送料・お支払いについて', '/guide', 't', '10'),
  ('2', 'footer', 'お問い合わせ', '/contact', 't', '20'),
  ('3', 'footer', '利用規約', '/terms', 't', '30'),
  ('4', 'footer', 'プライバシーポリシー', '/privacy', 't', '40'),
  ('5', 'footer', '特定商取引法に基づく表記', '/legal', 't', '50');

INSERT INTO public.notices (id, body, title, display_from, display_to, is_visible, created_at, updated_at) VALUES
  ('1', 'オープン記念クーポン「WELCOME500」を配布中です。', '', '2026-11-01', NULL, 't', '2026-11-07 12:48:01.112607+00', '2026-11-07 12:48:01.112607+00'),
  ('2', 'ライブグッズ先行予約セットの予約受付を開始しました。', '', '2026-10-28', NULL, 't', '2026-11-07 12:48:01.112607+00', '2026-11-07 12:48:01.112607+00');

INSERT INTO public.tax_classes (id, code, name) OVERRIDING SYSTEM VALUE VALUES
  ('1', 'standard', '標準税率'),
  ('2', 'reduced', '軽減税率'),
  ('3', 'exempt', '非課税'),
  ('4', 'out_of_scope', '不課税');

INSERT INTO public.products (id, name, price_yen, rating, image_path, category, is_new, discount_percent, is_preorder, is_event, created_at, description, status, updated_at, category_id, tax_class_id, cost_yen, sale_price_yen, sale_starts_at, sale_ends_at, product_code, brand, manufacturer, country_of_origin, spec_text, features_text, description_sp, search_keywords, track_stock, allow_backorder, low_stock_threshold, weight_grams, size_length_mm, size_width_mm, size_height_mm, cool_delivery, mail_delivery, free_shipping, undeliverable_regions, lead_time_days, publish_starts_at, publish_ends_at, is_featured, display_order, slug, seo_title, meta_description) VALUES
  ('4', 'Tokio パーカー', '6980', '4.7', '/assets/images/product-tokio-hoodie.svg', 'アパレル', 'f', '15', 'f', 'f', '2026-11-02 12:48:00.927007+00', '非同期ランタイムTokioをモチーフにしたプルオーバーパーカーです。裏地は起毛で、秋から春にかけて着られます。', 'public', '2026-11-07 12:48:01.071945+00', '1', '1', NULL, NULL, NULL, NULL, NULL, '', '', '', '', '', '', '', 't', 'f', NULL, NULL, NULL, NULL, NULL, 'f', 'f', 'f', '', NULL, NULL, NULL, 'f', '0', NULL, '', ''),
  ('1', 'Rust Tシャツ', '2980', '4.5', '/assets/images/product-rust-tshirt.svg', 'アパレル', 't', '0', 'f', 'f', '2026-11-06 12:48:00.927007+00', 'プログラミング言語Rustをモチーフにした半袖Tシャツです。綿素材で、普段使いしやすい一着です。', 'public', '2026-11-07 12:48:01.071945+00', '1', '1', NULL, NULL, NULL, NULL, NULL, '', '', '', '', '', '', '', 't', 'f', NULL, NULL, NULL, NULL, NULL, 'f', 'f', 'f', '', NULL, NULL, NULL, 'f', '0', NULL, '', ''),
  ('10', 'オンライン勉強会 参加チケット', '3000', '4.9', '/assets/images/product-study-ticket.svg', 'イベント', 't', '0', 'f', 't', '2026-11-07 00:48:00.927007+00', 'オンライン勉強会の参加チケットです。開催日が近づきましたら、参加方法をメールでご案内します。', 'public', '2026-11-07 12:48:01.071945+00', '2', '1', NULL, NULL, NULL, NULL, NULL, '', '', '', '', '', '', '', 't', 'f', NULL, NULL, NULL, NULL, NULL, 'f', 'f', 'f', '', NULL, NULL, NULL, 'f', '0', NULL, '', ''),
  ('9', '会場受取限定タオル', '1800', '4.4', '/assets/images/product-venue-towel.svg', 'イベント', 'f', '0', 'f', 't', '2026-11-01 12:48:00.927007+00', 'イベント会場で受け取る限定タオルです。ご注文時の注文番号を会場でご提示ください。', 'public', '2026-11-07 12:48:01.071945+00', '2', '1', NULL, NULL, NULL, NULL, NULL, '', '', '', '', '', '', '', 't', 'f', NULL, NULL, NULL, NULL, NULL, 'f', 'f', 'f', '', NULL, NULL, NULL, 'f', '0', NULL, '', ''),
  ('8', 'ライブグッズ先行予約セット', '4500', '4.6', '/assets/images/product-live-goods.svg', 'イベント', 'f', '0', 't', 't', '2026-11-03 12:48:00.927007+00', 'ライブイベントのグッズをまとめた先行予約セットです。発売日以降、順次発送します。', 'public', '2026-11-07 12:48:01.071945+00', '2', '1', NULL, NULL, NULL, NULL, NULL, '', '', '', '', '', '', '', 't', 'f', NULL, NULL, NULL, NULL, NULL, 'f', 'f', 'f', '', NULL, NULL, NULL, 'f', '0', NULL, '', ''),
  ('5', 'SQLx キーホルダー', '880', '4', '/assets/images/product-sqlx-keyholder.svg', 'グッズ', 't', '0', 'f', 'f', '2026-11-07 10:48:00.927007+00', 'SQLxの名前を入れたアクリルキーホルダーです。鍵やバッグの目印に使えます。', 'public', '2026-11-07 12:48:01.071945+00', '3', '1', NULL, NULL, NULL, NULL, NULL, '', '', '', '', '', '', '', 't', 'f', NULL, NULL, NULL, NULL, NULL, 'f', 'f', 'f', '', NULL, NULL, NULL, 'f', '0', NULL, '', ''),
  ('3', 'Askama ステッカーセット', '550', '4.2', '/assets/images/product-askama-stickers.svg', 'グッズ', 'f', '0', 'f', 'f', '2026-10-28 12:48:00.927007+00', 'テンプレートエンジンAskamaのステッカーセットです。ノートPCやノートに貼れる複数枚入りです。', 'public', '2026-11-07 12:48:01.071945+00', '3', '1', NULL, NULL, NULL, NULL, NULL, '', '', '', '', '', '', '', 't', 'f', NULL, NULL, NULL, NULL, NULL, 'f', 'f', 'f', '', NULL, NULL, NULL, 'f', '0', NULL, '', ''),
  ('2', 'Axum マグカップ', '1650', '4.8', '/assets/images/product-axum-mug.svg', 'グッズ', 'f', '0', 'f', 'f', '2026-11-04 12:48:00.927007+00', 'WebフレームワークAxumの名前をあしらったマグカップです。自宅やオフィスのデスク用にどうぞ。', 'public', '2026-11-07 12:48:01.071945+00', '3', '1', NULL, NULL, NULL, NULL, NULL, '', '', '', '', '', '', '', 't', 'f', NULL, NULL, NULL, NULL, NULL, 'f', 'f', 'f', '', NULL, NULL, NULL, 'f', '0', NULL, '', '');

INSERT INTO public.product_images (id, product_id, image_path, sort_order, created_at, alt_text) VALUES
  ('1', '4', '/assets/images/product-tokio-hoodie.svg', '0', '2026-11-07 12:48:01.255207+00', ''),
  ('2', '1', '/assets/images/product-rust-tshirt.svg', '0', '2026-11-07 12:48:01.255207+00', ''),
  ('3', '10', '/assets/images/product-study-ticket.svg', '0', '2026-11-07 12:48:01.255207+00', ''),
  ('4', '9', '/assets/images/product-venue-towel.svg', '0', '2026-11-07 12:48:01.255207+00', ''),
  ('5', '8', '/assets/images/product-live-goods.svg', '0', '2026-11-07 12:48:01.255207+00', ''),
  ('6', '5', '/assets/images/product-sqlx-keyholder.svg', '0', '2026-11-07 12:48:01.255207+00', ''),
  ('7', '3', '/assets/images/product-askama-stickers.svg', '0', '2026-11-07 12:48:01.255207+00', ''),
  ('8', '2', '/assets/images/product-axum-mug.svg', '0', '2026-11-07 12:48:01.255207+00', '');

INSERT INTO public.product_variants (id, product_id, option_signature, stock, created_at, updated_at, price_yen, sku, jan_code, image_path, is_enabled, incoming_qty, incoming_date) OVERRIDING SYSTEM VALUE VALUES
  ('1', '1', '', '12', '2026-11-07 12:48:01.131327+00', '2026-11-07 12:48:01.131327+00', NULL, NULL, NULL, NULL, 't', '0', NULL),
  ('2', '2', '', '3', '2026-11-07 12:48:01.131327+00', '2026-11-07 12:48:01.131327+00', NULL, NULL, NULL, NULL, 't', '0', NULL),
  ('3', '3', '', '0', '2026-11-07 12:48:01.131327+00', '2026-11-07 12:48:01.131327+00', NULL, NULL, NULL, NULL, 't', '0', NULL),
  ('4', '4', '', '8', '2026-11-07 12:48:01.131327+00', '2026-11-07 12:48:01.131327+00', NULL, NULL, NULL, NULL, 't', '0', NULL),
  ('5', '5', '', '25', '2026-11-07 12:48:01.131327+00', '2026-11-07 12:48:01.131327+00', NULL, NULL, NULL, NULL, 't', '0', NULL),
  ('6', '8', '', '5', '2026-11-07 12:48:01.131327+00', '2026-11-07 12:48:01.131327+00', NULL, NULL, NULL, NULL, 't', '0', NULL),
  ('7', '9', '', '2', '2026-11-07 12:48:01.131327+00', '2026-11-07 12:48:01.131327+00', NULL, NULL, NULL, NULL, 't', '0', NULL),
  ('8', '10', '', '20', '2026-11-07 12:48:01.131327+00', '2026-11-07 12:48:01.131327+00', NULL, NULL, NULL, NULL, 't', '0', NULL);

INSERT INTO public.shop_settings (id, shop_name, tagline, shop_description, contact_email, contact_url, support_note, footer_note, copyright_holder, top_title, default_meta_description, updated_at) VALUES
  ('1', 'Agentic Store', '', 'AIエージェントと買い物ができるECストア', '', '', '', '', '', '', 'Agentic Store — AIエージェントと買い物ができるECストア。商品検索、カート、代金引換でのご注文ができます。', '2026-11-07 12:48:01.15649+00');

INSERT INTO public.tags (id, slug, label, description) VALUES
  ('1', 'waterproof', '防水', ''),
  ('2', 'lightweight', '軽量', ''),
  ('3', 'large-capacity', '大容量', ''),
  ('4', 'insulated', '保温', ''),
  ('5', 'quick-dry', '速乾', ''),
  ('6', 'compact', 'コンパクト', ''),
  ('7', 'durable', '耐久性', ''),
  ('8', 'gift', 'ギフト向け', ''),
  ('9', 'event', 'イベント向け', ''),
  ('10', 'beginner', '初心者向け', ''),
  ('11', 'limited', '限定', ''),
  ('12', 'new', '新作', '');

INSERT INTO public.tax_rates (id, tax_class_id, rate_percent, effective_from, effective_to) OVERRIDING SYSTEM VALUE VALUES
  ('1', '1', '10', '2019-10-01', NULL),
  ('2', '2', '8', '2019-10-01', NULL),
  ('3', '3', '0', '2019-10-01', NULL),
  ('4', '4', '0', '2019-10-01', NULL);

SELECT pg_catalog.setval('public.categories_id_seq', 3, true);
SELECT pg_catalog.setval('public.content_pages_id_seq', 5, true);
SELECT pg_catalog.setval('public.coupons_id_seq', 1, true);
SELECT pg_catalog.setval('public.menu_items_id_seq', 5, true);
SELECT pg_catalog.setval('public.notices_id_seq', 2, true);
SELECT pg_catalog.setval('public.product_images_id_seq', 8, true);
SELECT pg_catalog.setval('public.product_variants_id_seq', 8, true);
SELECT pg_catalog.setval('public.products_id_seq', 10, true);
SELECT pg_catalog.setval('public.tags_id_seq', 12, true);
SELECT pg_catalog.setval('public.tax_classes_id_seq', 4, true);
SELECT pg_catalog.setval('public.tax_rates_id_seq', 4, true);