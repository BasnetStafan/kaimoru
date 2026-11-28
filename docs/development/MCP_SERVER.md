# MCP server (Phase 1: read-only catalog; Phase 2: carts; Phase 3: draft orders)

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

Phase 2 adds **cart tools behind API keys**: a client
with a `cart:write` key can build a cart and hand it to a
person, who completes checkout in their own browser. The
client never places an order and never sees payment
details.

Phase 3 adds **approval-gated draft orders**: a client
with an `orders:write` key can freeze a cart, with a
delivery address, into a draft and hand the person an
approval URL. The person opens it, checks the frozen
order, picks a payment method and pays in the browser —
that click, and only that click, places the order,
through the same settlement discipline as the in-store
agent's approvals. Building Phase 3 in the development
environment is **not** a go-live decision: the live
store still runs with MCP disabled, and the
no-agent-orders rule stands until a separate go-live
decision lifts it.

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
| `orders:read` | The catalog tools plus reading back this key's own drafts and the orders they placed (`get_order_draft`, `get_order_status`) |
| `orders:write` | Everything `orders:read` and `cart:write` allow, plus freezing a cart into an order draft (`create_order_draft`) |

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
usual — the client itself never places the order.

## Phase 3: order drafts

### Order tools

| Tool | Arguments | Returns |
| --- | --- | --- |
| `create_order_draft` | `cart_id`, `idempotency_key`, `address{email, family_name, given_name, postal_code, prefecture, city, address_line, building?, phone}` | The draft: id, status, approval URL (creating call only), expiry, frozen lines and totals |
| `get_order_draft` | `draft_id` | The draft's status, frozen snapshot, and the placed order's id once consumed |
| `get_order_status` | `order_id` | The order's fulfilment status, grand total, and shipment facts (carrier, tracking number, delivery time) once they exist |

The scope model (J1) keeps keys **single-scope** while
letting one `orders:write` key run the whole flow alone:
the cart tools accept `cart:write` **or** `orders:write`;
`create_order_draft` requires `orders:write` and a cart
bound to the caller's own key; the two read tools accept
`orders:read` or `orders:write`, again bound to the
caller's key. A `cart:write` key can never draft and an
`orders:read` key can never write. Each tool family has
its own single no-oracle auth message, on the Phase 2
terms. Draft creation is rate-limited separately:
**10 drafts per key per hour**.

`get_order_draft` (J2) exists because the plan's tool
list alone left a gap: without it a client could never
learn the `order_id` that `get_order_status` needs.
Orders placed any other way — storefront, agent chat —
are invisible to these tools: a status read resolves
only through a draft row owned by the caller's key, and
a miss answers the same `no order with id … for this
API key` as another key's order.

### Draft lifecycle

`pending → consumed` (the person paid), `pending →
expired` (the window closed), `pending → void` (the
binding broke). A draft is created against a cart with
at least one purchasable line; its **fingerprint** is
the agent approval's own `fingerprint_for` over the
lines and the cart totals — one fingerprint definition
for every approval in the store — and its **snapshot**
is the same approval snapshot shape, so the client and
the approval page read identical figures. The
payment-method-dependent part (the cash-on-delivery
fee) is deliberately not in the frozen totals: it binds
at pay time through `final_fingerprint`, exactly as in
the agent settlement.

The approval token is 32 random bytes, shown once in
the approval URL; only its HMAC-SHA-256 digest is
stored, the claim token's discipline. The window is
the agent approvals' TTL (`agent_approval_ttl_minutes`,
30 minutes in dev).

**Idempotency.** `(api_key_id, idempotency_key)` is
unique. Repeating a create with the same key returns
the existing draft **as-is** — even void, expired or
consumed — and never mints a second one. The approval
URL is returned only by the creating call: the token
is never stored, so a replay cannot reproduce it, and
a client that loses the URL loses the draft (it can
freeze a new one under a new idempotency key once the
cart is rebuilt).

**Void on change, two layers.** Any successful cart
mutation (add, update, remove) eagerly voids the
cart's pending drafts, so a stale draft reads `void`
immediately. Behind that, the pay-time fingerprint
re-check is the hard guarantee: the approval page
recomputes the live fingerprint and refuses — marking
the draft void — when it no longer matches, with the
agent settlement's own refusal wording.

### The approval page

`GET /orders/draft/<token>` is a storefront page in the
checkout's design language. Live and still bound, it
shows the cart's lines, the draft's delivery address,
the totals and the store's payment methods exactly as
checkout offers them; a dead draft shows its state
instead of a pay form. `POST /orders/draft/<token>/pay`
mirrors the agent settlement's sequence step for step:
verify (pending, in window, fingerprint) → bind the
final state → place the guest order through
`order_service::place_order` under the draft address's
identity, recorded as agent-placed → consume the draft
onto the order with a conditional update → follow
checkout's own outcome (hosted methods redirect to the
provider's page; cash on delivery lands on the
order-complete page). Placement re-checks the bound
fingerprint against its locked cart read, so drift
between binding and placement refuses instead of
placing under a stale approval.

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
- Order tools sit behind `orders:read` / `orders:write`
  keys with the same one-answer auth discipline, a separate
  draft-creation rate limit, and key-bound reads: an order
  is visible only to the key whose draft placed it. A draft
  never places anything by itself — placement happens only
  on the approval page, by the person, after the binding
  re-verifies.
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
- `scripts/tests/test_mcp_phase3.sh` — the Phase 3 suite
  (27 checks): keys minted through the admin UI, every
  scope refusal, the J1 flow on one `orders:write` key,
  idempotent creation, the approval page and a COD
  payment end to end, no-oracle reads across keys, the
  void and expiry paths, and the hosted path's dev
  outcome (provider redirect or payment error page).
- `cargo run --example mcp_client_check` with
  `MCP_TEST_API_KEY` set to a `cart:write` key also runs
  the cart flow (create, add, read back, checkout link) as
  a real rmcp client; with an `orders:write` key it also
  drafts the cart and reads the draft back (payment stays
  the person's step, so the harness never pays).
- `scripts/tests/fixtures/mcp_tools_list.json` — the
  checked-in `tools/list` snapshot. A tool name, description
  or schema change fails the suite until the fixture is
  regenerated on purpose (same canonicalisation the suite
  uses: tools sorted by name, keys sorted).
- `cargo run --example mcp_client_check` — an acceptance
  harness that connects as a real rmcp client and calls
  every tool and a resource.

## Later phases (not built)

Phase 4 is OAuth 2.1, if a real deployment ever demands
it: the documented position is API keys only until then.
