use crate::{Error, Result, cents};
use serde_json::{Value, json};
use std::sync::OnceLock;

pub struct Tool {
    pub name: &'static str,
    pub description: &'static str,
    pub write: bool,
    pub schema: Value,
}
fn string(max: usize) -> Value {
    json!({"type":"string","minLength":1,"maxLength":max})
}
fn integer(min: usize, max: usize) -> Value {
    json!({"type":"integer","minimum":min,"maximum":max})
}
fn boolean() -> Value {
    json!({"type":"boolean"})
}
fn enumeration(values: &[&str]) -> Value {
    json!({"type":"string","enum":values})
}
fn price() -> Value {
    json!({
        "type": "string",
        "pattern": r"^(0|[1-9][0-9]{0,7})(\.[0-9]{1,2})?$",
        "description": "Exact decimal currency units, e.g. 19.95; use a string."
    })
}
fn item() -> Value {
    json!({"item_id":string(2048)})
}
fn listing() -> Value {
    json!({
        "title": string(50), "description": string(640), "price": price(),
        "category_leaf_id": {
            "type": "string", "minLength": 1, "maxLength": 20,
            "description": "Take from a comparable listing: get_item(id).category."
        },
        "condition": enumeration(&[
            "un_opened", "in_box", "new", "as_good_as_new", "good", "fair", "has_given_it_all"
        ]),
        "brand": {
            "type": "string", "maxLength": 100,
            "description": "Optional. Many categories reject it with INVALID_ATTRIBUTE (e.g. 10082 dolls). On INVALID_ATTRIBUTE retry without brand; put it in title instead."
        },
        "model": {
            "type": "string", "maxLength": 100,
            "description": "Optional. Many categories reject it with INVALID_ATTRIBUTE. On INVALID_ATTRIBUTE retry without it."
        },
        "color": {
            "type": "string", "minLength": 1, "maxLength": 50,
            "description": "Optional. Many categories reject it with INVALID_ATTRIBUTE. On INVALID_ATTRIBUTE retry without it."
        },
        "max_weight_kg": integer(1,30),
        "shipping_allowed": boolean(), "cost_configuration_id": string(128)
    })
}
pub fn definitions() -> &'static [Tool] {
    static TOOLS: OnceLock<Vec<Tool>> = OnceLock::new();
    TOOLS.get_or_init(build_definitions)
}

fn build_definitions() -> Vec<Tool> {
    let mut tools = Vec::new();
    let mut add = |name, description, write: bool, mut props: Value, required: &[&str]| {
        if !write && name != "session_status" {
            props["fresh"] = boolean();
        }
        tools.push(Tool { name, description, write,
            schema: json!({"type":"object","properties":props,"required":required,"additionalProperties":false}) });
    };
    add(
        "session_status",
        "Inspect local session status; never contacts Wallapop or returns credentials.",
        false,
        json!({}),
        &[],
    );
    let search = json!({
        "keywords": string(250),
        "order_by": enumeration(&[
            "most_relevance", "newest", "price_low_to_high", "price_high_to_low", "closest"
        ]),
        "category_id": integer(1,10000000), "subcategory_ids": integer(1,10000000),
        "condition": string(100), "min_price": price(), "max_price": price(),
        "shippable": boolean(), "pages": integer(1,5), "limit": integer(1,50),
        "latitude": {"type": "number", "minimum": -90, "maximum": 90},
        "longitude": {"type": "number", "minimum": -180, "maximum": 180}
    });
    add(
        "search",
        "Search bounded pages with compact, deduplicated listings. Coordinates must be supplied together.",
        false,
        search.clone(),
        &["keywords"],
    );
    let mut deals = search;
    deals["min_discount"] = json!({"type":"number","minimum":0,"maximum":1});
    deals["top"] = integer(1, 50);
    add(
        "find_deals",
        "Find listings below the sample median; discounts describe this search sample, not market valuations.",
        false,
        deals,
        &["keywords"],
    );
    add(
        "get_item",
        "Get listing details from an internal ID, slug or es.wallapop.com item URL.",
        false,
        item(),
        &["item_id"],
    );
    add(
        "get_seller",
        "Get a seller profile, ratings and optionally ten recent reviews.",
        false,
        json!({"user_id":string(128),"include_reviews":boolean()}),
        &["user_id"],
    );
    add(
        "list_categories",
        "Get the search category tree.",
        false,
        json!({}),
        &[],
    );
    add(
        "list_own_items",
        "List the signed-in account's listings, up to 50 per page.",
        false,
        json!({"page":integer(0,1000),"limit":integer(1,50)}),
        &[],
    );
    add(
        "list_inbox",
        "Read a bounded inbox page.",
        false,
        json!({"page_size":integer(1,30),"max_messages":integer(1,30)}),
        &[],
    );
    add(
        "get_conversation",
        "Read the latest 100 available messages, ordered oldest first. History may be incomplete.",
        false,
        json!({"conversation_id":string(128)}),
        &["conversation_id"],
    );
    add(
        "unread_count",
        "Read the unread message count.",
        false,
        json!({}),
        &[],
    );
    for (name, description) in [
        ("list_purchases", "Read purchases without checkout."),
        ("list_sales", "Read sales."),
    ] {
        add(
            name,
            description,
            false,
            json!({"tab":enumeration(&["ongoing","finished"])}),
            &[],
        );
    }
    add(
        "list_favorites",
        "List the signed-in account's favorited listings, up to 5 pages.",
        false,
        json!({"pages": integer(1, 5)}),
        &[],
    );
    add(
        "saved_search_hits",
        "Total new listings across all saved searches (the badge counter).",
        false,
        json!({}),
        &[],
    );
    // saved_search_params reused for check_saved_search and save_search
    let alert_props = json!({
        "keywords": string(250),
        "order_by": enumeration(&[
            "most_relevance", "newest", "price_low_to_high", "price_high_to_low", "closest"
        ]),
        "category_id": integer(1, 10000000), "subcategory_ids": integer(1, 10000000),
        "condition": string(100), "min_price": price(), "max_price": price(),
        "shippable": boolean(),
        "latitude": {"type": "number", "minimum": -90, "maximum": 90},
        "longitude": {"type": "number", "minimum": -180, "maximum": 180}
    });
    add(
        "check_saved_search",
        "Check whether a search with exactly these filters is already saved; returns its id or null.",
        false,
        alert_props.clone(),
        &["keywords"],
    );
    add(
        "get_offer_constraints",
        "Read listing offer limits.",
        false,
        item(),
        &["item_id"],
    );
    add(
        "get_offer_status",
        "Derive offer history from a conversation, or summarize up to 30 inbox threads when conversation_id is omitted. History is not authoritative.",
        false,
        json!({"conversation_id":string(128),"last":integer(1,30)}),
        &[],
    );
    let mut p = item();
    p["favorited"] = boolean();
    add(
        "set_favorite",
        "Set favorite state.",
        true,
        p,
        &["item_id", "favorited"],
    );
    add(
        "send_message",
        "Message a seller by item_id or reply by conversation_id, including as seller. Supply exactly one. Unknown delivery must be checked before retrying.",
        true,
        json!({"item_id":string(2048),"conversation_id":string(128),"message":string(2000)}),
        &["message"],
    );
    let mut p = item();
    p["amount"] = price();
    p["message"] = string(1000);
    add(
        "send_offer",
        "Send a purchase offer. Does not buy or pay.",
        true,
        p,
        &["item_id", "amount"],
    );
    add(
        "respond_offer",
        "Accept or decline an offer.",
        true,
        json!({"offer_id":string(128),"action":enumeration(&["accept","decline"])}),
        &["offer_id", "action"],
    );
    add(
        "counter_offer",
        "Counter an open offer.",
        true,
        json!({"offer_id":string(128),"amount":price(),"currency":enumeration(&["EUR"])}),
        &["offer_id", "amount"],
    );
    let mut p = listing();
    p["photo_paths"] = json!({"type":"array","minItems":0,"maxItems":10,"items":string(4096),
        "description":"Server-local file paths only (no URLs). Use photo_files for URLs or ChatGPT uploads."});
    p["photo_files"] = json!({
        "type": "array", "minItems": 1, "maxItems": 10,
        "description": "Photos from URLs, data: URIs, upload: ticket refs, or OpenAI file objects. Exactly one of photo_paths / photo_files must be supplied.",
        "items": {
            "oneOf": [
                {"type": "string", "minLength": 1, "maxLength": 65536},
                {"type": "object",
                 "properties": {"download_url": {"type": "string", "minLength": 1, "maxLength": 65536}},
                 "required": ["download_url"]}
            ]
        }
    });
    add(
        "publish_item",
        "Publish with 1–10 photos. Supply photo_paths (server-local JPEG/PNG/WebP files) OR photo_files (https:// URLs from allowlisted hosts, data: URIs, upload: refs, or OpenAI {download_url} objects). Partial results retain the created item ID; never recreate a partial listing. Errors: outcome_unknown:false = nothing created, safe to retry with the same photo_files refs. upload_ticket_not_found = ticket expired or lost; create a new ticket, re-upload, then retry. Call get_item(item_id) for the public listing URL.",
        true,
        p,
        &[
            "title",
            "description",
            "price",
            "category_leaf_id",
            "condition",
            "shipping_allowed",
        ],
    );
    add(
        "create_upload_ticket",
        "Create 1–10 one-time upload tickets. Per photo: PUT raw bytes to upload_url, Content-Type: image/jpeg → 201 {ref, bytes}. Pass the returned ref values in publish_item.photo_files. Single-use, short TTL. Requires UPLOAD_SECRET and PUBLIC_BASE_URL to be configured.",
        true,
        json!({"count": integer(1, 10)}),
        &["count"],
    );
    let mut p = listing();
    p["item_id"] = string(2048);
    add(
        "edit_item",
        "Edit only supplied fields, preserving current attributes, location, shipping and pictures.",
        true,
        p,
        &["item_id"],
    );
    let mut p = item();
    p["reserved"] = boolean();
    add(
        "reserve_item",
        "Set reservation state on an owned listing.",
        true,
        p,
        &["item_id", "reserved"],
    );
    let mut p = item();
    p["sold"] = boolean();
    add(
        "mark_sold",
        "Set sold state on an owned listing.",
        true,
        p,
        &["item_id", "sold"],
    );
    add(
        "delete_item",
        "Permanently delete an owned listing.",
        true,
        item(),
        &["item_id"],
    );
    add(
        "pause_item",
        "Pause (inactivate) an owned listing: hidden from the marketplace but kept in your catalog. Editing it brings it back online.",
        true,
        item(),
        &["item_id"],
    );
    add(
        "save_search",
        "Save a search so Wallapop notifies you of new matching listings. Returns the saved search id.",
        true,
        alert_props,
        &["keywords"],
    );
    add(
        "remove_saved_search",
        "Delete a saved search by id.",
        true,
        json!({"search_id": string(128)}),
        &["search_id"],
    );
    tools
}
impl Tool {
    pub fn value(&self) -> Value {
        let destructive = self.write
            && !matches!(
                self.name,
                "set_favorite" | "send_message" | "send_offer" | "publish_item"
                    | "pause_item" | "save_search" | "create_upload_ticket"
            );
        let idempotent = matches!(
            self.name,
            "set_favorite" | "edit_item" | "reserve_item" | "mark_sold"
                | "delete_item" | "pause_item" | "remove_saved_search"
        );
        let mut v = json!({
            "name": self.name, "description": self.description, "inputSchema": self.schema,
            "annotations": {
                "readOnlyHint": !self.write, "destructiveHint": destructive,
                "idempotentHint": idempotent, "openWorldHint": self.name != "session_status"
            }
        });
        if self.name == "publish_item" {
            v["_meta"] = json!({"openai/fileParams": ["photo_files"]});
        }
        v
    }
    pub fn validate(&self, args: &Value) -> Result<()> {
        if args.to_string().len() > 32768 || !valid(&self.schema, args) {
            return Err(Error::new("invalid_arguments"));
        }
        if self.name == "send_message"
            && args.get("item_id").is_some() == args.get("conversation_id").is_some()
        {
            return Err(Error::new("provide_exactly_one_item_or_conversation"));
        }
        if matches!(self.name, "search" | "find_deals")
            && args.get("latitude").is_some() != args.get("longitude").is_some()
        {
            return Err(Error::new("coordinates_required_together"));
        }
        Ok(())
    }
}
// Only the small schema vocabulary emitted above is supported. Discovery and
// execution use the same definitions, including nested array and price checks.
fn valid(schema: &Value, v: &Value) -> bool {
    if let Some(one_of) = schema.get("oneOf").and_then(|v| v.as_array()) {
        return one_of.iter().filter(|s| valid(s, v)).count() == 1;
    }
    let typed = match schema["type"].as_str().unwrap_or("") {
        "object" => v.as_object().is_some_and(|m| {
            schema["required"]
                .as_array()
                .is_none_or(|a| a.iter().all(|k| m.contains_key(k.as_str().unwrap_or(""))))
                && m.iter()
                    .all(|(k, v)| {
                        if let Some(s) = schema["properties"].get(k) {
                            valid(s, v)
                        } else {
                            schema["additionalProperties"] != json!(false)
                        }
                    })
        }),
        "array" => v.as_array().is_some_and(|a| {
            bounds(schema, a.len() as f64, "minItems", "maxItems")
                && a.iter().all(|v| valid(&schema["items"], v))
        }),
        "string" => v.as_str().is_some_and(|s| {
            bounds(schema, s.chars().count() as f64, "minLength", "maxLength")
                && (schema.get("pattern").is_none() || cents(s).is_ok())
        }),
        "boolean" => v.is_boolean(),
        "integer" => {
            (v.is_u64() || v.is_i64())
                && v.as_f64()
                    .is_some_and(|n| bounds(schema, n, "minimum", "maximum"))
        }
        "number" => v
            .as_f64()
            .is_some_and(|n| n.is_finite() && bounds(schema, n, "minimum", "maximum")),
        _ => false,
    };
    typed && schema["enum"].as_array().is_none_or(|e| e.contains(v))
}
fn bounds(s: &Value, n: f64, min: &str, max: &str) -> bool {
    s[min].as_f64().is_none_or(|a| n >= a) && s[max].as_f64().is_none_or(|b| n <= b)
}
