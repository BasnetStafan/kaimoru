-- Campaign open/click tracking (Wave 3 C2 completion).
--
-- Tokens are unguessable random ids (32 random bytes as
-- hex, the same shape as unsubscribe tokens): they carry
-- no recipient information and are the lookup key for
-- their row. The open token lives on the delivery; click
-- targets live on per-(delivery, URL) link rows, so the
-- redirect endpoint can only ever send a visitor to a
-- URL that was in that recipient's campaign mail.
ALTER TABLE email_deliveries
    ADD COLUMN open_token TEXT,
    ADD COLUMN open_count INTEGER NOT NULL DEFAULT 0,
    ADD COLUMN opened_at TIMESTAMPTZ;

CREATE UNIQUE INDEX email_deliveries_open_token_key
    ON email_deliveries (open_token) WHERE open_token IS NOT NULL;

CREATE TABLE email_track_links (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    delivery_id BIGINT NOT NULL REFERENCES email_deliveries (id),
    url TEXT NOT NULL,
    token TEXT NOT NULL UNIQUE,
    click_count INTEGER NOT NULL DEFAULT 0,
    first_clicked_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (delivery_id, url)
);

CREATE INDEX email_track_links_delivery_idx ON email_track_links (delivery_id);
