#!/bin/bash
# MCP server suite (Kaimoru MCP Phase 1,
# docs/development/MCP_SERVER.md). Runs against the shared
# dev app with mcp_enabled = true in config.toml: the
# read-only public-catalog surface for external AI clients.
# Covers initialize in both protocol eras, the tools/list
# snapshot (a schema or description change fails the diff
# until the fixture is regenerated on purpose), parity with
# the REST catalog, boundary validation, the identical
# answer for draft and missing products, resources, and the
# host allowlist. Read-only: the suite creates nothing and
# changes nothing.
source "$(dirname "$0")/lib.sh"

db() { docker exec agentic-db psql -U agentic -d agentic_commerce -t -A -c "$1"; }
BASE=http://127.0.0.1:3000
MCP=$BASE/mcp
FIXTURE="$(dirname "$0")/fixtures/mcp_tools_list.json"

post() { curl -s -X POST "$MCP" -H 'Content-Type: application/json' -H 'Accept: application/json, text/event-stream' -d "$1"; }
# Prints one value out of a JSON document; the expression
# runs over `d` (the parsed document in $2).
jqe() { python3 -c "import json,sys; d=json.loads(sys.argv[2]); print($1)" x "$2"; }
call() { post "{\"jsonrpc\":\"2.0\",\"id\":9,\"method\":\"tools/call\",\"params\":{\"name\":\"$1\",\"arguments\":$2}}"; }
readres() { post "{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"resources/read\",\"params\":{\"uri\":\"$1\"}}"; }

echo "== initialize, both eras =="
INIT=$(post '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"suite","version":"0"}}}')
[ "$(jqe "d['result']['serverInfo']['name']" "$INIT")" = "kaimoru" ] && ok "initialize: server identifies as kaimoru" || ng "initialize: server identity"
DISC=$(curl -s -X POST "$MCP" -H 'Content-Type: application/json' -H 'Accept: application/json, text/event-stream' -H 'Mcp-Method: server/discover' -H 'Mcp-Name: server/discover' -H 'MCP-Protocol-Version: 2026-07-28' -d '{"jsonrpc":"2.0","id":0,"method":"server/discover","params":{"_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientInfo":{"name":"suite","version":"0"},"io.modelcontextprotocol/clientCapabilities":{}}}}')
[ "$(jqe "'2026-07-28' in d['result']['supportedVersions']" "$DISC")" = "True" ] && ok "server/discover: 2026-07-28 supported" || ng "server/discover"

echo "== tools/list snapshot =="
post '{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}' | python3 -c "
import json,sys
d=json.load(sys.stdin)
tools=sorted(d['result']['tools'], key=lambda t: t['name'])
print(json.dumps({'tools': tools}, ensure_ascii=False, sort_keys=True, indent=2))
" > /tmp/mcp_tools_now.json
if diff -q "$FIXTURE" /tmp/mcp_tools_now.json >/dev/null; then ok "tools/list matches the checked-in snapshot"; else ng "tools/list drifted from the snapshot (regenerate the fixture if the change is intended)"; fi

echo "== catalog parity with the REST API =="
REST=$(curl -s "$BASE/api/products")
MCP_R=$(call search_products '{}')
[ "$(jqe "d['result']['structuredContent']['total']" "$MCP_R")" = "$(jqe "d['total']" "$REST")" ] && ok "browse total matches the REST catalog" || ng "browse total != REST total"
P1_NAME=$(db "SELECT name FROM products WHERE id=1")
KW=$(call search_products "{\"query\": \"$P1_NAME\"}")
KW_TOTAL=$(jqe "d['result']['structuredContent']['total']" "$KW")
[ "${KW_TOTAL:-0}" -ge 1 ] && ok "exact-name keyword search finds the product" || ng "keyword search total=$KW_TOTAL"

echo "== get_product =="
GP=$(call get_product '{"id": 1}')
[ "$(jqe "d['result']['structuredContent']['id']" "$GP")" = "1" ] && ok "get_product returns the product" || ng "get_product id"
[ "$(jqe "isinstance(d['result']['structuredContent']['variants'], list)" "$GP")" = "True" ] && ok "get_product carries variants" || ng "get_product variants"
DRAFT_ID=$(db "SELECT id FROM products WHERE status <> 'public' ORDER BY id LIMIT 1")
DR=$(call get_product "{\"id\": $DRAFT_ID}")
DRAFT_MSG=$(jqe "d['error']['message']" "$DR")
[ "$(jqe "d['error']['code']" "$DR")" = "-32602" ] && ok "draft product error is invalid_params" || ng "draft error code"
case "$DRAFT_MSG" in "no public product with id "*) ok "draft product answers like a missing one";; *) ng "draft product answer: $DRAFT_MSG";; esac
MS=$(call get_product '{"id": 999999999}')
case "$(jqe "d['error']['message']" "$MS")" in "no public product with id "*) ok "missing product answers identically in shape";; *) ng "missing product answer";; esac

echo "== boundary validation =="
for args in '{"sort": "cheapest"}' '{"page": 0}' '{"min_price_yen": 5000, "max_price_yen": 100}' '{"min_price_yen": -1}'; do
  R=$(call search_products "$args")
  [ "$(jqe "d['error']['code']" "$R")" = "-32602" ] && ok "search_products refuses $args" || ng "search_products accepted $args"
done
R=$(call get_product '{"id": 0}')
[ "$(jqe "d['error']['code']" "$R")" = "-32602" ] && ok "get_product refuses id 0" || ng "get_product id 0"
R=$(call no_such_tool '{}')
[ -n "$(jqe "d.get('error', {}).get('code', '')" "$R")" ] && ok "unknown tool is a protocol error" || ng "unknown tool answered"

echo "== categories and store info =="
CATS=$(call list_categories '{}')
CAT_N=$(jqe "len(d['result']['structuredContent']['categories'])" "$CATS")
[ "${CAT_N:-0}" -gt 0 ] && ok "list_categories returns $CAT_N categories" || ng "list_categories empty"
INFO=$(call get_store_info '{}')
[ "$(jqe "d['result']['structuredContent']['currency']" "$INFO")" = "JPY" ] && ok "store info currency is JPY" || ng "store info currency"
DB_FEE=$(db "SELECT shipping_flat_yen FROM shop_settings LIMIT 1")
[ "$(jqe "d['result']['structuredContent']['shipping_flat_yen']" "$INFO")" = "$DB_FEE" ] && ok "store info shipping fee matches shop settings" || ng "store info shipping fee"

echo "== resources =="
RES=$(post '{"jsonrpc":"2.0","id":3,"method":"resources/list","params":{}}')
[ "$(jqe "any(r['uri']=='store://store/info' for r in d['result']['resources'])" "$RES")" = "True" ] && ok "resources/list has the store info" || ng "resources/list store info"
[ "$(jqe "any(r['uri']=='store://pages/guide' for r in d['result']['resources'])" "$RES")" = "True" ] && ok "resources/list has the guide page" || ng "resources/list guide"
TPL=$(post '{"jsonrpc":"2.0","id":4,"method":"resources/templates/list","params":{}}')
[ "$(jqe "any(t['uriTemplate']=='store://products/{id}' for t in d['result']['resourceTemplates'])" "$TPL")" = "True" ] && ok "templates/list has the product template" || ng "templates/list"
READ=$(readres "store://products/1")
[ "$(jqe "json.loads(d['result']['contents'][0]['text'])['id']" "$READ")" = "1" ] && ok "product resource reads as JSON" || ng "product resource read"
R=$(readres "store://nope")
[ "$(jqe "d['error']['code']" "$R")" = "-32002" ] && ok "unknown resource is resource_not_found" || ng "unknown resource code"
R=$(readres "store://products/$DRAFT_ID")
[ "$(jqe "d['error']['code']" "$R")" = "-32002" ] && ok "draft product resource is not found" || ng "draft resource code"

echo "== host allowlist =="
HOST_CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST "$MCP" -H 'Content-Type: application/json' -H 'Accept: application/json, text/event-stream' -H 'Host: evil.example.com' -d '{"jsonrpc":"2.0","id":8,"method":"tools/list","params":{}}')
[ "$HOST_CODE" = "403" ] && ok "bogus Host is refused (403)" || ng "bogus Host answered $HOST_CODE"

result
