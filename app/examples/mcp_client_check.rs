//! Manual acceptance harness for the MCP server (Phase 1):
//! connects to a running dev app as a real MCP client would,
//! lists the tools and resources, and calls each one.
//!
//! Run with the dev app up and `mcp_enabled = true`:
//! `cargo run --example mcp_client_check`
//! Exits non-zero at the first failed expectation.
//!
//! With `MCP_TEST_API_KEY` set to a cart:write key, the
//! harness also runs the Phase 2 cart flow (create, add,
//! read back, checkout link) on a second, authenticated
//! connection. Unset, the cart flow is skipped and the
//! Phase 1 behaviour is unchanged.

use rmcp::{
    ClientHandler, RoleClient, ServiceExt,
    model::{
        CallToolRequestParams, ClientCapabilities, ClientConfig, Implementation,
        ReadResourceRequestParams,
    },
    service::RunningService,
    transport::{
        StreamableHttpClientTransport, streamable_http_client::StreamableHttpClientTransportConfig,
    },
};

/// A minimal client: default handlers, the store's tools do
/// not ask the client for anything.
#[derive(Clone)]
struct CheckClient;

impl ClientHandler for CheckClient {
    fn get_info(&self) -> ClientConfig {
        ClientConfig::new(
            ClientCapabilities::default(),
            Implementation::new("mcp-client-check", "0.1.0"),
        )
    }
}

/// Fails the run with a message.
fn expect(condition: bool, message: &str) {
    if !condition {
        eprintln!("FAIL: {message}");
        std::process::exit(1);
    }
}

/// Reads a tool call's structured content as JSON.
fn structured(result: &rmcp::model::CallToolResult) -> serde_json::Value {
    expect(result.is_error != Some(true), "tool call returned an error");
    result
        .structured_content
        .clone()
        .expect("tool result carries structured content")
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let transport = StreamableHttpClientTransport::from_uri("http://127.0.0.1:3000/mcp");
    let client: RunningService<RoleClient, CheckClient> = CheckClient.serve(transport).await?;

    let info = client.peer_info().expect("initialize answered");
    let server = info.server_info.as_ref().expect("server info present");
    println!("server: {} {}", server.name, server.version);
    expect(server.name == "kaimoru", "server identifies as kaimoru");

    let tools = client.list_tools(Default::default()).await?;
    let names: Vec<&str> = tools.tools.iter().map(|t| t.name.as_ref()).collect();
    println!("tools: {names:?}");
    for expected in [
        "search_products",
        "get_product",
        "list_categories",
        "get_store_info",
    ] {
        expect(names.contains(&expected), "expected tool is listed");
    }

    let search = client
        .call_tool(
            CallToolRequestParams::new("search_products")
                .with_arguments(serde_json::json!({}).as_object().unwrap().clone()),
        )
        .await?;
    let search_json = structured(&search);
    let total = search_json["total"].as_i64().unwrap_or(0);
    println!("browse: total={total}");
    expect(total > 0, "catalog browse finds products");
    let first_id = search_json["products"][0]["id"]
        .as_i64()
        .expect("first product id");

    let product = client
        .call_tool(
            CallToolRequestParams::new("get_product").with_arguments(
                serde_json::json!({"id": first_id})
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
        )
        .await?;
    let product_json = structured(&product);
    println!("get_product {first_id}: {}", product_json["name"]);
    expect(
        product_json["variants"].is_array(),
        "detail carries variants",
    );

    let categories = client
        .call_tool(CallToolRequestParams::new("list_categories"))
        .await?;
    let categories_json = structured(&categories);
    let count = categories_json["categories"].as_array().map_or(0, Vec::len);
    println!("categories: {count}");
    expect(count > 0, "categories are listed");

    let store = client
        .call_tool(CallToolRequestParams::new("get_store_info"))
        .await?;
    let store_json = structured(&store);
    println!(
        "store: {} ({})",
        store_json["shop_name"], store_json["currency"]
    );
    expect(store_json["currency"] == "JPY", "currency is JPY");

    let resources = client.list_resources(Default::default()).await?;
    println!("resources: {}", resources.resources.len());
    expect(
        resources
            .resources
            .iter()
            .any(|r| r.uri == "store://store/info"),
        "store info resource is listed",
    );

    let read = client
        .read_resource(ReadResourceRequestParams::new(format!(
            "store://products/{first_id}"
        )))
        .await?;
    println!("resource read: {} content block(s)", read.contents.len());
    expect(!read.contents.is_empty(), "product resource reads");

    // Phase 2 cart flow, only when a key is provided: a
    // second connection carrying the bearer key, because the
    // anonymous connection above must stay anonymous.
    if let Ok(api_key) = std::env::var("MCP_TEST_API_KEY") {
        // The config's auth value is the bare token: the
        // transport sends it via reqwest's bearer_auth,
        // which adds the "Bearer " scheme itself.
        let config = StreamableHttpClientTransportConfig::with_uri("http://127.0.0.1:3000/mcp")
            .auth_header(api_key);
        let transport = StreamableHttpClientTransport::from_config(config);
        let authed: RunningService<RoleClient, CheckClient> = CheckClient.serve(transport).await?;

        let created = authed
            .call_tool(CallToolRequestParams::new("create_cart"))
            .await?;
        let created_json = structured(&created);
        let cart_id = created_json["cart_id"]
            .as_str()
            .expect("cart id is a string")
            .to_string();
        println!("cart: created {cart_id}");
        expect(
            cart_id.starts_with("mcp_"),
            "cart id carries the mcp_ prefix",
        );
        expect(
            created_json["lines"].as_array().is_some_and(Vec::is_empty),
            "new cart is empty",
        );

        // Add the product fetched above, through its first
        // variant when it has combinations.
        let mut add_args =
            serde_json::json!({"cart_id": cart_id, "product_id": first_id, "quantity": 2});
        if let Some(variant_id) = product_json["variants"][0]["id"].as_i64() {
            add_args["variant_id"] = serde_json::json!(variant_id);
        }
        let added = authed
            .call_tool(
                CallToolRequestParams::new("add_cart_item")
                    .with_arguments(add_args.as_object().unwrap().clone()),
            )
            .await?;
        let added_json = structured(&added);
        println!(
            "cart: {} item(s), total {} yen",
            added_json["item_count"], added_json["total_yen"]
        );
        expect(
            added_json["item_count"].as_i64() == Some(2),
            "cart holds the added units",
        );
        expect(
            added_json["total_yen"].as_i64().unwrap_or(0) > 0,
            "cart total is positive",
        );

        let fetched = authed
            .call_tool(
                CallToolRequestParams::new("get_cart").with_arguments(
                    serde_json::json!({"cart_id": cart_id})
                        .as_object()
                        .unwrap()
                        .clone(),
                ),
            )
            .await?;
        let fetched_json = structured(&fetched);
        expect(
            fetched_json["total_yen"] == added_json["total_yen"],
            "get_cart agrees with the add read-back",
        );

        let link = authed
            .call_tool(
                CallToolRequestParams::new("get_checkout_link").with_arguments(
                    serde_json::json!({"cart_id": cart_id})
                        .as_object()
                        .unwrap()
                        .clone(),
                ),
            )
            .await?;
        let link_json = structured(&link);
        let url = link_json["checkout_url"].as_str().unwrap_or("");
        println!("cart: checkout link {url}");
        expect(url.contains("/cart/claim/"), "checkout link is a claim URL");
        println!("MCP CLIENT CHECK (cart flow): ALL OK");
    }

    println!("MCP CLIENT CHECK: ALL OK");
    Ok(())
}
