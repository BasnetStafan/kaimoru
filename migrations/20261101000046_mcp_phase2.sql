-- MCP Phase 2 (docs/development/MCP_SERVER.md): bearer API
-- keys for external MCP clients, and the binding between an
-- MCP cart handle and the key that created it.
--
-- api_keys stores only an Argon2id hash of the key's secret
-- portion (the same treatment as a password); the full token
-- is shown to the operator exactly once, at creation. The
-- prefix is the lookup key and is not a secret. Revocation is
-- a timestamp, never a delete, so a revoked key's carts and
-- audit trail stay explainable.
--
-- mcp_carts binds one cart handle to its owning key. The
-- cart_id IS the cart_items session_id of the MCP cart (the
-- guest-cart shape), so the cart itself needs no new table.
-- The claim columns carry the single-use browser handoff: a
-- hashed claim token, its expiry, and its one use.
--
-- A backup was taken and restore-proven before this migration
-- was written (~/backups/agentic-commerce/agentic_commerce-20261128-062128.sql).
CREATE TABLE api_keys (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    name TEXT NOT NULL,
    prefix TEXT NOT NULL UNIQUE,
    secret_hash TEXT NOT NULL,
    scope TEXT NOT NULL CHECK (scope IN ('catalog:read', 'cart:write')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_used_at TIMESTAMPTZ,
    revoked_at TIMESTAMPTZ
);

CREATE TABLE mcp_carts (
    cart_id TEXT PRIMARY KEY,
    api_key_id BIGINT NOT NULL REFERENCES api_keys(id),
    claim_token_hash TEXT,
    claim_expires_at TIMESTAMPTZ,
    claim_used_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
