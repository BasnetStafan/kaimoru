# MCP server (Phase 1: read-only catalog)

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

- Anonymous surface, public data only — the same data as
  `GET /api/products`, with the same visibility predicate.
- No credentials exist to leak: Phase 1 has no auth because
  it needs none. Cart and account tools (later phases) will
  require API keys and are separate tools, never a widening
  of these.
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
- `scripts/tests/fixtures/mcp_tools_list.json` — the
  checked-in `tools/list` snapshot. A tool name, description
  or schema change fails the suite until the fixture is
  regenerated on purpose (same canonicalisation the suite
  uses: tools sorted by name, keys sorted).
- `cargo run --example mcp_client_check` — an acceptance
  harness that connects as a real rmcp client and calls
  every tool and a resource.

## Later phases (not built)

Phase 2 adds cart tools behind per-client API keys, with
checkout always handed back to the browser (no payment data
ever crosses MCP). Phase 3 adds approval-gated draft orders
on the existing agent-approval machinery. Both are separate
decisions; the no-agent-orders rule stands until then.
