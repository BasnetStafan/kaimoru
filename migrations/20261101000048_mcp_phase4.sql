-- MCP Phase 4 (docs/development/MCP_SERVER.md): OAuth 2.1
-- resource server. Auth0 (the external authorization
-- server) issues access tokens; a cart or draft created
-- with one belongs to an OAuth principal, not an API key.
--
-- Both MCP ownership tables gain the principal model:
-- `principal_key` is the one ownership value every check
-- compares ('key:<api_key_id>' or 'oauth:<subject>'), and
-- `oauth_subject` records which subject a row belongs to
-- when it was not a key that created it. `api_key_id`
-- becomes nullable provenance: exactly one of the two
-- creator columns is set, which the CHECK states. Existing
-- rows are all key-created, so the backfill is total.
--
-- The draft idempotency uniqueness moves from the key to
-- the principal: one principal, one draft per idempotency
-- key, whichever credential kind the principal used.
--
-- A backup was taken and restore-proven before this
-- migration was written
-- (~/backups/agentic-commerce/agentic_commerce-20261128-075654.sql).
ALTER TABLE mcp_carts ADD COLUMN principal_key TEXT;
ALTER TABLE mcp_carts ADD COLUMN oauth_subject TEXT;
UPDATE mcp_carts SET principal_key = 'key:' || api_key_id;
ALTER TABLE mcp_carts ALTER COLUMN principal_key SET NOT NULL;
ALTER TABLE mcp_carts ALTER COLUMN api_key_id DROP NOT NULL;
ALTER TABLE mcp_carts ADD CONSTRAINT mcp_carts_creator_check
    CHECK (api_key_id IS NOT NULL OR oauth_subject IS NOT NULL);

ALTER TABLE mcp_order_drafts ADD COLUMN principal_key TEXT;
ALTER TABLE mcp_order_drafts ADD COLUMN oauth_subject TEXT;
UPDATE mcp_order_drafts SET principal_key = 'key:' || api_key_id;
ALTER TABLE mcp_order_drafts ALTER COLUMN principal_key SET NOT NULL;
ALTER TABLE mcp_order_drafts ALTER COLUMN api_key_id DROP NOT NULL;
ALTER TABLE mcp_order_drafts ADD CONSTRAINT mcp_order_drafts_creator_check
    CHECK (api_key_id IS NOT NULL OR oauth_subject IS NOT NULL);
ALTER TABLE mcp_order_drafts
    DROP CONSTRAINT mcp_order_drafts_api_key_id_idempotency_key_key;
ALTER TABLE mcp_order_drafts ADD CONSTRAINT mcp_order_drafts_principal_idempotency_key
    UNIQUE (principal_key, idempotency_key);
