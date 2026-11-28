-- Pass 2 (SEC-5): the last accepted TOTP step counter per
-- member, so an authenticator code cannot be replayed inside
-- its validity window. NULL until the first accepted code.
-- The counter is time-derived (unix time / 30) and only ever
-- grows, so disabling two-factor does not need to reset it.
ALTER TABLE users ADD COLUMN totp_last_step bigint;
