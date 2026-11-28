-- Step-mail (scenario mail), EC V2 #2: automated mail
-- sequences on the existing mail pipeline
-- (internal step-mail design record). A scenario is a chain of
-- steps bound to a trigger; an enrollment is one member's
-- run through it. Sends reuse the campaign pipeline's
-- consent check, renderer and transport — these tables
-- carry only the scheduling state and the send log.
--
-- Owner decision (2026-11-20): build welcome + review
-- request only; no post-purchase or win-back seeds, and
-- no inactivity trigger.

CREATE TABLE mail_scenarios (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    -- Stable machine key the code enrolls by.
    key TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    -- signup | order_delivered
    trigger_kind TEXT NOT NULL,
    enabled BOOLEAN NOT NULL DEFAULT true,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE mail_scenario_steps (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    scenario_id BIGINT NOT NULL REFERENCES mail_scenarios(id) ON DELETE CASCADE,
    step_no INTEGER NOT NULL,
    -- Days after the previous step (step 1: after enrollment).
    delay_days INTEGER NOT NULL DEFAULT 0,
    subject TEXT NOT NULL,
    body_html TEXT NOT NULL,
    body_text TEXT NOT NULL,
    UNIQUE (scenario_id, step_no)
);

CREATE TABLE mail_scenario_enrollments (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    scenario_id BIGINT NOT NULL REFERENCES mail_scenarios(id) ON DELETE CASCADE,
    user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    -- Set only for order-triggered scenarios.
    order_id BIGINT REFERENCES orders(id) ON DELETE CASCADE,
    -- The step due next (1-based).
    current_step INTEGER NOT NULL DEFAULT 1,
    -- Consecutive send failures on the current step.
    attempts INTEGER NOT NULL DEFAULT 0,
    next_due_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- active | completed | stopped | failed
    status TEXT NOT NULL DEFAULT 'active',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- One run per member per scenario; order-triggered
-- scenarios run once per order instead.
CREATE UNIQUE INDEX mail_scenario_enrollments_member_once
    ON mail_scenario_enrollments (scenario_id, user_id)
    WHERE order_id IS NULL;
CREATE UNIQUE INDEX mail_scenario_enrollments_order_once
    ON mail_scenario_enrollments (scenario_id, order_id)
    WHERE order_id IS NOT NULL;
CREATE INDEX mail_scenario_enrollments_due
    ON mail_scenario_enrollments (next_due_at)
    WHERE status = 'active';

CREATE TABLE mail_scenario_sends (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    enrollment_id BIGINT NOT NULL REFERENCES mail_scenario_enrollments(id) ON DELETE CASCADE,
    scenario_id BIGINT NOT NULL REFERENCES mail_scenarios(id) ON DELETE CASCADE,
    user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    step_no INTEGER NOT NULL,
    -- sent | skipped_consent | failed
    status TEXT NOT NULL,
    error TEXT,
    sent_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

INSERT INTO mail_scenarios (key, name, description, trigger_kind, enabled) VALUES
    ('welcome', 'ウェルカムシリーズ', '会員登録から0日・3日・7日に送る3通のステップメールです。', 'signup', true),
    ('review_request', 'レビュー依頼', '注文の配送完了から3日後に、レビュー投稿をお願いするメールです。', 'order_delivered', true)
ON CONFLICT (key) DO NOTHING;

INSERT INTO mail_scenario_steps (scenario_id, step_no, delay_days, subject, body_html, body_text)
SELECT id, 1, 0,
    '会員登録ありがとうございます',
    '<p>{name} 様</p><p>このたびは会員登録いただき、ありがとうございます。</p><p>当店では、Rustエコシステムのファングッズを中心に取り揃えています。新作や人気商品のご案内を、メールマガジンでお届けします。</p><p>初回のお買い物には、500円OFFクーポン「WELCOME500」をご利用いただけます（お一人様1回限り）。ぜひご活用ください。</p>',
    '{name} 様\n\nこのたびは会員登録いただき、ありがとうございます。\n\n当店では、Rustエコシステムのファングッズを中心に取り揃えています。新作や人気商品のご案内を、メールマガジンでお届けします。\n\n初回のお買い物には、500円OFFクーポン「WELCOME500」をご利用いただけます（お一人様1回限り）。ぜひご活用ください。'
FROM mail_scenarios WHERE key = 'welcome'
ON CONFLICT (scenario_id, step_no) DO NOTHING;

INSERT INTO mail_scenario_steps (scenario_id, step_no, delay_days, subject, body_html, body_text)
SELECT id, 2, 3,
    '人気商品のご紹介',
    '<p>{name} 様</p><p>当店で人気の商品をご紹介します。</p>{{product:4}}{{product:5}}<p>気になる商品がございましたら、ぜひ商品ページで詳細をご覧ください。</p>',
    '{name} 様\n\n当店で人気の商品をご紹介します。\n\n{{product:4}}\n{{product:5}}\n\n気になる商品がございましたら、ぜひ商品ページで詳細をご覧ください。'
FROM mail_scenarios WHERE key = 'welcome'
ON CONFLICT (scenario_id, step_no) DO NOTHING;

INSERT INTO mail_scenario_steps (scenario_id, step_no, delay_days, subject, body_html, body_text)
SELECT id, 3, 4,
    '初めてのお買い物はお済みですか',
    '<p>{name} 様</p><p>会員登録から1週間が経ちました。初めてのお買い物はお済みですか。</p><p>初回のお買い物には、500円OFFクーポン「WELCOME500」がご利用いただけます（お一人様1回限り）。</p>{{product:5}}<p>ご不明な点がございましたら、お問い合わせフォームからお気軽にご連絡ください。</p>',
    '{name} 様\n\n会員登録から1週間が経ちました。初めてのお買い物はお済みですか。\n\n初回のお買い物には、500円OFFクーポン「WELCOME500」がご利用いただけます（お一人様1回限り）。\n\n{{product:5}}\n\nご不明な点がございましたら、お問い合わせフォームからお気軽にご連絡ください。'
FROM mail_scenarios WHERE key = 'welcome'
ON CONFLICT (scenario_id, step_no) DO NOTHING;

INSERT INTO mail_scenario_steps (scenario_id, step_no, delay_days, subject, body_html, body_text)
SELECT id, 1, 3,
    '商品のレビューをお聞かせください',
    '<p>{name} 様</p><p>先日はご注文いただき、ありがとうございました。お届けした商品はいかがでしたでしょうか。</p><p>よろしければ、注文詳細ページからレビューをお聞かせください。今後の商品選びや改善の参考にさせていただきます。</p><p><a href="{order_url}">注文詳細ページを開く</a></p>',
    '{name} 様\n\n先日はご注文いただき、ありがとうございました。お届けした商品はいかがでしたでしょうか。\n\nよろしければ、注文詳細ページからレビューをお聞かせください。今後の商品選びや改善の参考にさせていただきます。\n\n注文詳細ページ: {order_url}'
FROM mail_scenarios WHERE key = 'review_request'
ON CONFLICT (scenario_id, step_no) DO NOTHING;
