# MCP server (Phase 1: read-only catalog; Phase 2: carts)

The store speaks the Model Context Protocol (MCP) so external
AI clients — Claude, Cursor, VS Code, and other MCP hosts —
can use the catalog as tools instead of scraping pages. The
server is built on the official Rust SDK (`rmcp`), mounted
in-process on the existing Axum router at `/mcp`, and serves
the current protocol (2026-07-28, stateless) plus the
2025-era compatibility mode.

Phase 1 is **read-only and anonymous by design**. Every tool
and resource serves exactly what the storefront and the agent
API already expose without a login. There is no member data,
no order data, and no write of any kind on this surface. The
handlers call the same catalog service as the agent API, so
an AI client and a person browsing the store never see
different facts.

Phase 2 adds **cart tools behind bearer API keys**: a client
with a `cart:write` key can build a cart and hand it to a
person, who completes checkout in their own browser. The
server still never places an order and never sees payment
details.

## Enabling it

Off by default. Set `mcp_enabled = true` in `config.toml`
and restart; with the flag off the `/mcp` path does not
exist. (The flag is a boot-shape setting like the payment
and SNS-login switches: secrets stay in the environment,
live business knobs stay in the database.)

## Tools

| Tool | Arguments | Returns |
| --- | --- | --- |
| `search_products` | `query?`, `category?`, `min_price_yen?`, `max_price_yen?`, `sort?`, `in_stock_only?`, `page?` | One page of products |
| `get_product` | `id` | One product, with its variants |
| `list_categories` | — | Every category (id, name, slug, parent) |
| `get_store_info` | — | Shop identity, contact, shipping fee and free-shipping threshold |

- Prices are integer yen, tax included. Search runs the
  storefront search's own machinery (normalisation, term
  splitting, the public predicate), so the same words give
  the same products in both places.
- `sort` is one of `recommended`, `newest`, `price_asc`,
  `price_desc`, `rating`. Unknown values are refused at the
  boundary with a protocol error, never silently re-sorted.
- A draft or archived product answers exactly like a missing
  one (`no public product with id N`): the surface cannot be
  used to enumerate unpublished products.
- Every tool declares an input and an output JSON Schema;
  results carry `structuredContent` matching the schema.

## Phase 2: carts and API keys

### API keys

Keys are issued by an operator on the admin page
**Settings → APIキー** (`/admin/settings/api-keys`, settings
permission area). A key looks like
`kmcp_<prefix>_<secret>`: the prefix (12 hex chars) is the
public lookup key, the secret (64 hex chars) is shown
**exactly once**, on the create response — only its Argon2id
hash is stored, the same treatment as a password, so a lost
key is revoked and reissued, never recovered. The list shows
name, prefix, scope, issue and last-used times; revocation
is immediate and permanent, and both issuing and revoking
are written to the admin audit log.

| Scope | Can use |
| --- | --- |
| `catalog:read` | The Phase 1 catalog tools (which are anonymous anyway — the scope exists for the key's own bookkeeping) |
| `cart:write` | The catalog tools plus every cart tool below |

Clients send the key as `Authorization: Bearer <token>` —
the header only, never a query string. Every authentication
failure (missing, malformed, unknown, revoked, wrong scope)
returns the **same** error (`-32001`, "a valid API key with
the cart:write scope is required"), so the endpoint cannot
be used as an oracle for probing keys. Cart calls are
rate-limited per key: **120 calls per 60 seconds**,
in-memory (a restart resets the windows).

### Cart tools

| Tool | Arguments | Returns |
| --- | --- | --- |
| `create_cart` | — | A new empty cart and its `cart_id` handle |
| `get_cart` | `cart_id` | The cart's lines and totals |
| `add_cart_item` | `cart_id`, `product_id`, `variant_id?`, `quantity` | The cart read back after the add |
| `update_cart_item` | `cart_id`, `variant_id`, `quantity` | The cart read back (quantity stock-capped; 0 is refused — use remove) |
| `remove_cart_item` | `cart_id`, `variant_id` | The cart read back |
| `get_checkout_link` | `cart_id` | A single-use browser handoff URL and its expiry |

- A cart is **bound to the key that created it** and the
  binding is checked on every call. A cart that does not
  exist and a cart owned by another key answer identically
  (`no cart with id … for this API key`) — no existence
  oracle across keys.
- The handle is also the cart's session key in the cart
  service, so an MCP cart is mechanically a guest cart:
  same lines table, same stock rules, same totals — the
  numbers match the cart page and the agent API exactly.
  Its lines are cleared by the same abandoned-guest-cart
  sweep after `guest_cart_retention_days`; the small
  binding row itself stays (it records which key created
  the cart).
- Adds run the agent API's own pre-checks: draft or
  archived products answer exactly like missing ones, a
  combination product without `variant_id` is refused,
  and stock is re-validated on every call.

### Checkout handoff

`get_checkout_link` mints a **single-use claim URL**,
`<public base>/cart/claim/<token>`, valid for **30
minutes**. The raw token is never stored — only its
HMAC-SHA-256 digest, keyed by the server secret, under the
same discipline as the password-reset codes. Minting a new
link invalidates the previous one. When a person opens the
link, one transaction moves the cart's lines onto their own
cart (merged into their member cart when logged in, or
their guest cart otherwise, quantities summed on overlap)
and marks the claim used; a second open, an expired link,
and an unknown token all land on the cart page with the
same notice. After the handoff the MCP cart is empty. The
person then checks out and pays on the store site as
usual — no order is ever placed through MCP.

## Resources

| URI | Content |
| --- | --- |
| `store://store/info` | The `get_store_info` payload, as JSON |
| `store://products/{id}` (template) | The `get_product` payload, as JSON |
| `store://pages/{slug}` | A published page's Markdown body (guide, FAQ, terms, privacy, legal, and published custom pages) |

## Connecting a client

Any MCP host that supports streamable HTTP can connect to
`https://<the store>/mcp`. For local development against the
dev app on another machine, tunnel the port so the client
sees a loopback address:

```sh
ssh -L 3000:127.0.0.1:3000 <dev-host>
```

then point the client at `http://127.0.0.1:3000/mcp`
(Claude Desktop / Cursor: a `mcpServers` entry with
`"url": "http://127.0.0.1:3000/mcp"`).

**Allowed hosts:** the transport refuses requests whose
`Host` header is not on its allowlist (loopback by default)
as a DNS-rebinding guard. A public deployment must extend
the allowlist to the store's own host before enabling the
flag there — until then `/mcp` answers 403 to non-loopback
hosts even with the flag on.

## Security posture

- The catalog surface is anonymous, public data only — the
  same data as `GET /api/products`, with the same
  visibility predicate.
- Cart tools require a `cart:write` API key (above): hashed
  at rest, shown once, revocable, with one indistinguishable
  error for every auth failure and a per-key rate limit.
  Read and write credentials are separate tools and scopes,
  never a widening of the catalog tools.
- Service failures surface as a generic protocol error; the
  detail is logged server-side only.
- The surface sits under the maintenance gate and is logged
  on the API surface.

## Testing

- `scripts/tests/test_mcp.sh` — the black-box suite
  (26 checks): both protocol eras, parity with the REST
  catalog, boundary validation, draft invisibility,
  resources, host allowlist. Needs the dev app running with
  `mcp_enabled = true`.
- `scripts/tests/test_mcp_carts.sh` — the Phase 2 suite
  (23 checks): keys minted through the real admin UI flow,
  every auth refusal shape, the no-oracle ownership rule,
  totals against REST ground truth, and the full claim
  handoff (including second-use refusal) with a cookie jar.
- `cargo run --example mcp_client_check` with
  `MCP_TEST_API_KEY` set to a `cart:write` key also runs
  the cart flow (create, add, read back, checkout link) as
  a real rmcp client.
- `scripts/tests/fixtures/mcp_tools_list.json` — the
  checked-in `tools/list` snapshot. A tool name, description
  or schema change fails the suite until the fixture is
  regenerated on purpose (same canonicalisation the suite
  uses: tools sorted by name, keys sorted).
- `cargo run --example mcp_client_check` — an acceptance
  harness that connects as a real rmcp client and calls
  every tool and a resource.

## Later phases (not built)

Phase 3 adds approval-gated draft orders on the existing
agent-approval machinery. It is a separate decision; the
no-agent-orders rule stands until then.
