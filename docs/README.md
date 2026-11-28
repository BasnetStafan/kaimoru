# ドキュメント索引

このディレクトリは、本プロジェクトの設計ドキュメントの入口です。
初めての方は上から順に読むと、環境構築から開発の進め方、仕様、
運用までを追える構成になっています。

## はじめに

- [README（日本語）](../README.md)：プロジェクトの概要、機能、
  設計上の判断、技術スタックの全体像です。
- [README（English）](../README.en.md)：英語版の README です。

## 環境構築

- [前提環境と起動手順](../README.md#前提環境)：必要なツール
  （Docker、Rust 1.99.1、cargo-audit）と、`.env.example` から
  `.env` を作って `./run.sh` で起動するまでの手順です。

## 開発の進め方

- [コントリビューションガイド](../CONTRIBUTING.md)：ハウスルール
  （整形・静的解析・ドキュメントコメントの規約）と、変更を出す
  前の確認手順です。
- [テストガイド](development/TESTING.md)：品質ゲート
  （`scripts/check.sh`）と E2E スイート群の戦略、前提条件、
  ケース ID の規約、スイートごとの一覧です。
- [単体試験仕様書](development/UNIT_TEST_SPEC.md)：単体テスト
  全件（301件）の一覧と実施記録です。保守スクリプト
  （`scripts/ops/unit_test_spec.sh`）でコードとの一致を保ちます。
- [Rust ベストプラクティス](development/RUST_BEST_PRACTICES.md)：
  このコードベースで守る Rust の書き方の基準です。

## アーキテクチャ／仕様

- [アーキテクチャ](development/ARCHITECTURE.md)：システムの
  全体像、レイヤー構成、決済とバックグラウンドタスクの流れ、
  主要な設計判断と「どこを触るか」の早見表です。コードを
  読み始める前にまずここを参照してください。
- [画面・機能仕様書](specs/EC_PAGE_FUNCTION_SPEC.pdf)（PDF）：
  全画面の機能仕様です。画面一覧・画面遷移と、画面ごとの
  目的・操作・規則をまとめています。
- [詳細仕様書](specs/EC_DETAILED_SPEC.pdf)（PDF）：実装の
  詳細仕様です。データベース全表、金額計算の順序、
  バックグラウンドワーカー、決済・ポイント・ステップメールの
  内部動作をまとめています。
- [楽観的ロック仕様](specs/OPTIMISTIC_LOCKING_SPEC.md)：管理画面の
  レコード編集における競合検出の設計記録です。

## 運用

- 起動と停止：`./run.sh`（起動）、`./stop.sh`（停止。`db` 引数で
  DB コンテナも停止）です。詳細はルート README の
  「環境構築と起動」を参照してください。
- [セキュリティポリシー](../SECURITY.md)：脆弱性の報告窓口です。
- ライセンス：[LICENSE-MIT](../LICENSE-MIT) と
  [LICENSE-APACHE](../LICENSE-APACHE) のデュアルライセンスです。
  条件はルート README の「ライセンス」を参照してください。
