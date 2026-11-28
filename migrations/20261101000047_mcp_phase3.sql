-- MCP Phase 3 (docs/development/MCP_SERVER.md): approval-
-- gated draft orders. An orders:write key freezes a cart
-- into a draft; a person approves and pays in the browser.
--
-- The scope CHECK widens to the two order scopes (keys stay
-- single-scope; an orders:write key may also run the cart
-- tools so it can work the whole flow alone).
--
-- mcp_order_drafts is the MCP analogue of agent_approvals:
-- the cart's fingerprint and display snapshot at creation,
-- the delivery address the client supplied, and a single-
-- use approval token stored only as its HMAC-SHA-256
-- digest (the Phase 2 claim discipline). Status moves
-- pending -> consumed (the pay click placed the order),
-- expired (the TTL passed), or void (the cart changed
-- under it). The idempotency key makes a retried create
-- return the same draft instead of a second one.
--
-- A backup was taken and restore-proven before this
-- migration was written
-- (~/backups/agentic-commerce/agentic_commerce-20261128-065407.sql).
ALTER TABLE api_keys DROP CONSTRAINT api_keys_scope_check;
ALTER TABLE api_keys ADD CONSTRAINT api_keys_scope_check
    CHECK (scope IN ('catalog:read', 'cart:write', 'orders:read', 'orders:write'));

CREATE TABLE mcp_order_drafts (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    api_key_id BIGINT NOT NULL REFERENCES api_keys(id),
    cart_id TEXT NOT NULL,
    idempotency_key TEXT NOT NULL,
    UNIQUE (api_key_id, idempotency_key),
    address JSONB NOT NULL,
    fingerprint TEXT NOT NULL,
    snapshot JSONB NOT NULL,
    approval_token_hash TEXT NOT NULL UNIQUE,
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'consumed', 'expired', 'void')),
    order_id BIGINT REFERENCES orders(id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at TIMESTAMPTZ NOT NULL
);

CREATE TRIGGER trg_mcp_order_drafts_set_updated_at
    BEFORE UPDATE ON mcp_order_drafts
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();
