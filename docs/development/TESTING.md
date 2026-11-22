# テストガイド

このプロジェクトのテストは3層で構成されています。

1. **単体テスト（cargo test）**：Rust コード内のユニットテストです。
   ロジック単体を検証します。
2. **品質ゲート（scripts/check.sh）**：フォーマット、静的解析、単体テスト、
   ドキュメント、依存クレートの脆弱性監査を一括で実行する関門です。
3. **E2E スイート（scripts/tests/test_*.sh）**：起動中のアプリに対して HTTP
   経由で実際に操作し、画面・メール・DB の結果まで確認するシェル製の
   スイート群です。アプリの振る舞いの最終確認はこの層が担います。

## 品質ゲート（check.sh）

```bash
./scripts/check.sh
```

実行内容は次のとおりです（いずれか1つでも失敗するとゲートは失敗します）。

| 段階 | コマンド | 見るもの |
|---|---|---|
| フォーマット | `cargo fmt --check` | 整形の崩れ |
| 静的解析 | `cargo clippy --all-targets -- -D warnings` | 警告ゼロ（警告はエラー扱い） |
| 単体テスト | `cargo test` | ユニットテスト全件 |
| ドキュメント | `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps` | ドキュメントコメントの不備 |
| 依存監査 | `cargo audit` | 既知の脆弱性があるクレート |

`cargo audit` には `cargo-audit` のインストールが必要です
（`cargo install cargo-audit --locked`）。

## 単体試験仕様書

単体テスト全件（301件）の一覧と実施記録は
[単体試験仕様書](UNIT_TEST_SPEC.md)で管理しています。各ケースに
試験 ID（`UT-001` 形式）、対象、試験観点、期待結果、実施結果を
明記し、コードとの乖離を防ぐために保守スクリプトで運用します。

```bash
bash scripts/ops/unit_test_spec.sh generate  # 一覧表を cargo test の一覧から再構築
bash scripts/ops/unit_test_spec.sh run       # 全件実行し、結果と実施記録を書き込む
bash scripts/ops/unit_test_spec.sh check     # 一覧とコードの 1:1 一致を検証（テスト追加後に必須）
```

## E2E スイートの仕組み

各スイートは `scripts/tests/lib.sh` の共通ハーネス（`ok` / `ng` のカウンタ、
CSRF トークン抽出の `csrf()`、結果表示の `result()`）を使い、`curl` で
アプリを操作します。終了時に `RESULT: N passed, M failed` と表示され、
失敗があれば非ゼロで終わります。

多くのスイートは「すでに起動している開発アプリ」
（http://127.0.0.1:3000）に対して実行します。一部は専用の設定で
アプリを別途起動します（`start_app` を使うスイート：決済、SNS、
エージェントチェックアウト）。外部サービスは実物ではなくスタブを
相手にします。

| スタブ | 対象 | 使うスイートの例 |
|---|---|---|
| `scripts/stubs/komoju_stub.py` | KOMOJU 決済 API と Webhook | test_payments.sh、test_order_cancel.sh、test_agent_checkout.sh |
| `scripts/stubs/stripe_stub.py` | Stripe 決済 API と Webhook | test_stripe.sh |
| `scripts/stubs/sns_stub.py` | SNS ログインの OIDC プロバイダ | test_sns.sh |
| `scripts/stubs/llm_stub.py` | LLM エンドポイント | test_agent_chat.sh |

## 前提条件と fixture

### 初回のみ：fixture の作成

新しい DB に入っているのはカタログ（マイグレーション2の
シード）だけです。多くのスイートが前提とする共有 fixture
（会員 `test@example.com` と有効なカテゴリ `テスト`）は、
次の手順で作成します（冪等で、再実行しても既存の fixture
はそのまま残ります）。

```bash
./run.sh                                    # アプリと Mailpit を起動
bash scripts/ops/setup_test_fixtures.sh     # fixture を作成
```

会員は SQL やマイグレーションではなく、実際の登録フローで
作成されます。既知パスワードのアカウントは、この手順を
意図的に実行した場合にだけ存在します。この fixture が無いと
約15本のスイートが前提を欠いたり分かりにくい形で失敗したり
するため、新しい DB では最初に実行してください。

- `./run.sh` でアプリが起動していること。DB（PostgreSQL）と
  Mailpit のコンテナ（`agentic-db` / `agentic-mail`）も起動している
  こと。確認コードなどのメールは Mailpit の API
  （http://127.0.0.1:8025）から読み取ります。
- 共有 fixture のアカウントを使います。会員は `test@example.com`
  （上の手順で作成。パスワードはルート README 参照）。管理者は
  `.env` の `ADMIN_EMAIL` / `ADMIN_PASSWORD`（`.env.example` の
  Admin bootstrap の組）で、初回起動時に作成される管理者です。
- 一部のスイートは「オーナーの会員レコード」を DB から探して使います。
  そのメールアドレスは環境変数 `OWNER_EMAIL` で上書きでき、既定値は
  `owner-fixture@example.com` です。自前のシードで実行する場合は
  `OWNER_EMAIL=<対象のメールアドレス>` を付けて実行してください。
- 決済系のスイートは KOMOJU をテスト／スタブモードで使い、
  `PUBLIC_BASE_URL=http://127.0.0.1:3000` を前提にします。
  実際の課金は発生しません。
- 共有アプリに対して実行するスイート（`test_checkout_c.sh` など）は、
  KOMOJU をダミーのテストキーで有効化しておく必要があります
  （`config.toml` の `komoju_enabled = true` と、`.env` の
  `KOMOJU_SECRET_KEY` / `KOMOJU_WEBHOOK_SECRET` にテスト用の
  ダミー値）。実際の呼び出しは `scripts/stubs/` のスタブが
  受けます。`test_payments.sh` と `test_stripe.sh` は自前で
  キーを設定するため不要です。
- 一部のスイートは専用ポートでアプリを別途起動します
  （`test_contact.sh`：3100 と 3200、`test_csrf.sh`：3200、
  `test_agent_chat.sh`：3300）。そのポートを別のローカル
  アプリが使っていると起動を拒否するため、先にそちらを
  止めてください。
- 住所の自動入力は `postal_codes` テーブルを参照しますが、
  新しい DB では空です。日本郵便の CSV を
  `scripts/ops/import_postal_codes.sh` で取り込んでください
  （データセット自体は同梱していません。入手先はスクリプト
  冒頭に記載）。自動入力を検証するスイートには取り込みが
  必要です。
- スイート同士は同じ開発 DB と同じアプリを共有します。**並行実行せず、
  1本ずつ直列に**実行してください。各スイートは再実行できるよう、
  自分が作った fixture を片付ける前提で書かれています。

## 注意：ログインスロットル

ログイン試行のスロットルはアプリのプロセス内メモリにあります
（同一メールアドレス＋IP で失敗が続くと一定時間ロックされます）。
ログインを大量に行うスイート（`test_admin.sh`、`test_admin_totp.sh`、
`test_account.sh`、`test_sns.sh`、`test_totp.sh` など）を続けて実行
すると、スロットルに引っかかって無関係な失敗が連鎖します。
**ログイン負荷の高いスイートの間では、アプリを再起動して新しい
プロセスにしてから**実行してください。再起動でカウンタはリセット
されます。

## ケース ID の規約

スイート内のケースには `接頭辞-番号` の ID が付いています。
接頭辞は検証領域を表します。

| 接頭辞 | 領域 | 例 |
|---|---|---|
| `P` | ポイント | P-16（失効済みロットへの返金復元） |
| `SM` | ステップメール | SM-09（送信失敗時の繰り延べ） |
| `TC` | 仕様書由来のテストケース | TC-01〜TC-13（アドレス帳） |
| `TS` | Stripe 決済 | TS（test_stripe.sh） |
| `CG` | カバレッジギャップ横断 | CG（test_coverage_gaps.sh） |
| `AC` | エージェントチャット | AC（test_agent_chat.sh） |
| `AS` | 通知テンプレート等 | AS（test_notification_templates.sh） |

このほか、プログラムや仕様への相互参照として `SF`、`ST`、`BIZ`、
`R2`、`SEC`、`C` などのタグがコメントやラベルに現れることがあります。

## 既知の不安定なスイート

`test_agent_checkout.sh` はローカル LLM（Ollama）の応答に依存する
ため、モデルの解釈次第で結果が揺れる**既知の非決定的スイート**です。
失敗した場合はまず再実行して再現するかを確認してください。
それ以外のスイートは決定的で、失敗は原則として本物の不具合です。

## 運用スクリプト（scripts/ops/）

テストではなく運用・保守のためのスクリプトです。

| スクリプト | 内容 |
|---|---|
| `scripts/ops/backup_db.sh` | DB のバックアップ取得 |
| `scripts/ops/reconcile_payments.sh` | 決済レコードの照合 |
| `scripts/ops/unit_test_spec.sh` | 単体試験仕様書の保守（前節） |
| `scripts/ops/setup_test_fixtures.sh` | 共有 fixture（会員とカテゴリ）の作成 |
| `scripts/ops/import_postal_codes.sh` | 郵便番号データ（日本郵便 CSV）の取り込み。整形に `cleanse_postal_csv.py` を使う |

## スイート一覧

項目数の数え方：ケース ID 方式のスイートはファイル内のケース ID
マーカーの種類数を「ケース」とし、それ以外は `ok` / `ng` の検証
呼び出し箇所数を「検証箇所」としています（静的カウントのため、
ループ内検証などにより実行時の合否件数とは一致しません）。

実行方法はすべて共通で、リポジトリのルートから
`bash scripts/tests/<スイート名>` です。

| スイート | 検証領域 | 項目数 |
|---|---|---|
| test_account.sh | アカウントのセルフサービス（パスワード再設定、メール変更、プロフィール） | 検証98箇所 |
| test_address_book.sh | アドレス帳（配送先の登録・編集・削除と検証） | 13ケース |
| test_admin.sh | 管理画面全体（バックオフィスの各画面と基本動作） | 検証260箇所 |
| test_admin_roles.sh | 管理者ロールとエリア別権限（owner / staff） | 29ケース |
| test_admin_totp.sh | 管理者の二要素認証（TOTP 設定、ログイン時のチャレンジ、リカバリコード） | 検証36箇所 |
| test_agent.sh | ショッピングエージェント（/agent チャット経由の買い物支援） | 検証134箇所 |
| test_agent_chat.sh | エージェントチャット画面と LLM 連携 | 23ケース |
| test_agent_checkout.sh | エージェントによる KOMOJU 決済（チャット内精算）※LLM 依存で不安定 | 検証168箇所 |
| test_api.sh | エージェント向け API（発見プロファイル、カタログ、セッション） | 検証69箇所 |
| test_cart_b.sh | カート基礎（ゲストの追加、バッジ、カート画面、数量ステッパー） | 検証48箇所 |
| test_categories.sh | カテゴリ管理（管理画面 CRUD とバリデーション） | 検証72箇所 |
| test_checkout_c.sh | チェックアウト（ゲスト／会員、クーポン、二重送信、在庫減算、価格スナップショット） | 検証90箇所 |
| test_cms.sh | CMS：店舗設定ページとストアフロントへの反映 | 検証30箇所 |
| test_cms_home.sh | CMS：ホーム画面コンテンツ（お知らせバー等） | 検証96箇所 |
| test_cms_menus.sh | CMS：ヘッダー／フッターメニュー | 検証70箇所 |
| test_cms_pages.sh | CMS：コンテンツページ | 検証102箇所 |
| test_contact.sh | お問い合わせ（フォーム送信と管理側の対応フロー） | 15ケース |
| test_coupon_admin.sh | クーポン管理（発行・編集・条件設定） | 12ケース |
| test_coupon_classes.sh | クーポンの種別・併用クラス | 7ケース |
| test_coverage_gaps.sh | 他スイートが触れない機能の横断スモーク（ゲスト注文照会、領収書、返品受付、セグメント、休業日、ギフト） | 13ケース |
| test_csrf.sh | CSRF ミドルウェア（トークン検証とローテーションのライフサイクル） | 17ケース |
| test_delivery_slots.sh | 配送希望日時の選択 UI と検証 | 13ケース |
| test_design_fixes.sh | デザインレビュー修正のスモーク | スモーク（独自形式） |
| test_detail_d.sh | 商品詳細ページ | 検証44箇所 |
| test_favorites.sh | お気に入り（会員限定トグルとマイページ一覧） | 検証36箇所 |
| test_guest_cart_sweep.sh | ゲストカートの保持期間スイープ | 検証8箇所 |
| test_happy.sh | ハッピーパス回帰（登録→確認→ログインの基本動線） | 検証10箇所 |
| test_header.sh | ヘッダー（ゲスト時／会員時の表示） | 検証16箇所 |
| test_home_pins.sh | トップページのピン留めとスタッフのおすすめ | 15ケース |
| test_mail.sh | メール基盤（同意記録、テンプレートの無害化、配信） | 検証70箇所 |
| test_notification_templates.sh | 通知メールテンプレートの管理編集 | 29ケース |
| test_order_cancel.sh | 会員の注文キャンセル（在庫・ポイント・返金の連動） | 13ケース |
| test_payments.sh | KOMOJU 決済統合（スタブ相手のセッション、Webhook、返金、補償リトライ） | 検証204箇所 |
| test_points.sh | ポイントプログラム（付与、利用、失効、返金時の復元） | 16ケース |
| test_postal_codes.sh | 郵便番号の自動入力（自前ルックアップ API の利用） | 検証24箇所 |
| test_product_csv.sh | 商品・カテゴリ・タグの CSV 入出力（形式、見出し、文字コード） | 検証48箇所 |
| test_product_upload.sh | 商品画像アップロード（形式検証とギャラリー） | 検証77箇所 |
| test_reorder.sh | もう一度購入（過去の注文からの再追加） | 検証20箇所 |
| test_reviews.sh | レビュー（半ステップ評価と任意コメント） | 検証80箇所 |
| test_search.sh | 商品検索 | 検証30箇所 |
| test_security_headers.sh | セキュリティヘッダーとセッション Cookie のフラグ | 検証30箇所 |
| test_shipping_mail.sh | 発送通知メール（送信の一回性） | 15ケース |
| test_sns.sh | SNS ログイン（スタブ OIDC プロバイダとの往復） | 検証169箇所 |
| test_stage_e.sh | ログイン時のカートマージと登録時のカート再キー | 検証12箇所 |
| test_stage_f.sh | ランキング、送料の境界（5,000円）、検索ボタン、ログの秘匿 | 検証12箇所 |
| test_step_mail.sh | ステップメール（ウェルカム／レビュー依頼のシナリオ実行） | 9ケース |
| test_stock_consistency.sh | 在庫の整合性（在庫はバリエーションのみが保持する） | 検証64箇所 |
| test_stripe.sh | Stripe 決済統合（スタブ相手の Checkout、Webhook 署名検証、返金、レール切替） | 10ケース |
| test_tags.sh | タグ管理（管理画面 CRUD とバリデーション） | 検証54箇所 |
| test_tax.sh | 消費税（税区分ごとの内税計算と注文スナップショット） | 検証44箇所 |
| test_totp.sh | 会員の二要素認証（登録、ログイン時のチャレンジ、リカバリコード） | 検証98箇所 |
| test_upgrade_features.sh | 商品アップグレード（セール価格、バリエーション項目、在庫の振る舞い） | 検証36箇所 |
| test_upgrade_shipping_seo.sh | 配送連携、公開期間、SEO／スラッグの振る舞い | 検証26箇所 |
| test_validation.sh | バリデーション表示（必須バッジ、項目別エラー） | 検証8箇所 |
| test_withdraw.sh | 退会（アカウント削除） | 10ケース |
