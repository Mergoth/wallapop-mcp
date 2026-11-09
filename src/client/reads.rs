use super::{
    Client,
    models::{
        conversation, deals, normalize, offer_inbox, offer_status, transactions, valid_categories,
    },
    search::Nonempty,
};
use crate::{Context, Error, Result, identifier, pick, text};
use crate::transport::query;
use serde_json::{Value, json};

impl Client {
    pub(super) fn read(&self, ctx: &Context, name: &str, a: &Value) -> Result<Value> {
        let fresh = a["fresh"] == true;
        let id = || identifier(a["conversation_id"].as_str().unwrap_or(""));
        match name {
            "session_status" => Ok(self.status()),
            "search" | "find_deals" => {
                let result = self.search(ctx, a)?;
                if name == "find_deals" {
                    deals(result, a)
                } else {
                    Ok(result)
                }
            }
            "get_item" => {
                let id = self.resolve(ctx, &text(&a["item_id"]), fresh)?;
                normalize(
                    &self.get(ctx, &format!("/items/{id}"), false, fresh)?,
                    &self.web,
                    false,
                )
            }
            "list_categories" => {
                let v = self.get(ctx, "/categories?context=search", false, fresh)?;
                let categories = v
                    .get("categories")
                    .filter(|c| valid_categories(c))
                    .ok_or(Error::new("invalid_response"))?;
                Ok(json!({"categories":categories}))
            }
            "get_seller" => {
                let id = identifier(a["user_id"].as_str().unwrap_or(""))?;
                let v = self.get(ctx, &format!("/users/{id}"), false, fresh)?;
                if v.get("id").is_none() && v.get("micro_name").is_none() {
                    return Err(Error::new("invalid_response"));
                }
                let mut out = pick(
                    &v,
                    &["id", "is_top_profile", "seller_type", "register_date"],
                );
                out["name"] = v["micro_name"].clone();
                let stats = self.get(ctx, &format!("/users/{id}/stats"), false, fresh)?;
                if !stats["rating_average"].is_number() && !stats["counters"].is_array() {
                    return Err(Error::new("invalid_response"));
                }
                out["rating_average"] = stats["rating_average"].clone();
                out["counters"] = stats["counters"].clone();
                if a["include_reviews"] == true {
                    let v = self.get(ctx, &format!("/users/{id}/reviews"), false, fresh)?;
                    out["reviews"] = v
                        .as_array()
                        .ok_or(Error::new("invalid_response"))?
                        .iter()
                        .take(10)
                        .cloned()
                        .collect();
                }
                Ok(out)
            }
            "list_own_items" => {
                let me = self.get(ctx, "/users/me", true, fresh)?;
                let id = text(&me["id"]);
                identifier(&id)?;
                let path = self.with_query(
                    &format!("/users/{id}/items"),
                    &[
                        ("page", text(&a["page"]).nonempty("0")),
                        ("max_items", text(&a["limit"]).nonempty("25")),
                    ],
                );
                let v = self.get(ctx, &path, true, fresh)?;
                let items = v["data"]
                    .as_array()
                    .ok_or(Error::new("invalid_response"))?
                    .iter()
                    .take(a["limit"].as_u64().unwrap_or(25) as usize)
                    .map(|v| normalize(v, &self.web, true))
                    .collect::<Result<Vec<_>>>()?;
                Ok(json!({"count":items.len(),"items":items}))
            }
            "list_inbox" => {
                let path = self.with_query(
                    "/bff/messaging/inbox",
                    &[
                        ("page_size", text(&a["page_size"]).nonempty("30")),
                        ("max_messages", text(&a["max_messages"]).nonempty("30")),
                    ],
                );
                let v = self.get(ctx, &path, true, fresh)?;
                let conversations = v["conversations"]
                    .as_array()
                    .ok_or(Error::new("invalid_response"))?
                    .iter()
                    .take(30)
                    .map(|v| conversation(v, true))
                    .collect::<Result<Vec<_>>>()?;
                Ok(
                    json!({"count":conversations.len(),"conversations_shown":conversations.len(),"conversations":conversations,"history_complete":false}),
                )
            }
            "get_offer_status" if a.get("conversation_id").is_none() => {
                let v = self.get(
                    ctx,
                    "/bff/messaging/inbox?page_size=30&max_messages=30",
                    true,
                    fresh,
                )?;
                offer_inbox(&v, a["last"].as_u64().unwrap_or(30) as usize)
            }
            "get_conversation" | "get_offer_status" => {
                let v = self.get(
                    ctx,
                    &format!("/bff/messaging/conversation/{}", id()?),
                    true,
                    fresh,
                )?;
                if name == "get_offer_status" {
                    offer_status(&v)
                } else {
                    conversation(&v, false)
                }
            }
            "unread_count" => {
                let v = self.get(ctx, "/instant-messaging/messages/unread", true, fresh)?;
                let n = v["unread_counter"]
                    .as_u64()
                    .ok_or(Error::new("invalid_response"))?;
                Ok(json!({"unread_count":n}))
            }
            "list_purchases" | "list_sales" => {
                let tab = a["tab"].as_str().unwrap_or("ongoing");
                let kind = if name == "list_sales" {
                    "selling"
                } else {
                    "purchases"
                };
                let v = self.get(ctx, &format!("/sdui/{kind}/{tab}"), true, fresh)?;
                let mut out = Vec::new();
                let mut recognized = false;
                transactions(v.get("main").unwrap_or(&v), &mut out, &mut recognized);
                if !recognized {
                    return Err(Error::new("invalid_response"));
                }
                Ok(json!({"count":out.len(),"transactions":out,"tab":tab}))
            }
            "get_offer_constraints" => {
                let id = self.resolve(ctx, &text(&a["item_id"]), fresh)?;
                let v = self.get(
                    ctx,
                    &format!("/bff/delivery/make-an-offer?item_id={id}"),
                    true,
                    fresh,
                )?;
                if !v["item"]["price"]["amount"].is_number() || !v["restriction"].is_object() {
                    return Err(Error::new("invalid_response"));
                }
                Ok(json!({"price":v["item"]["price"]["amount"],"restriction":v["restriction"]}))
            }
            "list_favorites" => {
                let pages = a["pages"].as_u64().unwrap_or(1).min(5) as usize;
                let mut items: Vec<Value> = Vec::new();
                let mut since = String::new();
                for _ in 0..pages {
                    let path = if since.is_empty() {
                        "/favorite-lists".into()
                    } else {
                        query("/favorite-lists", &[("since".into(), since.clone())])
                    };
                    let v = self.get(ctx, &path, true, fresh)?;
                    let raw = v["data"].as_array().ok_or(Error::new("invalid_response"))?;
                    for item in raw {
                        if let Ok(n) = normalize(item, &self.web, true) {
                            items.push(n);
                        }
                    }
                    let next = text(&v["meta"]["next"]);
                    if next.is_empty() {
                        break;
                    }
                    since = next;
                }
                Ok(json!({"count": items.len(), "items": items}))
            }
            "saved_search_hits" => {
                let v = self.get(
                    ctx,
                    "/searchalerts/savedsearch/alerts/total-hits",
                    true,
                    fresh,
                )?;
                Ok(json!({"hits": v["hits"].as_u64().unwrap_or(0)}))
            }
            "check_saved_search" => {
                let params = self.alert_params(ctx, a)?;
                let path = query("/searchalerts/savedsearch/exists", &params);
                let (status, location) = self.authed_with_location(ctx, "HEAD", &path)?;
                let id = location.as_deref().and_then(id_from_location).map(str::to_owned);
                Ok(json!({"saved": status == 200, "id": id}))
            }
            _ => Err(Error::new("unknown_tool")),
        }
    }
}

fn id_from_location(loc: &str) -> Option<&str> {
    loc.trim_end_matches('/').rsplit('/').next().filter(|s| !s.is_empty())
}
