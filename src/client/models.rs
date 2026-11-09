//! Normalize untrusted response shapes without performing I/O.
use crate::{Error, Result, cents, first, identifier, pick, text, transport::encode};
use serde_json::{Value, json};

pub(super) fn contains_category(v: &Value, id: &Value) -> bool {
    v["id"] == *id
        || v["subcategories"]
            .as_array()
            .is_some_and(|a| a.iter().any(|v| contains_category(v, id)))
}

pub(super) fn valid_categories(v: &Value) -> bool {
    v.as_array().is_some_and(|a| {
        a.iter().all(|v| {
            v["id"].as_u64().is_some_and(|n| n > 0)
                && v["name"].as_str().is_some_and(|s| !s.trim().is_empty())
                && v.get("subcategories").is_none_or(valid_categories)
        })
    })
}

pub(super) fn normalize(v: &Value, web: &str, compact: bool) -> Result<Value> {
    let id = text(&v["id"]);
    identifier(&id)?;
    let p = if !v["price"]["cash"].is_null() {
        &v["price"]["cash"]
    } else if !v["price"]["full"].is_null() {
        &v["price"]["full"]
    } else {
        &v["price"]
    };
    let amount = p.get("amount").unwrap_or(p);
    let price = text(amount);
    let slug = text(&first(v, &["web_slug", "slug", "id"]));
    let shippable = v["is_shippable"] == true
        || ["item_is_shippable", "is_shippable", "user_allows_shipping"]
            .iter()
            .any(|key| v["shipping"][key] == true);
    let mut out = json!({
        "id": id, "title": attribute(v,"title"), "city": v["location"]["city"],
        "user_id": v["user"]["id"], "user_name": v["user"]["micro_name"],
        "url": format!("{web}/item/{}",encode(&slug)), "shippable": shippable,
        "reserved": v["reserved"] == true || v["reserved"]["flag"] == true,
        "sold": v["is_sold"] == true || v["sold"] == true || v["sold"]["flag"] == true
    });
    if cents(&price).is_ok() {
        out["price"] = serde_json::from_str(&price).unwrap_or(Value::Null);
        out["currency"] = p["currency"].clone();
    }
    out["category"] = v.get("category_leaf_id").cloned().unwrap_or_else(|| {
        v["taxonomy"]
            .as_array()
            .and_then(|a| a.last())
            .map(|v| v["id"].clone())
            .unwrap_or(Value::Null)
    });
    let images: Vec<_> = v["images"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| {
            let urls = v.get("urls").or_else(|| v.get("media"))?;
            ["medium", "small", "big"]
                .iter()
                .find_map(|k| urls[k].as_str().map(|s| Value::String(s.into())))
        })
        .take(if compact { 1 } else { 10 })
        .collect();
    out["images"] = images.into();
    if !compact {
        out["description"] = attribute(v, "description");
    }
    Ok(out)
}

pub(super) fn attribute(v: &Value, key: &str) -> Value {
    let value = v
        .get(key)
        .or_else(|| v["type_attributes"].get(key))
        .or_else(|| v["attributes"].get(key))
        .unwrap_or(&Value::Null);
    if value.is_string() {
        value.clone()
    } else {
        first(value, &["original", "value", "text"])
    }
}

pub(super) fn messages(v: &Value) -> Result<Vec<Value>> {
    let mut a = v["messages"]["messages"]
        .as_array()
        .ok_or(Error::new("invalid_conversation"))?
        .clone();
    a.sort_by(|a, b| timestamp_order(&a["timestamp"], &b["timestamp"]));
    Ok(a)
}

fn timestamp_order(a: &Value, b: &Value) -> std::cmp::Ordering {
    let (a, b) = (text(a), text(b));
    match (a.parse::<i64>(), b.parse::<i64>()) {
        (Ok(a), Ok(b)) => a.cmp(&b),
        _ => a.cmp(&b),
    }
}

pub(super) fn conversation(v: &Value, inbox: bool) -> Result<Value> {
    let id = first(v, &["hash", "conversation_id", "conversation_hash", "id"]);
    if text(&id).is_empty() {
        return Err(Error::new("invalid_conversation"));
    }
    let mut out = pick(v, &["id", "unread_counter", "last_message_date", "channel"]);
    out["conversation_id"] = id;
    out["item"] = pick(&v["item"], &["id", "title", "price"]);
    out["with_user"] = pick(&v["with_user"], &["id", "hash"]);
    out["with_user"]["name"] = first(&v["with_user"], &["micro_name", "name"]);
    let mut msgs = messages(v)?;
    let count = msgs.len();
    out["history_complete"] = false.into();
    if inbox {
        out["messages_returned"] = count.into();
        out["last_message"] = msgs
            .last()
            .map(|v| pick(v, &["text", "timestamp"]))
            .unwrap_or(Value::Null);
    } else {
        if count > 100 {
            msgs.drain(..count - 100);
        }
        out["messages_returned"] = msgs.len().into();
        out["messages"] = msgs
            .iter()
            .map(|v| pick(v, &["id", "text", "type", "timestamp", "from_self"]))
            .collect();
    }
    Ok(out)
}

pub(super) fn transactions(v: &Value, out: &mut Vec<Value>, recognized: &mut bool) {
    if v["type"] == "empty_state" {
        *recognized = true;
        return;
    }
    if v["type"] == "list_item_transaction" {
        *recognized = true;
        let d = &v["data"];
        let t = |k| d[k]["data"]["content"].clone();
        let details: Vec<_> = d["extra_details"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|v| {
                json!({"title":v["data"]["title"]["data"]["content"],
            "description":first(&v["data"],&["description","label"])["data"]["content"]})
            })
            .collect();
        out.push(json!({"id":v["id"],"title":t("main"),"detail":t("detail"),"price":t("detail_price"),"details":details,
            "url":v["events"]["on_click"]["data"]["url"]}));
        return;
    }
    match v {
        Value::Object(m) => {
            for v in m.values() {
                transactions(v, out, recognized);
            }
        }
        Value::Array(a) => {
            for v in a {
                transactions(v, out, recognized);
            }
        }
        _ => (),
    }
}

pub(super) fn deals(mut result: Value, a: &Value) -> Result<Value> {
    let mut prices: Vec<_> = result["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|v| v["reserved"] != true && v["sold"] != true)
        .filter_map(|v| cents(&text(&v["price"])).ok())
        .filter(|p| *p > 0)
        .collect();
    prices.sort_unstable();
    let median = if prices.is_empty() {
        0
    } else {
        (prices[(prices.len() - 1) / 2] + prices[prices.len() / 2]) / 2
    };
    let threshold = a["min_discount"].as_f64().unwrap_or(0.15);
    let mut items: Vec<_> = result["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| {
            let p = cents(&text(&v["price"])).ok()?;
            let discount = if median == 0 {
                0.0
            } else {
                1.0 - p as f64 / median as f64
            };
            if p == 0 || discount < threshold || v["reserved"] == true || v["sold"] == true {
                return None;
            }
            let mut v = v.clone();
            v["discount"] = discount.into();
            Some(v)
        })
        .collect();
    items.sort_by(|a, b| {
        b["discount"]
            .as_f64()
            .partial_cmp(&a["discount"].as_f64())
            .unwrap()
    });
    items.truncate(a["top"].as_u64().unwrap_or(10) as usize);
    result["sample_size"] = prices.len().into();
    result["sample_median"] = json!(median as f64 / 100.0);
    result["count"] = items.len().into();
    result["items"] = items.into();
    Ok(result)
}

pub(super) fn offer_status(v: &Value) -> Result<Value> {
    let mut events = Vec::new();
    let mut amount = Value::Null;
    let mut offer_id = Value::Null;
    for m in messages(v)? {
        if m["type"] != "delivery_generic" {
            continue;
        }
        let payload = m["payload"]
            .as_str()
            .and_then(|s| serde_json::from_str::<Value>(s).ok())
            .unwrap_or_else(|| m["payload"].clone());
        let s = m["text"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| {
                text(&first(&payload["loc"], &["current"])) + &text(&payload["text"])
            });
        let low = s.trim().to_lowercase();
        let kind = if low.contains("contraoferta") {
            if low.starts_with("has") {
                "our-counter"
            } else {
                "seller-counter"
            }
        } else if low.contains("cómpralo ahora") || low.contains("oferta aceptada") {
            "accepted-buy"
        } else if low.contains("ha aceptado tu oferta") {
            "seller-accepted"
        } else if low.contains("ha rechazado tu oferta") {
            "seller-declined"
        } else if low.starts_with("has rechazado") {
            "our-declined"
        } else if low.contains("oferta de") && low.starts_with("has") {
            "our-offer"
        } else if low.contains("caducad") {
            "expired"
        } else {
            continue;
        };
        if let Some(prefix) = s.split('€').next().filter(|_| s.contains('€')) {
            let n: String = prefix
                .trim_end()
                .chars()
                .rev()
                .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == ',')
                .collect::<String>()
                .chars()
                .rev()
                .collect();
            if let Ok(v) = serde_json::from_str::<Value>(&n.replace(',', ".")) {
                amount = v;
            }
        }
        let p = first(
            &payload,
            &["offer_price_amount", "amount", "counter_offer_price_amount"],
        );
        if !p.is_null() {
            amount = p;
        }
        if let Some(buttons) = payload["buttons"].as_array() {
            for b in buttons {
                if let Some((_, id)) = text(&b["action"]).split_once("/app/chat/offer/") {
                    let id = id.split(['?', '#', '/']).next().unwrap_or("");
                    if identifier(id).is_ok() {
                        offer_id = id.into();
                    }
                }
            }
        }
        events.push(
            json!({"kind":kind,"amount":amount,"offer_id":offer_id,"timestamp":m["timestamp"]}),
        );
    }
    let state = events.last().cloned().unwrap_or(json!({"kind":"none"}));
    Ok(json!({
        "conversation_id": first(v,&["hash","conversation_id","conversation_hash"]),
        "status": state["kind"], "amount": amount, "offer_id": offer_id,
        "available": !events.is_empty(), "actionable": state["kind"] == "seller-counter",
        "event_count": events.len(), "events": events,
        "history_complete": false, "authoritative": false
    }))
}

pub(super) fn offer_inbox(v: &Value, limit: usize) -> Result<Value> {
    let mut threads = v["conversations"]
        .as_array()
        .ok_or(Error::new("invalid_response"))?
        .iter()
        .take(30)
        .map(|v| {
            let mut summary = offer_status(v)?;
            summary["item"] = pick(&v["item"], &["id", "title", "price", "status"]);
            summary["with_user"] = pick(&v["with_user"], &["id", "hash", "name", "micro_name"]);
            summary["last_message_date"] = messages(v)?
                .last()
                .map(|m| m["timestamp"].clone())
                .unwrap_or_else(|| v["last_message_date"].clone());
            Ok(summary)
        })
        .collect::<Result<Vec<_>>>()?;
    threads.sort_by(|a, b| timestamp_order(&b["last_message_date"], &a["last_message_date"]));
    threads.truncate(limit);
    Ok(
        json!({"count":threads.len(), "conversations":threads, "history_complete":false, "authoritative":false}),
    )
}
