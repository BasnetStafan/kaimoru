# 単体試験仕様書（UNIT_TEST_SPEC）

本書は、本プロジェクトの単体試験（Rust のユニットテスト）の一覧と実施記録である。
対象・試験観点・期待結果をテストごとに明記し、コードとの乖離を防ぐために
保守スクリプトで管理する。

## 運用方法

保守スクリプト: `scripts/ops/unit_test_spec.sh`

| コマンド | 内容 |
|---|---|
| `generate` | `cargo test --workspace -- --list` から一覧表を再構築する。既存ケースの ID と記入済みの内容は保持され、新規テストは新しい ID と（未記入）マーカー付きで追加される |
| `run` | `cargo test --workspace` を全件実行し、各ケースの結果と日付入りの実施記録を本書へ書き込む |
| `check` | 本書のケース一覧が `cargo test -- --list` と 1:1 で一致し、（未記入）が残っていないことを検証する（テスト追加後に必ず実行する） |

手順: テストを追加したら `generate` → 新規行の対象・観点・期待結果を記入 → `check` で検証。
試験を実施したら `run` で結果を記録する。

## 試験 ID の規約

- ID は `UT-001` 形式で、一覧への登場順に採番する。
- ID はケースの恒久的な識別子であり、**再利用しない**。テストが削除されても
  その ID は欠番のまま残る（採番済みの最大 ID は一覧の MAX_UT_ID マーカーで管理する）。
- 表はクレート → モジュールの順にグループ化している。

## ケース一覧

<!-- MAX_UT_ID: 319 -->
<!-- INVENTORY:BEGIN -->

### app / api::mcp::tools

| 試験ID | テスト名（フルパス） | 対象（関数・対象） | 試験観点 | 期待結果 | 結果 |
|---|---|---|---|---|---|
| UT-307 | api::mcp::tools::tests::defaults_are_valid | api::mcp::tools | （未記入） | （未記入） | 未実施 |
| UT-308 | api::mcp::tools::tests::inverted_price_range_is_refused | api::mcp::tools | （未記入） | （未記入） | 未実施 |
| UT-309 | api::mcp::tools::tests::negative_price_is_refused | api::mcp::tools | （未記入） | （未記入） | 未実施 |
| UT-310 | api::mcp::tools::tests::page_below_one_is_refused | api::mcp::tools | （未記入） | （未記入） | 未実施 |
| UT-311 | api::mcp::tools::tests::unknown_sort_is_refused | api::mcp::tools | （未記入） | （未記入） | 未実施 |

### app / api::stripe_webhook

| 試験ID | テスト名（フルパス） | 対象（関数・対象） | 試験観点 | 期待結果 | 結果 |
|---|---|---|---|---|---|
| UT-001 | api::stripe_webhook::tests::reencoded_body_and_wrong_secret_reject | signature_valid（Stripe Webhook署名検証） | 異常系・改ざん検知 | 再エンコードした本文、別シークレット、形式不正なヘッダのいずれでも署名検証が false を返すこと | Pass |
| UT-002 | api::stripe_webhook::tests::stale_timestamp_rejects_even_with_a_valid_hmac | signature_valid | 境界値・リプレイ防止 | HMAC自体は有効でもタイムスタンプが許容幅（300秒）を超えると false を返すこと | Pass |
| UT-003 | api::stripe_webhook::tests::valid_signature_accepts_within_tolerance | signature_valid | 正常系 | 許容幅内の正しい署名は true を返すこと | Pass |

### app / controllers::storefront::login

| 試験ID | テスト名（フルパス） | 対象（関数・対象） | 試験観点 | 期待結果 | 結果 |
|---|---|---|---|---|---|
| UT-004 | controllers::storefront::login::safe_next_order_tests::order_detail_paths_are_allowed | safe_next（ログイン後遷移先） | 正常系 | /mypage/orders/12 がそのまま遷移先として許可されること | Pass |
| UT-005 | controllers::storefront::login::safe_next_order_tests::order_like_paths_are_rejected | safe_next | 異常系 | 末尾スラッシュ、非数値ID、追加パス、クエリ付き、// 始まりの注文風パスはすべて None になること | Pass |
| UT-006 | controllers::storefront::login::safe_next_product_tests::product_like_paths_are_rejected | safe_next | 異常系 | 末尾スラッシュ、非数値、追加パス、クエリ付き、スラッグ形式の商品風パスはすべて None になること | Pass |
| UT-007 | controllers::storefront::login::safe_next_product_tests::product_paths_are_allowed | safe_next | 正常系 | /products/42 がそのまま許可されること | Pass |
| UT-008 | controllers::storefront::login::tests::allowlisted_pages_pass | safe_next | 正常系 | /mypage、/checkout、/mypage/orders が許可されること | Pass |
| UT-009 | controllers::storefront::login::tests::open_redirect_attempts_fall_back | safe_next | 異常系・オープンリダイレクト対策 | 外部URL、バックスラッシュ混入、/admin、未指定、空文字はすべて None（既定先へフォールバック）になること | Pass |

### app / middleware::admin_ip_guard

| 試験ID | テスト名（フルパス） | 対象（関数・対象） | 試験観点 | 期待結果 | 結果 |
|---|---|---|---|---|---|
| UT-010 | middleware::admin_ip_guard::tests::cf_header_counts_only_from_a_loopback_peer | effective_ip | 正常系・異常系 | ループバック接続時のみ CF ヘッダの IP（203.0.113.7）を採用し、リモート接続では実 IP（198.51.100.9）を使うこと | Pass |
| UT-011 | middleware::admin_ip_guard::tests::cidr_v4_matches_the_whole_range | Entry::matches（CIDR照合） | 境界値 | /24 範囲の両端（.0 と .255）が一致し、範囲外（203.0.114.1）は一致しないこと | Pass |
| UT-012 | middleware::admin_ip_guard::tests::exact_v4_matches_only_itself | Entry::matches | 正常系・異常系 | 完全一致のアドレスのみ一致し、隣のアドレスは一致しないこと | Pass |
| UT-013 | middleware::admin_ip_guard::tests::families_never_cross_match | Entry::matches | 異常系 | IPv4 エントリは IPv6 アドレスに、IPv6 エントリは IPv4 アドレスに一致しないこと | Pass |
| UT-014 | middleware::admin_ip_guard::tests::invalid_entries_are_errors | Entry::parse | 異常系 | 非 IP 文字列、範囲外プレフィックス（/33、/129）、不正なサフィックス、空文字がすべてエラーになること | Pass |
| UT-015 | middleware::admin_ip_guard::tests::prefix_edges | Entry::parse / matches | 境界値 | /32 は本人のみ、/0 は全アドレスに一致すること | Pass |
| UT-016 | middleware::admin_ip_guard::tests::v6_exact_and_cidr | Entry::matches（IPv6） | 正常系・境界値 | ::1 の完全一致、2001:db8::/32 の範囲内一致と範囲外不一致が正しいこと | Pass |

### app / middleware::admin_permission_guard

| 試験ID | テスト名（フルパス） | 対象（関数・対象） | 試験観点 | 期待結果 | 結果 |
|---|---|---|---|---|---|
| UT-017 | middleware::admin_permission_guard::tests::account_management_is_owner_only | gate_for（管理画面権限ゲート） | 検証・権限 | /admin/admins 配下は Gate::OwnerOnly になること | Pass |
| UT-018 | middleware::admin_permission_guard::tests::area_prefixes_cover_their_subpaths | gate_for | 検証・権限 | /admin/mail 配下はメール領域、/admin/settings/shop は設定領域のゲートになること | Pass |
| UT-019 | middleware::admin_permission_guard::tests::dashboard_and_account_paths_are_any_admin | gate_for | 検証・権限 | /admin、login、logout、profile、password は Gate::Any になること | Pass |
| UT-020 | middleware::admin_permission_guard::tests::prefixes_stop_at_segment_boundaries | gate_for | 境界値 | /admin/mailbox や /admin/settings2 は領域プレフィックスに化けず OwnerOnly になること | Pass |
| UT-021 | middleware::admin_permission_guard::tests::unlisted_paths_are_owner_only_by_default | gate_for | 検証・フェイルクローズ | 未掲載パスは既定で OwnerOnly に倒れること | Pass |

### app / middleware::csrf

| 試験ID | テスト名（フルパス） | 対象（関数・対象） | 試験観点 | 期待結果 | 結果 |
|---|---|---|---|---|---|
| UT-022 | middleware::csrf::tests::every_form_is_served_and_text_between_is_kept | inject_tokens（CSRFトークン注入） | 正常系 | POST フォーム2つ両方にトークン字段が注入され、フォーム前後のテキストが保持されること | Pass |
| UT-023 | middleware::csrf::tests::get_form_is_left_alone | inject_tokens | 正常系 | GET フォームは変更されないこと | Pass |
| UT-024 | middleware::csrf::tests::htmx_form_without_method_gets_the_field | inject_tokens | 正常系 | method 属性のない hx-post フォームにもトークン字段が注入されること | Pass |
| UT-025 | middleware::csrf::tests::missing_token_is_none | submitted_token | 異常系 | トークン字段がなければ None を返すこと | Pass |
| UT-026 | middleware::csrf::tests::only_the_standard_safe_methods_skip_the_token | is_safe_method | 検証 | 標準の安全メソッドのみ true を返し、新奇なメソッドは false になること | Pass |
| UT-027 | middleware::csrf::tests::page_without_forms_is_unchanged | inject_tokens | 正常系 | フォームのないページは変更されないこと | Pass |
| UT-028 | middleware::csrf::tests::post_form_gets_the_field | inject_tokens | 正常系 | POST フォームにトークン字段が注入されること | Pass |
| UT-029 | middleware::csrf::tests::similar_field_names_do_not_match | submitted_token | 異常系 | 類似名のフィールドはトークンとして一致せず None になること | Pass |
| UT-030 | middleware::csrf::tests::token_found_among_other_fields | submitted_token | 正常系 | 他フィールドに混ざっていてもトークン値 abc123 を取り出せること | Pass |
| UT-031 | middleware::csrf::tests::token_found_as_only_field | submitted_token | 正常系 | 単独フィールドからトークン値 deadbeef を取り出せること | Pass |

### app / models::agent

| 試験ID | テスト名（フルパス） | 対象（関数・対象） | 試験観点 | 期待結果 | 結果 |
|---|---|---|---|---|---|
| UT-032 | models::agent::tests::preset_form_values_round_trip | Preset::from_form_value | 変換 | 全プリセットが form_value 経由で往復し、未知の値 random は None になること | Pass |
| UT-033 | models::agent::tests::preset_weights_sum_to_one | Preset の重み定義 | 検証 | 各プリセットの重み合計が 1.0（誤差 1e-9 未満）であること | Pass |

### app / models::carrier

| 試験ID | テスト名（フルパス） | 対象（関数・対象） | 試験観点 | 期待結果 | 結果 |
|---|---|---|---|---|---|
| UT-034 | models::carrier::tests::tracking_url_only_for_deep_linkable_carriers | tracking_url | 変換・異常系 | sagawa は追跡 URL を生成し、other と未知キャリアは None になること | Pass |

### app / models::delivery

| 試験ID | テスト名（フルパス） | 対象（関数・対象） | 試験観点 | 期待結果 | 結果 |
|---|---|---|---|---|---|
| UT-035 | models::delivery::tests::delivery_estimate_labels_come_from_the_window | earliest_delivery_label / delivery_window_range_label | 変換 | 基準日から「11月5日（木）」「11月5日（木）〜11月17日（火）」のラベルが得られること | Pass |
| UT-036 | models::delivery::tests::delivery_request_formats_each_shape | format_delivery_request | 変換 | 未指定は「指定なし（最短でお届け）」、日付のみ・時間帯付きはそれぞれの表示形式になること | Pass |
| UT-037 | models::delivery::tests::delivery_validation_rejects_tampering_and_pairing_breaks | validate_delivery_request | 異常系・改ざん検知 | 日付なしの時間帯指定は SlotNeedsDate、窓外の日付は WindowChanged、不正な日付・未知スロットは Invalid となり、正しい組は t1416 と「14時〜16時」を返すこと | Pass |
| UT-038 | models::delivery::tests::delivery_window_spans_plus_two_to_plus_fourteen | delivery_window | 境界値 | 最短が +2 日（11月5日）、最長が +14 日（11月17日）、候補が13日分であること | Pass |
| UT-039 | models::delivery::tests::holidays_push_the_earliest_date_and_leave_the_latest | delivery_window_with_holidays | 境界値 | 休業日により最短が11月6日へ繰り下がり最長は不変、休業日は候補から除外され、休業日の指定は検証エラーになること | Pass |
| UT-040 | models::delivery::tests::shipping_details_accept_empty_and_valid_pairs | validate_shipping_details | 正常系 | 空ペア、yamato＋追跡番号（前後空白を除去）、番号のみ、other のみが Ok になること | Pass |
| UT-041 | models::delivery::tests::shipping_details_reject_unknown_carrier_and_bad_numbers | validate_shipping_details | 異常系 | 未知キャリアは UnknownCarrier、空白入り・日本語混入・過長の番号は BadTrackingNumber になること | Pass |

### app / models::gift

| 試験ID | テスト名（フルパス） | 対象（関数・対象） | 試験観点 | 期待結果 | 結果 |
|---|---|---|---|---|---|
| UT-042 | models::gift::tests::gift_lines_render_only_what_was_chosen | gift_display_lines / gift_mail_lines | 変換 | 未選択時は何も出力されず、のし＋名入れは「のし：御中元（名入れ：山田）」、メッセージのみはメール行だけが生成されること | Pass |
| UT-043 | models::gift::tests::gift_validation_accepts_each_shape | validate_gift_options | 正常系 | なし・のし＋名入れ・カードのみの各形状が受理され、ラベルと名入れが保持されること | Pass |
| UT-044 | models::gift::tests::gift_validation_rejects_tampering_pairing_and_overlong | validate_gift_options | 異常系 | 未知のしは UnknownNoshi、のし無しの名入れは NameWithoutNoshi、過長な名前・メッセージは TooLong 系エラーになること | Pass |

### app / models::order

| 試験ID | テスト名（フルパス） | 対象（関数・対象） | 試験観点 | 期待結果 | 結果 |
|---|---|---|---|---|---|
| UT-045 | models::order::tests::order_numbers_carry_the_jst_date | Order::order_number | 変換 | 注文番号が JST 日付入りの AS-20261102-0001 形式になること | Pass |
| UT-046 | models::order::tests::order_numbers_parse_back | parse_order_number | 変換・異常系 | 正常形式は連番を返し、0000・桁不足・接頭辞違い・余分なセグメントは None、前後空白と小文字は許容されること | Pass |
| UT-047 | models::order::tests::payment_labels_follow_the_parcel | payment_status_label | 変換 | delivered のみ「代引き（回収済み）」、cancelled は「代引き（対象外）」、他は「代引き（未収）」になること | Pass |
| UT-048 | models::order::tests::placed_times_render_in_jst_regardless_of_process_timezone | placed_display / placed_long | 変換 | プロセスのタイムゾーンに依らず「2026/11/01 09:30」「2026年11月1日 09:30」と表示されること | Pass |
| UT-049 | models::order::tests::placed_times_roll_over_the_date_line_in_jst | placed_display | 境界値 | 日付を跨ぐ時刻でも JST で「2026/11/02 01:00」と表示されること | Pass |

### app / models::payment

| 試験ID | テスト名（フルパス） | 対象（関数・対象） | 試験観点 | 期待結果 | 結果 |
|---|---|---|---|---|---|
| UT-050 | models::payment::tests::capture_after_cancel_is_flagged_never_applied | next_status（決済状態遷移） | 状態遷移・異常系 | cancelled / expired への captured は適用せず FlagLateCapture になること | Pass |
| UT-051 | models::payment::tests::card_capture_moves_pending_to_paid | next_status | 状態遷移 | pending ＋ captured で Apply（PAID）になること | Pass |
| UT-052 | models::payment::tests::external_order_numbers_suffix_from_the_second_attempt | external_order_num_for | 変換 | 1回目はそのまま、2回目以降は -2、-3 のサフィックスが付くこと | Pass |
| UT-053 | models::payment::tests::failed_attempt_is_not_terminal | next_status | 状態遷移 | pending ＋ failed で FAILED、failed ＋ captured では PAID へ復帰できること | Pass |
| UT-054 | models::payment::tests::konbini_authorized_stays_pending | next_status | 状態遷移 | pending ＋ authorized では PENDING を維持すること | Pass |
| UT-055 | models::payment::tests::method_slugs_round_trip | PaymentMethod::from_slug / slug | 変換 | 全メソッドが slug 経由で往復し、bitcoin は None、既定値は Cod であること | Pass |
| UT-056 | models::payment::tests::paid_never_regresses | next_status | 状態遷移・冪等性 | paid はどのプロバイダ状態を受けても Ignore で後退しないこと | Pass |
| UT-057 | models::payment::tests::refunds_progress_from_captured_amounts | next_status | 状態遷移 | 一部返金で PARTIALLY_REFUNDED、全額返金で REFUNDED になること | Pass |
| UT-058 | models::payment::tests::terminal_states_only_refresh_themselves | next_status | 状態遷移 | expired / refunded は自己更新のみで、他の状態語は Ignore になること | Pass |
| UT-059 | models::payment::tests::unknown_provider_status_never_invents_a_state | next_status | 異常系 | 未知の状態語では現状態を維持し、新しい状態を作らないこと | Pass |
| UT-060 | models::payment::tests::zero_yen_komoju_payment_is_born_settled_without_a_session | initial_status_for / settles_without_session | 境界値 | 0円の KOMOJU 系は PAID で生まれセッション無しで確定し、1円は PENDING、COD は0円でも PENDING だがセッション無しであること | Pass |

### app / models::return_request

| 試験ID | テスト名（フルパス） | 対象（関数・対象） | 試験観点 | 期待結果 | 結果 |
|---|---|---|---|---|---|
| UT-061 | models::return_request::tests::the_walk_is_linear_with_rejection_exits | legal_next_statuses / is_terminal | 状態遷移 | requested からは approved / rejected、inspected からは refunded のみ、refunded は次がなく、rejected は終端、approved は非終端であること | Pass |

### app / models::review

| 試験ID | テスト名（フルパス） | 対象（関数・対象） | 試験観点 | 期待結果 | 結果 |
|---|---|---|---|---|---|
| UT-062 | models::review::tests::aggregate_averages_half_steps_too | ReviewAggregate の集計表示 | 変換 | 平均表示が「3.3」、星フィルが66%になること | Pass |
| UT-063 | models::review::tests::aggregate_rounds_to_one_decimal | ReviewAggregate の集計表示 | 変換 | 平均表示が「4.4」（小数1桁）、件数が5、星フィルが88%になること | Pass |
| UT-064 | models::review::tests::aggregate_with_no_reviews_has_no_average | ReviewAggregate の集計表示 | 異常系・空 | レビュー0件では平均が None、表示が空、星フィルが0%になること | Pass |
| UT-065 | models::review::tests::review_fill_percent_covers_half_steps | Review::stars_fill_percent | 変換 | 半刻み評価の星フィルが90%、100%、10%になること | Pass |

### app / models::tax

| 試験ID | テスト名（フルパス） | 対象（関数・対象） | 試験観点 | 期待結果 | 結果 |
|---|---|---|---|---|---|
| UT-066 | models::tax::tests::floor_of_tax_exact_division_and_zero_cases | tax_from_inclusive（内税計算） | 金額計算・境界値 | 1,080円・8%で税80円、金額0では0、税率0では0になること | Pass |
| UT-067 | models::tax::tests::floor_of_tax_reference_values | tax_from_inclusive | 金額計算 | 500円・8%で37円、5,100円・10%で463円になること（切捨て） | Pass |
| UT-068 | models::tax::tests::shop_rounding_applies_to_the_tax_fraction | tax_from_inclusive | 金額計算 | 同一入力（500円・8%）で Round は37円、Ceil は38円と、店舗の端数設定が税の端数に効くこと | Pass |

### app / pii

| 試験ID | テスト名（フルパス） | 対象（関数・対象） | 試験観点 | 期待結果 | 結果 |
|---|---|---|---|---|---|
| UT-069 | pii::tests::blind_index_is_deterministic_and_normalised | blind_index | 検証・暗号 | 正規化により同一メールアドレスが同一インデックス（64桁）になり、別の値は異なること | Pass |
| UT-070 | pii::tests::empty_and_legacy_values_pass_through | encrypt_with / decrypt_field | 変換・互換 | 空文字と旧形式の平文はそのまま通過すること | Pass |
| UT-071 | pii::tests::equal_plaintexts_encrypt_differently | encrypt_with | 検証・暗号 | 同一平文でも暗号文は毎回異なること（ランダム nonce） | Pass |
| UT-072 | pii::tests::round_trip_restores_the_value | encrypt_with / decrypt_with | 正常系 | 接頭辞付きで保存され、復号で元のメールアドレスに戻ること | Pass |
| UT-073 | pii::tests::tampered_ciphertext_fails_the_tag_check | decrypt_with | 異常系・改ざん検知 | 改ざんした暗号文と別キーでの復号はタグ検証でエラーになること | Pass |

### app / pricing

| 試験ID | テスト名（フルパス） | 対象（関数・対象） | 試験観点 | 期待結果 | 結果 |
|---|---|---|---|---|---|
| UT-074 | pricing::tests::groups_thousands | format_with_symbol（金額表示） | 変換 | ¥2,980、¥1,234,567、¥500、¥0 と桁区切りされること | Pass |
| UT-075 | pricing::tests::keeps_the_minus_sign | format_with_symbol | 変換 | 負額が ¥-1,500 と符号を保持すること | Pass |
| UT-076 | pricing::tests::yen_word_symbol_follows_the_amount | format_with_symbol | 変換 | 記号「円」は金額の後ろに付き「500円」になること | Pass |

### app / services::admin::home_content_service

| 試験ID | テスト名（フルパス） | 対象（関数・対象） | 試験観点 | 期待結果 | 結果 |
|---|---|---|---|---|---|
| UT-077 | services::admin::home_content_service::tests::home_fingerprint_is_stable_and_reacts_to_edits | fingerprint（ホーム内容） | 検証 | 同内容は同一指紋、タイトル・ピン留め・画像の各変更で指紋が変わること | Pass |

### app / services::admin::mail_service

| 試験ID | テスト名（フルパス） | 対象（関数・対象） | 試験観点 | 期待結果 | 結果 |
|---|---|---|---|---|---|
| UT-078 | services::admin::mail_service::tests::empty_fields_error_together | 管理メール検証 | 異常系 | 空の項目がまとめて4件エラーになること | Pass |
| UT-079 | services::admin::mail_service::tests::jst_local_parses_as_jst_wall_clock | 管理メールの日時解釈 | 変換 | 入力が JST の壁時計として解釈され UTC 01:30 に変換され、エラーがないこと | Pass |
| UT-080 | services::admin::mail_service::tests::script_only_body_fails_as_empty | 管理メール検証 | 異常系 | script のみの本文は body_html エラー（実質空）になること | Pass |
| UT-081 | services::admin::mail_service::tests::script_tag_is_sanitised_out_at_validation | 管理メール検証 | 検証・サニタイズ | script タグと alert が除去され、本文の「こんにちは」は保持されること | Pass |

### app / services::admin::menu_service

| 試験ID | テスト名（フルパス） | 対象（関数・対象） | 試験観点 | 期待結果 | 結果 |
|---|---|---|---|---|---|
| UT-082 | services::admin::menu_service::tests::menu_fingerprint_is_stable_and_reacts_to_edits | fingerprint（メニュー） | 検証 | 同内容は同一指紋、ラベル変更と非表示化で指紋が変わること | Pass |

### app / services::admin::notice_service

| 試験ID | テスト名（フルパス） | 対象（関数・対象） | 試験観点 | 期待結果 | 結果 |
|---|---|---|---|---|---|
| UT-083 | services::admin::notice_service::tests::body_limit_is_two_thousand_characters | validate_notice | 境界値 | 本文2,000字は Ok、超過は body エラー1件になること | Pass |
| UT-084 | services::admin::notice_service::tests::empty_title_and_body_error_together | validate_notice | 異常系 | タイトルと本文の空がまとめて2件エラーになること | Pass |
| UT-085 | services::admin::notice_service::tests::end_before_start_errors_on_the_end_field | validate_notice | 異常系 | 終了日が開始日より前だと display_to に「終了日は開始日以降の日付を選択してください」のエラー、同日は Ok であること | Pass |
| UT-086 | services::admin::notice_service::tests::garbage_date_is_an_error_not_an_open_end | validate_notice | 異常系 | 解釈不能な日付は display_from エラー1件になり、オープンエンドに化けないこと | Pass |
| UT-087 | services::admin::notice_service::tests::state_derivation_follows_flag_then_period | display_state | 状態導出 | 表示フラグ→期間の順で Hidden / Visible / OutOfPeriod が導出され、同日範囲は Visible になること | Pass |
| UT-088 | services::admin::notice_service::tests::title_limit_counts_characters_not_bytes | validate_notice | 境界値 | タイトル上限はバイト数でなく文字数で数えられること | Pass |

### app / services::admin::product_service

| 試験ID | テスト名（フルパス） | 対象（関数・対象） | 試験観点 | 期待結果 | 結果 |
|---|---|---|---|---|---|
| UT-089 | services::admin::product_service::tests::detects_each_raster_type_from_its_magic_bytes | uploaded_image_extension | 検証 | PNG、JPEG、GIF87a/89a、WebP をマジックバイトで判定できること | Pass |
| UT-090 | services::admin::product_service::tests::refuses_svg_and_other_non_raster_bytes | uploaded_image_extension | 異常系 | SVG、平文、空、WAVE は None になること | Pass |
| UT-091 | services::admin::product_service::tests::stored_name_falls_back_to_the_bare_uuid | 保存ファイル名生成 | 異常系 | 元名が使えない場合は UUID のみの stem になること | Pass |
| UT-092 | services::admin::product_service::tests::stored_name_keeps_japanese_letters | 保存ファイル名生成 | 変換 | 「夏用シャツ-」で始まり、日本語が保持されること | Pass |
| UT-093 | services::admin::product_service::tests::stored_name_prefixes_the_first_five_letters | 保存ファイル名生成 | 変換 | 先頭5文字「summe-」＋拡張子 .png になること | Pass |
| UT-094 | services::admin::product_service::tests::stored_name_sanitises_a_traversal_filename | 保存ファイル名生成 | 異常系・パストラバーサル対策 | 「evil-」で始まり、/ も \ も含まないこと | Pass |
| UT-095 | services::admin::product_service::tests::stored_name_uses_the_detected_extension_not_the_clients | 保存ファイル名生成 | 検証 | クライアント申告でなく検出した拡張子 .png が使われ、.jpeg を含まないこと | Pass |
| UT-096 | services::admin::product_service::tests::upload_paths_must_stay_inside_the_uploads_store | is_upload_path | 異常系 | uploads 外、.. を含むパス、外部 URL、不存在ファイルはすべて false になること | Pass |

### app / services::admin::rfm_service

| 試験ID | テスト名（フルパス） | 対象（関数・対象） | 試験観点 | 期待結果 | 結果 |
|---|---|---|---|---|---|
| UT-312 | services::admin::rfm_service::tests::distinct_population_spreads_across_bands | services::admin::rfm_service | （未記入） | （未記入） | 未実施 |
| UT-313 | services::admin::rfm_service::tests::named_segments_first_match | services::admin::rfm_service | （未記入） | （未記入） | 未実施 |
| UT-314 | services::admin::rfm_service::tests::segment_counts_cover_every_member_once | services::admin::rfm_service | （未記入） | （未記入） | 未実施 |
| UT-315 | services::admin::rfm_service::tests::single_member_is_band_five_everywhere | services::admin::rfm_service | （未記入） | （未記入） | 未実施 |
| UT-316 | services::admin::rfm_service::tests::ties_always_share_a_band | services::admin::rfm_service | （未記入） | （未記入） | 未実施 |

### app / services::admin::shipping_csv_service

| 試験ID | テスト名（フルパス） | 対象（関数・対象） | 試験観点 | 期待結果 | 結果 |
|---|---|---|---|---|---|
| UT-097 | services::admin::shipping_csv_service::tests::carrier_cells_accept_codes_and_labels | carrier_code_for_cell | 変換 | コード、日本語ラベル、前後空白付きを受け付け、dhl は None になること | Pass |
| UT-098 | services::admin::shipping_csv_service::tests::slot_codes_follow_each_carriers_bands | 時間帯コード変換 | 変換 | 業者ごとの時間帯コードになること（yamato の am は 0812、sagawa の am は 01、ゆうパックはラベル表記） | Pass |

### app / services::agent_approval_service

| 試験ID | テスト名（フルパス） | 対象（関数・対象） | 試験観点 | 期待結果 | 結果 |
|---|---|---|---|---|---|
| UT-099 | services::agent_approval_service::tests::approval_expires_at_its_ttl_boundary | AgentApproval::is_expired | 境界値 | ちょうど TTL で期限切れ、+1秒は有効、-1秒は期限切れであること | Pass |
| UT-100 | services::agent_approval_service::tests::final_fingerprint_binds_method_fee_and_address | final_fingerprint | 検証 | 支払方法・手数料・住所のいずれの変更でも最終指紋が変わること | Fail |
| UT-101 | services::agent_approval_service::tests::final_fingerprint_trims_address_fields | final_fingerprint | 変換 | 住所項目の前後空白は指紋に影響しないこと | Pass |
| UT-102 | services::agent_approval_service::tests::final_snapshot_preserves_the_lines_binding | 承認スナップショット | 検証 | 作成時の lines_fingerprint、cod、郵便番号、代引き手数料330円が保持されること | Pass |
| UT-103 | services::agent_approval_service::tests::fingerprint_changes_when_a_line_is_added_or_removed | fingerprint_for | 検証 | 行の追加と空カートとの比較で指紋が変わること | Pass |
| UT-104 | services::agent_approval_service::tests::fingerprint_changes_with_each_bound_total | fingerprint_for | 検証 | 小計・送料・合計いずれの変更でも指紋が変わること | Pass |
| UT-105 | services::agent_approval_service::tests::fingerprint_changes_with_quantity_price_and_variant | fingerprint_for | 検証 | 数量・単価・バリアント ID の変更で指紋が変わること | Pass |
| UT-106 | services::agent_approval_service::tests::fingerprint_ignores_line_storage_order | fingerprint_for | 冪等性 | 行の格納順は指紋に影響しないこと | Pass |
| UT-107 | services::agent_approval_service::tests::fingerprint_ignores_non_purchasable_lines | fingerprint_for | 検証 | 下書き行と売り切れ行は指紋に影響しないこと | Pass |
| UT-108 | services::agent_approval_service::tests::fingerprint_is_deterministic_hex | fingerprint_for | 検証 | 決定的で、64桁の小文字16進文字列であること | Pass |
| UT-109 | services::agent_approval_service::tests::has_agent_line_requires_a_purchasable_agent_line | has_agent_line | 検証 | 購入可能なエージェント行がある場合のみ true になること | Pass |
| UT-110 | services::agent_approval_service::tests::snapshot_lines_follow_canonical_order | 承認スナップショット | 変換 | 行が ID 昇順（1、2）に並び、行合計が 2,980円と1,100円、支払方法と配送先は未確定（None）であること | Pass |

### app / services::agent_chat_rules

| 試験ID | テスト名（フルパス） | 対象（関数・対象） | 試験観点 | 期待結果 | 結果 |
|---|---|---|---|---|---|
| UT-111 | services::agent_chat_rules::tests::affirmations_match_heads_and_comma_continuations | is_affirmation | 正常系 | 「はい」「はい、…」「お願いします」「そうして」「OK」「うん、いいよ」が肯定として一致すること | Pass |
| UT-112 | services::agent_chat_rules::tests::affirmations_refuse_lookalikes_and_other_speech | is_affirmation | 異常系 | 「はいそうです」「探して」、質問文、空文字は肯定に一致しないこと | Pass |
| UT-113 | services::agent_chat_rules::tests::closings_and_resets_match_their_phrases | is_closing / is_reset_request | 正常系・異常系 | 終了とリセットの定型句が一致し、条件付きの「ありがたいけど…」は終了に化けないこと | Pass |
| UT-114 | services::agent_chat_rules::tests::search_promises_match_the_stall_wording | promises_search | 正常系・異常系 | 検索を約束する文面が一致し、予算の質問や了承のみの文面は一致しないこと | Pass |
| UT-115 | services::agent_chat_rules::tests::transforms_read_presets_with_rating_first | follow_up_transform | 変換 | 評価優先は BestRated、安さは Cheapest、評価の言及が優先し、無関係な文は None になること | Pass |
| UT-116 | services::agent_chat_rules::tests::transforms_read_yen_amounts_as_budgets | follow_up_transform | 変換 | 「5000円」は5,000円、「1万円」は10,000円、全角数字とカンマ区切りも解釈され、金額のない文は None になること | Pass |

### app / services::agent_chat_service

| 試験ID | テスト名（フルパス） | 対象（関数・対象） | 試験観点 | 期待結果 | 結果 |
|---|---|---|---|---|---|
| UT-117 | services::agent_chat_service::tests::a_closing_turn_never_searches_and_keeps_the_brief | チャット判断（decide） | 正常系 | 終了ターンは Reply（終了メッセージ）になり、検索せず brief 更新もないこと | Pass |
| UT-118 | services::agent_chat_service::tests::a_deliberate_preset_is_search_signal | has_signal | 検証 | 明示されたプリセットは検索シグナルになり、デフォルト brief は無シグナルであること | Pass |
| UT-119 | services::agent_chat_service::tests::a_follow_up_adjustment_re_runs_the_previous_search | チャット判断 | 正常系 | フォローアップの調整で前回検索が Cheapest で再実行されること | Pass |
| UT-120 | services::agent_chat_service::tests::a_narrowed_brief_gets_only_its_category_lines | プロンプト構築 | 検証 | 絞り込み済み brief のプロンプトには対象カテゴリの商品のみが入り、別商品は入らないこと | Pass |
| UT-121 | services::agent_chat_service::tests::a_promise_without_signal_stays_a_reply | チャット判断 | 異常系 | シグナルのない検索の約束は Reply のままになること | Pass |
| UT-122 | services::agent_chat_service::tests::a_promised_search_becomes_the_search | チャット判断 | 正常系 | 約束された検索は Search になること | Pass |
| UT-123 | services::agent_chat_service::tests::a_search_with_a_need_survives_the_downgrade | チャット判断 | 正常系 | ニーズのある検索はダウングレードされず Search を維持すること | Pass |
| UT-124 | services::agent_chat_service::tests::a_search_without_a_message_gets_the_default_lead_in | チャット判断 | 正常系 | メッセージのない検索は既定文「承知しました。お探しします。」と予算5,000円の設定になること | Pass |
| UT-125 | services::agent_chat_service::tests::a_signalless_search_is_downgraded_to_a_reply | チャット判断 | 異常系 | シグナルのない検索は Reply にダウングレードされること | Pass |
| UT-126 | services::agent_chat_service::tests::a_signalless_search_is_still_downgraded_after_the_rules | チャット判断 | 異常系 | ルール適用後でもシグナルのない検索は Reply にダウングレードされること | Pass |
| UT-127 | services::agent_chat_service::tests::accumulated_needs_keep_a_follow_up_search_alive | チャット判断 | 正常系 | 蓄積したニーズがフォローアップ検索を Search のまま維持すること | Pass |
| UT-128 | services::agent_chat_service::tests::an_affirmation_after_a_question_forces_the_search | チャット判断 | 正常系 | 質問のあとの肯定は検索を強制すること | Pass |
| UT-129 | services::agent_chat_service::tests::an_affirmation_without_signal_changes_nothing | チャット判断 | 異常系 | シグナルのない肯定は何も変えず Reply のままであること | Pass |
| UT-130 | services::agent_chat_service::tests::an_unknown_category_update_is_dropped | brief 更新の解釈 | 異常系 | 未知カテゴリの更新は捨てられ（None）、正規カテゴリ「グッズ」は Set になること | Pass |
| UT-131 | services::agent_chat_service::tests::budget_zero_clears_and_a_numeric_string_sets | brief 更新の解釈 | 変換・境界値 | 予算0で Clear、数値文字列で Set（8,000円）、負数は None になること | Pass |
| UT-132 | services::agent_chat_service::tests::clearing_a_brief_item_removes_exactly_that_item | clear_brief_item | 正常系・異常系 | 項目ごとに正確に除去され（予算、条件「防水」、カテゴリ）、未知項目と再除去は None、他の項目は保持されること | Pass |
| UT-133 | services::agent_chat_service::tests::fenced_json_still_parses | parse_decision | 変換 | コードフェンス付き JSON でも Reply として解釈できること | Pass |
| UT-134 | services::agent_chat_service::tests::merge_marks_a_preset_update_explicit | brief のマージ | 検証 | マージで preset_explicit が立つこと | Pass |
| UT-135 | services::agent_chat_service::tests::merge_sets_clears_and_keeps | brief のマージ | 正常系 | 設定・クリア・保持が各項目に正しく適用されること | Pass |
| UT-136 | services::agent_chat_service::tests::parses_a_search_decision_with_brief_updates | parse_decision | 変換 | Search と、予算10,000円・カテゴリ・必須条件・Cheapest の更新が抽出されること | Pass |
| UT-137 | services::agent_chat_service::tests::parses_an_ask_decision_with_no_updates | parse_decision | 変換 | Ask とメッセージが抽出され、更新はないこと | Pass |
| UT-138 | services::agent_chat_service::tests::the_digest_cap_truncates_with_a_remainder_line | プロンプト構築（ダイジェスト） | 境界値 | 上位2点のみが入り、「（ほか1点）」の残数行が付くこと | Pass |
| UT-139 | services::agent_chat_service::tests::the_question_budget_forces_a_search_once_asking_is_spent | チャット判断 | 境界値 | 質問予算を使い切ると Search になること | Pass |
| UT-140 | services::agent_chat_service::tests::the_question_budget_keeps_asking_when_there_is_no_signal | チャット判断 | 正常系 | シグナルがなければ Ask を継続すること | Pass |
| UT-141 | services::agent_chat_service::tests::the_system_prompt_carries_the_grounding_and_the_state | システムプロンプト構築 | 検証 | カテゴリ・条件・予算・JSON 指示・在庫行を含み、対象外の商品は含まないこと | Pass |
| UT-142 | services::agent_chat_service::tests::unusable_output_is_refused | parse_decision | 異常系 | 非 JSON、action 欠落、未知の action、空メッセージはすべてエラーになること | Pass |

### app / services::agent_run_service

| 試験ID | テスト名（フルパス） | 対象（関数・対象） | 試験観点 | 期待結果 | 結果 |
|---|---|---|---|---|---|
| UT-143 | services::agent_run_service::tests::a_brief_without_a_budget_fits_everything | 予算投影 | 正常系 | 予算のない brief では合計3,530円がそのまま収まること | Pass |
| UT-144 | services::agent_run_service::tests::budget_cap_is_hard_and_the_overage_is_exact | 予算投影 | 金額計算・境界値 | 予測が3,530円、超過が正確に530円と算出されること | Pass |
| UT-145 | services::agent_run_service::tests::budget_projection_counts_the_owners_existing_lines | 予算投影 | 金額計算 | 既存行を含めて超過550円、送料0円、合計5,550円になること | Pass |
| UT-146 | services::agent_run_service::tests::keyword_match_never_satisfies_a_must_have | ニーズ照合（スコアリング） | 検証 | キーワード一致は必須条件を満たさず（met 0）、未達の「防水」が報告されること | Pass |
| UT-147 | services::agent_run_service::tests::must_have_relaxation_reports_the_unmet_needs | ニーズ照合の緩和 | 正常系 | 緩和で対象が2件になり未達が報告され、完全一致では未達がないこと | Pass |
| UT-148 | services::agent_run_service::tests::negated_keyword_does_not_match_but_a_positive_mention_does | keyword_matches | 異常系・正常系 | 否定文脈は一致せず、肯定的な言及は一致し、撥水のみの文は一致しないこと | Pass |
| UT-149 | services::agent_run_service::tests::pool_match_requires_some_need_credit | pool_has_need_match | 検証 | ニーズクレジットのないプールは不一致になること | Pass |
| UT-150 | services::agent_run_service::tests::price_score_normalises_across_the_pool | 価格スコア | 変換 | プール内最安が1.0、最高が0.0、同額なら全て1.0になること | Pass |
| UT-151 | services::agent_run_service::tests::rating_score_is_smoothed_toward_the_prior | smoothed_rating | 変換 | 未評価は事前値のまま、評価は事前値方向へ平滑化されること（5.0が1件で40/11、4.8が200件で995/210） | Pass |
| UT-152 | services::agent_run_service::tests::run_cap_refuses_the_eleventh_start_inside_an_hour | try_begin_run | 境界値・回数制限 | 同一セッションで1時間に11回目の開始が拒否され、別セッションは開始できること | Pass |
| UT-153 | services::agent_run_service::tests::tag_match_earns_full_credit_keyword_only_partial | ニーズクレジット | 変換 | タグ一致は1.0、キーワード一致は0.5のクレジットになること | Pass |
| UT-154 | services::agent_run_service::tests::unrated_products_score_on_the_prior | rating_or_prior | 変換 | 未評価と件数0は3.5、実評価があれば実値が使われ、スコアが0.7になること | Pass |
| UT-155 | services::agent_run_service::tests::variant_choice_defaults_to_most_stock_unless_a_need_names_a_label | バリアント選択 | 正常系 | 既定は在庫最多のバリアント（ID 12）、ニーズがラベルを指せばそのバリアント（ID 11）になること | Pass |

### app / services::api_key_service

| 試験ID | テスト名（フルパス） | 対象（関数・対象） | 試験観点 | 期待結果 | 結果 |
|---|---|---|---|---|---|
| UT-317 | services::api_key_service::tests::malformed_tokens_are_refused | parse_token | 異常系 | マーカーを持たないもの、シークレット部分を持たないもの、短いまたは16進数でないシークレット、空入力はいずれも拒否される（None）こと | 未実施 |
| UT-318 | services::api_key_service::tests::minted_token_parses_back | parse_token | 正常系 | 発行したトークンが自身のプレフィックス（16進数12文字）とシークレット（16進数64文字）に分割して戻ること | 未実施 |

### app / services::cart_service

| 試験ID | テスト名（フルパス） | 対象（関数・対象） | 試験観点 | 期待結果 | 結果 |
|---|---|---|---|---|---|
| UT-156 | services::cart_service::tests::all_free_shipping_lines_pay_no_shipping | カート金額計算 | 金額計算 | 全行が送料無料対象なら送料0円、無料までの残額も0円になること | Pass |
| UT-157 | services::cart_service::tests::discount_is_capped_at_the_subtotal | カート金額計算 | 金額計算・境界値 | 割引は小計1,000円で頭打ちになり、合計は送料＋代引き手数料の880円になること | Pass |
| UT-158 | services::cart_service::tests::empty_cart_is_all_zero | カート金額計算 | 境界値・空 | 空カートは合計も代引き手数料も0円になること | Pass |
| UT-159 | services::cart_service::tests::komoju_methods_never_carry_the_cod_fee | カート金額計算 | 金額計算 | KOMOJU 系の方法では代引き手数料が常に0円で、COD を選んだ場合のみ330円が加わること | Pass |
| UT-160 | services::cart_service::tests::no_method_chosen_yet_carries_no_cod_fee | カート金額計算 | 金額計算 | 支払方法が未選択の間は代引き手数料0円、合計3,530円になること | Pass |
| UT-161 | services::cart_service::tests::off_sale_lines_are_excluded_from_totals | カート金額計算 | 検証 | 販売終了の行は小計から除外されること（1,000円） | Pass |
| UT-162 | services::cart_service::tests::physical_below_threshold_pays_shipping_and_cod | カート金額計算 | 金額計算 | 小計2,980円、送料550円、代引き330円、合計3,860円、無料までの残額2,020円になること | Pass |
| UT-163 | services::cart_service::tests::shipping_is_free_exactly_at_the_threshold | カート金額計算 | 境界値 | 小計がちょうど5,000円で送料無料になること | Pass |
| UT-164 | services::cart_service::tests::sold_out_lines_are_excluded_from_totals | カート金額計算 | 検証 | 売り切れの行は小計から除外されること（1,000円） | Pass |
| UT-165 | services::cart_service::tests::tax_breakdown_decomposes_the_same_total | カート税内訳 | 金額計算 | 合計3,860円が標準税率1区分に分解され、課税対象3,860円・税350円になること | Pass |
| UT-166 | services::cart_service::tests::tax_breakdown_mixed_classes_floor_of_tax | カート税内訳 | 金額計算 | 合計4,940円が標準（税350円）と軽減（税80円）に分解され、税合計が430円になること | Pass |
| UT-167 | services::cart_service::tests::zero_priced_lines_still_pay_the_flat_shipping_fee | カート金額計算 | 金額計算・境界値 | 小計0円でも定額送料550円が課され、無料までの残額は5,000円になること | Pass |

### app / services::coupon_service

| 試験ID | テスト名（フルパス） | 対象（関数・対象） | 試験観点 | 期待結果 | 結果 |
|---|---|---|---|---|---|
| UT-168 | services::coupon_service::tests::absent_scope_target_blocks_a_product_coupon | クーポン適用判定 | 異常系 | 対象商品がカートに無い商品クーポンは適用されないこと | Pass |
| UT-169 | services::coupon_service::tests::caps_and_minimum_block_a_candidate | クーポン適用判定 | 境界値 | 上限と最低金額に届かない候補は適用されないこと | Pass |
| UT-170 | services::coupon_service::tests::entered_code_beats_automatic_in_the_same_class | クーポン適用判定 | 検証 | 同クラスでは自動付与より入力コードが優先され、割引が300円になること | Pass |
| UT-171 | services::coupon_service::tests::free_shipping_does_not_apply_when_shipping_is_already_free | クーポン適用判定 | 異常系 | 既に送料無料のときは送料クーポンが適用されないこと | Pass |
| UT-172 | services::coupon_service::tests::order_percent_prices_after_the_product_discount | クーポン金額計算 | 金額計算 | 注文 % クーポンは商品割引の後に適用され450円、総割引は950円になること | Pass |
| UT-173 | services::coupon_service::tests::product_percent_prices_from_the_scoped_basis | クーポン金額計算 | 金額計算 | 商品 % クーポンは対象範囲の基礎額から200円を割り引くこと | Pass |
| UT-174 | services::coupon_service::tests::three_classes_combine_on_one_cart | クーポン適用判定 | 正常系 | 3クラスが1つのカートに併用でき、割引500円・送料割引550円になること | Pass |

### app / services::form_ticket_service

| 試験ID | テスト名（フルパス） | 対象（関数・対象） | 試験観点 | 期待結果 | 結果 |
|---|---|---|---|---|---|
| UT-175 | services::form_ticket_service::tests::set_stamp_is_deterministic | stamp_of | 検証 | 同一の集合は同一スタンプ、別の集合は異なるスタンプになること | Pass |
| UT-176 | services::form_ticket_service::tests::set_stamp_keeps_field_boundaries | stamp_of | 検証 | ["ab","c"] と ["a","bc"] など、字段境界を跨ぐ同一化が起きないこと | Pass |
| UT-177 | services::form_ticket_service::tests::set_stamp_reacts_to_int_and_flag_fields | スタンプ生成 | 検証 | 数値とフラグの変更でスタンプが変わり、同値は同一になること | Pass |

### app / services::komoju_client

| 試験ID | テスト名（フルパス） | 対象（関数・対象） | 試験観点 | 期待結果 | 結果 |
|---|---|---|---|---|---|
| UT-178 | services::komoju_client::tests::payment_objects_translate_at_the_boundary | KOMOJU 支払オブジェクト変換 | 変換 | id が pay_123、金額4,740円、案内 URL、支払期限が UTC 表記に変換されること | Pass |
| UT-179 | services::komoju_client::tests::rfc3339_timestamps_parse_as_given | parse_komoju_time | 変換・異常系 | RFC3339 の時刻がそのまま解釈され、不正な文字列は None になること | Pass |

### app / services::marketing_mail_service

| 試験ID | テスト名（フルパス） | 対象（関数・対象） | 試験観点 | 期待結果 | 結果 |
|---|---|---|---|---|---|
| UT-180 | services::marketing_mail_service::tests::escape_html_neutralises_markup_characters | escape_html | 検証・サニタイズ | <、>、&、" がエスケープされること | Pass |
| UT-181 | services::marketing_mail_service::tests::member_name_cannot_expand_a_product_block | expand_product_blocks | 異常系・注入対策 | 会員名に含まれる {{product:2}} は展開されずリテラルのまま残り、後続の文も保持されること | Pass |
| UT-182 | services::marketing_mail_service::tests::rewrite_hrefs_wraps_only_the_listed_urls | rewrite_hrefs | 変換 | 対象 URL のみがトラッキング URL に書き換えられ、解除 URL も対象形式に含まれること | Pass |
| UT-183 | services::marketing_mail_service::tests::substitute_escapes_the_name_only_for_html | substitute（変数置換） | 検証・サニタイズ | HTML 版では会員名がエスケープされ、テキスト版では素のまま置換されること | Pass |
| UT-184 | services::marketing_mail_service::tests::substitute_replaces_both_variables | substitute | 変換 | 名前と解除 URL の両変数が置換されること | Pass |
| UT-185 | services::marketing_mail_service::tests::trackable_hrefs_skips_unsubscribe_and_dedupes | trackable_hrefs | 変換 | 解除リンクを除き、重複を除いて対象 URL が抽出されること | Pass |
| UT-186 | services::marketing_mail_service::tests::unterminated_product_block_stays_literal | expand_product_blocks | 異常系 | 未終了のブロックはリテラルのまま残り、解釈不能な ID のブロックは除去されること | Pass |

### app / services::mcp_cart_service

| 試験ID | テスト名（フルパス） | 対象（関数・対象） | 試験観点 | 期待結果 | 結果 |
|---|---|---|---|---|---|
| UT-319 | services::mcp_cart_service::tests::cart_handles_carry_the_prefix | カートハンドルの形式 | 境界値 | ハンドルのプレフィックスは mcp_ でランダムな16進数32文字、クレームトークンは16進数64文字であること | 未実施 |

### app / services::member_account_service

| 試験ID | テスト名（フルパス） | 対象（関数・対象） | 試験観点 | 期待結果 | 結果 |
|---|---|---|---|---|---|
| UT-187 | services::member_account_service::tests::birth_date_parsing | parse_birth_date | 変換・異常系 | 空は None、ISO 形式は Some、スラッシュ形式はエラーになること | Pass |
| UT-188 | services::member_account_service::tests::every_bad_field_is_reported_at_once | validate_details | 異常系 | password、カナ、郵便番号、電話の不正が一括で報告されること | Pass |
| UT-189 | services::member_account_service::tests::password_needs_a_letter_and_a_digit | validate_details | 検証 | 英字と数字を欠くパスワードは password エラーになること | Pass |
| UT-190 | services::member_account_service::tests::valid_details_have_no_errors | validate_details | 正常系 | 正しいフォームはエラーがないこと | Pass |

### app / services::notification_template_service

| 試験ID | テスト名（フルパス） | 対象（関数・対象） | 試験観点 | 期待結果 | 結果 |
|---|---|---|---|---|---|
| UT-191 | services::notification_template_service::tests::code_default_matches_legacy | render_default（認証コード） | 検証・回帰 | 認証コードメールのデフォルト文面が従来文面と完全一致すること（コード123456、有効期限10分） | Pass |
| UT-192 | services::notification_template_service::tests::konbini_default_matches_legacy | render_default（コンビニ案内） | 検証・回帰 | コンビニ支払い案内のデフォルト文面が従来文面と完全一致すること | Pass |
| UT-193 | services::notification_template_service::tests::order_confirmation_default_matches_legacy | render_default（注文確認） | 検証・回帰 | 注文確認メールのデフォルト文面が従来文面と完全一致すること | Pass |
| UT-194 | services::notification_template_service::tests::payment_confirmation_default_matches_legacy | render_default（支払完了） | 検証・回帰 | 支払完了メールのデフォルト文面が従来文面と完全一致すること | Pass |
| UT-195 | services::notification_template_service::tests::shipping_default_matches_legacy_full | render_default（発送通知・宛名付き） | 検証・回帰 | 発送通知（宛名付き）のデフォルト文面が従来文面と完全一致すること | Pass |
| UT-196 | services::notification_template_service::tests::shipping_default_matches_legacy_plain | render_default（発送通知・プレーン） | 検証・回帰 | 発送通知（プレーン）のデフォルト文面が従来文面と完全一致すること | Pass |
| UT-197 | services::notification_template_service::tests::sole_tag_line_with_empty_value_drops | render_text | 変換 | 値が空のタグだけの行は行ごと落ちること（結果が「店\n」になる） | Pass |
| UT-198 | services::notification_template_service::tests::validation_rejects_unknown_and_missing_required | validate_template | 異常系 | 未知タグと必須タグの欠落は body エラーになり、正しいテンプレートは Ok であること | Pass |

### app / services::order_service

| 試験ID | テスト名（フルパス） | 対象（関数・対象） | 試験観点 | 期待結果 | 結果 |
|---|---|---|---|---|---|
| UT-199 | services::order_service::tests::bad_postal_and_phone_are_both_reported | validate_delivery | 異常系 | 郵便番号と電話の不正が両方まとめて報告されること | Pass |
| UT-200 | services::order_service::tests::empty_flow_fields_are_filled_from_the_profile | 配送先の補完 | 正常系 | 空欄がプロフィール（山田、1500001、東京都、電話番号）で補完されること | Pass |
| UT-201 | services::order_service::tests::entered_fields_survive_the_fill | 配送先の補完 | 検証 | 入力済みの市区町村は保持され、空欄のみが補完されること | Pass |
| UT-202 | services::order_service::tests::guest_with_a_second_at_sign_fails_on_email | validate_delivery | 異常系 | @ を2つ含むメールアドレスは email エラー1件になること | Pass |
| UT-203 | services::order_service::tests::guest_without_email_fails_on_email_only | validate_delivery | 異常系 | メールアドレスのないゲストは email エラー1件のみになること | Pass |
| UT-204 | services::order_service::tests::member_needs_no_email_field | validate_delivery | 正常系 | 会員はメールアドレス欄なしで検証を通過すること | Pass |
| UT-205 | services::order_service::tests::overlong_delivery_fields_fail_per_field | validate_delivery | 境界値 | 過長な氏名・市区町村・建物名が各フィールドでエラーになること | Pass |
| UT-206 | services::order_service::tests::slot_without_a_date_fails_on_the_date_field | validate_delivery | 異常系 | 日付のない時間帯指定は delivery_date エラー1件になること | Pass |
| UT-207 | services::order_service::tests::unknown_slot_code_fails_on_the_slot_field | validate_delivery | 異常系 | 未知の時間帯コードは delivery_slot エラー1件になること | Pass |
| UT-208 | services::order_service::tests::valid_guest_flow_passes | validate_delivery | 正常系 | 正しいゲスト情報はエラーがないこと | Pass |

### app / services::password

| 試験ID | テスト名（フルパス） | 対象（関数・対象） | 試験観点 | 期待結果 | 結果 |
|---|---|---|---|---|---|
| UT-209 | services::password::tests::hash_round_trip | verify_password | 正常系・異常系 | 正しいパスワードは検証に成功し、誤ったパスワードと非ハッシュ文字列は失敗すること | Pass |
| UT-210 | services::password::tests::rule_counts_characters_not_bytes | password_rule_error | 境界値 | 12字（日本語＋英数）は可、短い・英字のみ・数字のみは不可、34字は可で66字は不可と、文字数基準で判定されること | Pass |

### app / services::payment_gateway

| 試験ID | テスト名（フルパス） | 対象（関数・対象） | 試験観点 | 期待結果 | 結果 |
|---|---|---|---|---|---|
| UT-211 | services::payment_gateway::tests::fake_card_session_captures_immediately | FakeGateway（カード） | 正常系 | Fake ではカード決済が即時に captured になり、金額が4,740円であること | Pass |
| UT-212 | services::payment_gateway::tests::fake_creation_failure_is_scriptable | FakeGateway | 異常系 | 失敗を仕込むとセッション作成がエラーになり、解除後は成功すること | Pass |
| UT-213 | services::payment_gateway::tests::fake_konbini_waits_then_captures | FakeGateway（コンビニ） | 状態遷移 | コンビニは authorized で支払期限と案内 URL を持ち、後に captured になること | Pass |
| UT-214 | services::payment_gateway::tests::fake_refund_accumulates_to_full | FakeGateway（返金） | 状態遷移 | 返金が累積して refunded になり、累計が4,740円になること | Pass |

### app / services::point_service

| 試験ID | テスト名（フルパス） | 対象（関数・対象） | 試験観点 | 期待結果 | 結果 |
|---|---|---|---|---|---|
| UT-215 | services::point_service::tests::earn_is_percent_of_base_floored | earned_points | 金額計算・境界値 | 10,000円の1%で100、999円で9（切捨て）、5%で500、0・負数・0%では0になること | Pass |
| UT-216 | services::point_service::tests::lines_deduction_allocated_pro_rata | earned_points_for_lines | 金額計算 | 行ごとに按分した結果が477になり、控除が基礎額以上では0になること | Pass |
| UT-217 | services::point_service::tests::lines_mixed_rates_and_exclusion | earned_points_for_lines | 金額計算 | 混合率では509、付与率0の行を除くと69になること | Pass |
| UT-218 | services::point_service::tests::lines_single_rate_matches_order_level | earned_points_for_lines | 金額計算 | 単一率では注文単位の計算と一致すること（100、控除500で95、混合5%で450） | Pass |
| UT-219 | services::point_service::tests::spend_plan_handles_zero_and_empty_lots | plan_spend | 境界値 | 残高0のロットを飛ばして (2, 50) を使い、使用0では空計画、負数は None になること | Pass |
| UT-220 | services::point_service::tests::spend_plan_refuses_when_lots_cannot_cover | plan_spend | 異常系 | ロット合計で賄えない場合は None になること | Pass |
| UT-221 | services::point_service::tests::spend_plan_takes_fifo_across_lots | plan_spend | 正常系 | 有効期限の古い順に (1, 300)、(2, 400) と配分され、全額では3ロットを使うこと | Pass |

### app / services::product_service

| 試験ID | テスト名（フルパス） | 対象（関数・対象） | 試験観点 | 期待結果 | 結果 |
|---|---|---|---|---|---|
| UT-222 | services::product_service::tests::merge_caps_at_the_limit | ピン留めマージ | 境界値 | マージ結果が上限の2件に制限されること | Pass |
| UT-223 | services::product_service::tests::merge_puts_pins_first_and_fills_the_rest | ピン留めマージ | 正常系 | ピン留めが先に並び [3, 1, 2, 4] になること | Pass |
| UT-224 | services::product_service::tests::merge_with_no_pins_is_the_automatic_row | ピン留めマージ | 正常系 | ピン留めなしでは自動の並び [7, 8] そのままになること | Pass |

### app / services::review_service

| 試験ID | テスト名（フルパス） | 対象（関数・対象） | 試験観点 | 期待結果 | 結果 |
|---|---|---|---|---|---|
| UT-225 | services::review_service::tests::half_step_ratings_pass_and_the_comment_is_optional | レビュー検証 | 正常系 | 4.5 と 0.5 の評価が有効で、コメントは空でもよいこと | Pass |
| UT-226 | services::review_service::tests::missing_rating_reports_and_a_long_comment_still_can | レビュー検証 | 異常系・境界値 | 未選択は「評価を選択してください」、コメント1,000字は可で超過は文字数エラーになること | Pass |
| UT-227 | services::review_service::tests::off_step_and_out_of_range_ratings_get_the_step_message | レビュー検証 | 異常系 | 0.5刻み外と範囲外は刻みを指定するメッセージになり、正しい値は通過すること | Pass |

### app / services::search_service

| 試験ID | テスト名（フルパス） | 対象（関数・対象） | 試験観点 | 期待結果 | 結果 |
|---|---|---|---|---|---|
| UT-228 | services::search_service::tests::fullwidth_ascii_becomes_halfwidth | normalize_query_width | 変換 | 全角英数が半角化されること（Ｔシャツ→Tシャツ、１００％→100%） | Pass |
| UT-229 | services::search_service::tests::halfwidth_katakana_becomes_fullwidth | normalize_query_width | 変換 | 半角カタカナが全角化されること（ｽﾃｯｶｰ→ステッカー） | Pass |
| UT-230 | services::search_service::tests::like_wildcards_in_input_are_escaped | normalize_patterns | 検証・サニタイズ | LIKE の % と _ がエスケープされてパターン化されること | Pass |
| UT-231 | services::search_service::tests::patterns_use_the_normalized_width | normalize_patterns / suggest_pattern | 変換 | 正規化後の文字幅でパターンが生成されること | Pass |
| UT-232 | services::search_service::tests::price_range_bounds_parse_and_validate | 価格範囲の解釈 | 正常系・異常系 | 未指定は None、範囲指定は Some、片側の不正と逆転した範囲はエラーフラグが立つこと | Pass |
| UT-233 | services::search_service::tests::sound_marks_merge_into_the_preceding_kana | normalize_query_width | 変換 | 濁点と半濁点が前のカナに結合されること（ガーデン、パン、ヴァ） | Pass |
| UT-234 | services::search_service::tests::suggest_needs_two_characters | suggest_pattern | 境界値 | 1文字は None、前後空白を除去した2文字は Some になること | Pass |
| UT-235 | services::search_service::tests::term_count_is_capped | normalize_patterns | 境界値 | 検索語が5語に頭打ちになること | Pass |
| UT-236 | services::search_service::tests::terms_become_contains_patterns | normalize_patterns | 変換 | 各語が %語% の部分一致パターンになること | Pass |
| UT-237 | services::search_service::tests::unknown_sort_falls_back_to_recommended | parse_sort | 異常系 | 未知のソートは recommended に落ち、price_asc は保持され、未指定は recommended になること | Pass |

### app / services::sitemap_service

| 試験ID | テスト名（フルパス） | 対象（関数・対象） | 試験観点 | 期待結果 | 結果 |
|---|---|---|---|---|---|
| UT-302 | services::sitemap_service::tests::escapes_all_five_xml_specials | services::sitemap_service | （未記入） | （未記入） | 未実施 |
| UT-303 | services::sitemap_service::tests::render_escapes_specials_in_locations | services::sitemap_service | （未記入） | （未記入） | 未実施 |
| UT-304 | services::sitemap_service::tests::render_omits_lastmod_when_absent | services::sitemap_service | （未記入） | （未記入） | 未実施 |
| UT-305 | services::sitemap_service::tests::render_trims_base_slash_and_formats_lastmod | services::sitemap_service | （未記入） | （未記入） | 未実施 |
| UT-306 | services::sitemap_service::tests::robots_disallows_private_surfaces_and_names_sitemap | services::sitemap_service | （未記入） | （未記入） | 未実施 |

### app / services::sns_service

| 試験ID | テスト名（フルパス） | 対象（関数・対象） | 試験観点 | 期待結果 | 結果 |
|---|---|---|---|---|---|
| UT-238 | services::sns_service::tests::base64url_uses_the_url_safe_alphabet_without_padding | base64url | 変換 | URL 安全な文字でパディングなしになること（空は空、hello は aGVsbG8） | Pass |
| UT-239 | services::sns_service::tests::pkce_challenge_matches_the_rfc_7636_vector | pkce_challenge | 検証・標準準拠 | RFC 7636 のテストベクトルと一致すること | Pass |
| UT-240 | services::sns_service::tests::url_encode_keeps_unreserved_and_escapes_the_rest | url_encode | 変換 | 予約文字はエスケープされ、非予約文字は保持されること | Pass |

### app / services::stripe_client

| 試験ID | テスト名（フルパス） | 対象（関数・対象） | 試験観点 | 期待結果 | 結果 |
|---|---|---|---|---|---|
| UT-241 | services::stripe_client::tests::konbini_intent_carries_voucher_details | Stripe 支払オブジェクト変換（コンビニ） | 変換 | requires_action になり、案内 URL と支払期限が付き、captured_at はないこと | Pass |
| UT-242 | services::stripe_client::tests::refunded_total_comes_from_the_charge | Stripe 支払オブジェクト変換 | 金額計算 | 返金累計がチャージ由来の3,530円になること | Pass |
| UT-243 | services::stripe_client::tests::session_form_charges_plain_yen_never_minor_units | Stripe セッション作成フォーム | 金額計算・検証 | unit_amount が 3530（円のまま、×100しない）、currency が jpy、card 指定、success_url に {CHECKOUT_SESSION_ID}、client_reference_id とメタデータに注文番号が入ること | Pass |
| UT-244 | services::stripe_client::tests::succeeded_intent_translates_with_charge_truth | Stripe 支払オブジェクト変換 | 変換 | id が pi_123、succeeded、3,530円、JPY、返金0、captured_at あり、セッションが cs_123 であること | Pass |

### app / services::totp

| 試験ID | テスト名（フルパス） | 対象（関数・対象） | 試験観点 | 期待結果 | 結果 |
|---|---|---|---|---|---|
| UT-245 | services::totp::tests::base32_matches_rfc4648_vectors | base32_encode | 検証・標準準拠 | RFC 4648 のベクトルと一致すること（f→MY、foobar→MZXW6YTBOI、数字列も一致） | Pass |
| UT-246 | services::totp::tests::base32_round_trips_and_tolerates_paste_noise | base32_decode | 変換・異常系 | 往復で復元でき、小文字・空白・パディングを許容し、不正文字（! と 1）は None になること | Pass |
| UT-247 | services::totp::tests::generated_secrets_are_160_bit_base32 | generate_secret | 検証 | 32文字（160bit）で復号すると20バイトになり、生成ごとに異なること | Pass |
| UT-248 | services::totp::tests::otpauth_uri_encodes_the_account | otpauth URI 生成 | 変換 | アカウントが URL エンコードされ、issuer、algorithm、digits、period を含むこと | Pass |
| UT-249 | services::totp::tests::qr_svg_renders | QR SVG 生成 | 正常系 | SVG 文字列が生成されること | Pass |
| UT-250 | services::totp::tests::recovery_codes_normalise_and_validate | リカバリーコード | 変換・異常系 | 11文字形式で、正規化により小文字入力を受理し、6桁と不正文字は None、生成数が RECOVERY_CODE_COUNT であること | Pass |
| UT-251 | services::totp::tests::rfc6238_sha1_vectors_truncated_to_six_digits | code_at / verify（TOTP） | 検証・標準準拠 | RFC 6238 の SHA1 ベクトルを6桁に切り詰めた値と一致し、verify も成功すること | Pass |
| UT-252 | services::totp::tests::verify_accepts_neighbouring_steps_only | verify（TOTP） | 境界値 | 前後1ステップは受理し、それ以外（+31秒、+61秒、桁数違い、非数字、+3600秒）は拒否すること | Pass |
| UT-253 | services::totp::tests::verify_step_reports_the_matched_counter | verify_step | 変換 | 一致したカウンタを返し（59秒と89秒で1）、範囲外と非数字は None になること | Pass |

### app / session

| 試験ID | テスト名（フルパス） | 対象（関数・対象） | 試験観点 | 期待結果 | 結果 |
|---|---|---|---|---|---|
| UT-254 | session::tests::constant_time_compare | constant_time_eq | 検証 | 同値は true、1字違いと長さ違いは false、空同士は true になること | Pass |
| UT-255 | session::tests::credential_fingerprint_depends_on_every_field | derive_credential_fingerprint | 検証 | ハッシュ・メール・シークレットのいずれの変更でも指紋が変わり、字段境界を跨ぐ同一化がなく、同入力では同一になること | Pass |

### app / util::asset_fingerprint

| 試験ID | テスト名（フルパス） | 対象（関数・対象） | 試験観点 | 期待結果 | 結果 |
|---|---|---|---|---|---|
| UT-256 | util::asset_fingerprint::tests::fingerprint_changes_when_contents_or_files_change | fingerprint_of | 検証 | 内容とファイル集合の変更で指紋が変わること | Pass |
| UT-257 | util::asset_fingerprint::tests::fingerprint_is_sixteen_lowercase_hex_characters | fingerprint_of | 検証 | 16桁の小文字16進文字列であること | Pass |
| UT-258 | util::asset_fingerprint::tests::fingerprint_is_stable_for_identical_contents | fingerprint_of | 冪等性 | 同内容なら別ディレクトリでも同一の指紋になること | Pass |

### app / util::csv_transfer

| 試験ID | テスト名（フルパス） | 対象（関数・対象） | 試験観点 | 期待結果 | 結果 |
|---|---|---|---|---|---|
| UT-259 | util::csv_transfer::tests::decodes_shift_jis | decode_bytes | 変換 | Shift_JIS のバイト列が「商品名、価格」に復号されること | Pass |
| UT-260 | util::csv_transfer::tests::decodes_utf8_with_and_without_bom | decode_bytes | 変換 | BOM の有無にかかわらず UTF-8 が復号されること | Pass |
| UT-261 | util::csv_transfer::tests::formula_leading_cells_are_quoted | escape_formula | 検証・CSVインジェクション対策 | =、+、-、@ で始まるセルに ' が前置され、通常文字列・空・途中の = は不変であること | Pass |
| UT-262 | util::csv_transfer::tests::parses_quoted_commas_and_embedded_newlines | CSV 読み取り | 変換 | 引用内のカンマと改行が1セルとして解釈され、行番号が2になること | Pass |
| UT-263 | util::csv_transfer::tests::rejects_a_mismatched_header | CSV 読み取り | 異常系 | ヘッダ不一致は1行目のエラーになること | Pass |
| UT-264 | util::csv_transfer::tests::write_csv_quotes_formula_cells | CSV 書き出し | 検証 | 書き出し時も数式セルが引用されること | Pass |
| UT-265 | util::csv_transfer::tests::writes_bom_and_round_trips | CSV 書き出し・読み取り | 変換 | BOM 付きで書き出され、読み戻しで値が往復すること | Pass |

### app / util::hmac_util

| 試験ID | テスト名（フルパス） | 対象（関数・対象） | 試験観点 | 期待結果 | 結果 |
|---|---|---|---|---|---|
| UT-266 | util::hmac_util::tests::parts_concatenate_in_order | hmac_sha256_hex | 変換 | 分割した入力が連結順どおりに全体の HMAC と一致すること | Pass |
| UT-267 | util::hmac_util::tests::rfc4231_test_case_1 | hmac_sha256_hex | 検証・標準準拠 | RFC 4231 テストケース1の既知値と一致すること | Pass |
| UT-268 | util::hmac_util::tests::verify_accepts_only_the_exact_bytes | verify_hmac_sha256_hex | 検証 | 正しい署名のみ true で、再エンコード、別キー、非 hex、空はすべて false になること | Pass |

### app / util::html_sanitize

| 試験ID | テスト名（フルパス） | 対象（関数・対象） | 試験観点 | 期待結果 | 結果 |
|---|---|---|---|---|---|
| UT-269 | util::html_sanitize::tests::event_handler_attributes_are_stripped | HTML サニタイズ | 検証・サニタイズ | onclick 属性が除去され、本文は保持されること | Pass |
| UT-270 | util::html_sanitize::tests::form_is_unwrapped | HTML サニタイズ | 変換 | form タグが外され、中身は保持されること | Pass |
| UT-271 | util::html_sanitize::tests::https_link_survives | HTML サニタイズ | 正常系 | https リンクが保持されること | Pass |
| UT-272 | util::html_sanitize::tests::javascript_href_is_neutralised | HTML サニタイズ | 異常系 | javascript: リンクが無効化され、リンク文字は保持されること | Pass |
| UT-273 | util::html_sanitize::tests::meta_refresh_is_stripped | HTML サニタイズ | 異常系 | meta refresh が除去され、本文は保持されること | Pass |
| UT-274 | util::html_sanitize::tests::scanner_accepts_allowed_destinations | find_invalid_url_html | 正常系 | 許可先のみの文書では None になること | Pass |
| UT-275 | util::html_sanitize::tests::scanner_ignores_lookalike_attributes | find_invalid_url_html | 異常系 | 類似した属性名には反応しないこと | Pass |
| UT-276 | util::html_sanitize::tests::scanner_names_a_javascript_href | find_invalid_url_html | 異常系 | 違反した URL（javascript:alert(1)）をそのまま返すこと | Pass |
| UT-277 | util::html_sanitize::tests::scanner_names_a_scheme_relative_href | find_invalid_url_html | 異常系 | スキーム相対 URL（//evil.example/x）をそのまま返すこと | Pass |
| UT-278 | util::html_sanitize::tests::scheme_relative_href_does_not_survive | HTML サニタイズ | 異常系 | //evil.example が出力に残らないこと | Pass |
| UT-279 | util::html_sanitize::tests::script_tag_and_its_code_are_gone | HTML サニタイズ | 異常系 | script タグと中身が除去され、<p>本文</p> は保持されること | Pass |
| UT-280 | util::html_sanitize::tests::style_with_url_is_dropped_whole | HTML サニタイズ | 異常系 | url() を含む style は丸ごと除去され、本文は保持されること | Pass |
| UT-281 | util::html_sanitize::tests::styled_div_keeps_class_and_style | HTML サニタイズ | 正常系 | class と安全な style は保持されること | Pass |
| UT-282 | util::html_sanitize::tests::table_and_colspan_survive | HTML サニタイズ | 正常系 | table と colspan が保持されること | Pass |
| UT-283 | util::html_sanitize::tests::uploaded_image_path_survives | HTML サニタイズ | 正常系 | uploads 画像の src と alt が保持されること | Pass |

### app / util::image_paths

| 試験ID | テスト名（フルパス） | 対象（関数・対象） | 試験観点 | 期待結果 | 結果 |
|---|---|---|---|---|---|
| UT-284 | util::image_paths::tests::non_derivable_paths_pass_through | thumb_path / medium_path | 異常系 | GIF、uploads 外、拡張子なしのパスは素通しされること | Pass |
| UT-285 | util::image_paths::tests::upload_paths_map_to_derivatives | thumb_path / medium_path | 変換 | _thumb と _medium の派生パスが生成され、png と webp は _thumb.webp に、大文字拡張子も正規化されること | Pass |

### app / util::markdown

| 試験ID | テスト名（フルパス） | 対象（関数・対象） | 試験観点 | 期待結果 | 結果 |
|---|---|---|---|---|---|
| UT-286 | util::markdown::tests::allowlist_accepts_the_documented_shapes | url_allowed | 正常系 | https、http、mailto、サイト内パス、#、空が許可されること | Pass |
| UT-287 | util::markdown::tests::allowlist_refuses_script_and_smuggled_schemes | url_allowed | 異常系 | javascript（大文字混在とタブ混入）、data、//、相対パスが拒否されること | Pass |
| UT-288 | util::markdown::tests::disallowed_link_is_neutralised_at_render | Markdown レンダリング | 異常系 | 危険なリンクはレンダリング時に無効化され、リンク文字は残ること | Pass |
| UT-289 | util::markdown::tests::find_invalid_url_names_the_offender | find_invalid_url | 異常系 | 違反した URL を特定でき、正常なリンクのみでは None になること | Pass |
| UT-290 | util::markdown::tests::headings_lists_and_links_render | Markdown レンダリング | 変換 | 見出し、リスト、リンクが HTML 化されること | Pass |
| UT-291 | util::markdown::tests::raw_html_renders_as_text | Markdown レンダリング | 検証・サニタイズ | 生 HTML はエスケープされ、スクリプトとして実行されないこと | Pass |

### app / validation

| 試験ID | テスト名（フルパス） | 対象（関数・対象） | 試験観点 | 期待結果 | 結果 |
|---|---|---|---|---|---|
| UT-292 | validation::tests::email_shape_is_the_strict_rule_everywhere | is_valid_email | 検証 | 正常な形式は可、ドメイン不完全、@ の欠落と重複、空白入りは不可であること | Pass |
| UT-293 | validation::tests::katakana_names | is_katakana | 検証 | 全角カタカナは可、ひらがなと空は不可であること | Pass |
| UT-294 | validation::tests::link_url_rule_is_the_strict_section_7_shape | link_url_allowed | 検証 | サイト内、https、# は可、空、http、//、javascript、data、空白入り、タブ混入は不可であること | Pass |
| UT-295 | validation::tests::phone_accepts_ten_or_eleven_digits | phone_digits | 変換・境界値 | ハイフンを除去して10桁と11桁を受理し、英字混入と短すぎる番号は None になること | Pass |
| UT-296 | validation::tests::postal_code_accepts_hyphens_only_as_separators | postal_code_digits | 変換・異常系 | 7桁と3-4ハイフン形式を受理し、英字、空白、桁不足、空は None になること | Pass |
| UT-297 | validation::tests::slug_shape_is_the_url_safe_subset | is_valid_slug | 検証・境界値 | 小文字英数とハイフンが可、空、大文字、両端ハイフン、日本語、空白、51字は不可であること | Pass |

### configuration / config

| 試験ID | テスト名（フルパス） | 対象（関数・対象） | 試験観点 | 期待結果 | 結果 |
|---|---|---|---|---|---|
| UT-298 | config::tests::parses_each_unit | parse_ttl | 変換 | 45s、30m、8h、14d がそれぞれ秒数に変換されること | Pass |
| UT-299 | config::tests::rejects_anything_else | parse_ttl | 異常系 | 空、単位なし、未知の単位、0、負数、小数はすべて None になること | Pass |

### app / models::variant

| 試験ID | テスト名（フルパス） | 対象（関数・対象） | 試験観点 | 期待結果 | 結果 |
|---|---|---|---|---|---|
| UT-300 | app/src/models/variant.rs - models::variant::cartesian (line 119) | cartesian（doc テスト） | 変換・doc例 | doc コメントの例どおり、軸ごとの直積が [[1, 3], [2, 3]] になり、空の軸リストでは空の組合せ1つになること | Pass |
| UT-301 | app/src/models/variant.rs - models::variant::signature_for (line 98) | signature_for（doc テスト） | 変換・doc例 | doc コメントの例どおり、[3, 7] は "3-7"、空は "" になること | Pass |
<!-- INVENTORY:END -->

## 実施記録

`run` のたびに1行追加される。

| 実施日 | 対象件数 | 合格 | 不合格 | 備考 |
|---|---|---|---|---|
<!-- EXECUTION:BEGIN -->
| 2026-11-22 | 301 | 301 | 0 | 全件実施 |
| 2026-11-22 | 301 | 296 | 1 | 要確認: listed=301 executed=297 |
<!-- EXECUTION:END -->
