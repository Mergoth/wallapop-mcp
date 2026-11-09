use crate::{Error, Result, tools};
use serde_json::{Value, json};
use std::collections::HashMap;

#[derive(Default)]
pub(super) struct Options {
    pub(super) values: HashMap<String, String>,
    pub(super) positions: Vec<String>,
}
impl Options {
    pub(super) fn parse(args: &[String]) -> Result<Self> {
        let flags = [
            "help",
            "version",
            "allow-writes",
            "json",
            "csv",
            "quiet",
            "ship-only",
            "fresh",
            "finished",
            "remove",
            "undo",
            "yes",
            "dry-run",
            "once",
            "token-stdin",
            "password-stdin",
        ];
        let mut out = Self::default();
        let mut i = 0;
        while i < args.len() {
            if args[i] == "--" {
                out.positions.extend_from_slice(&args[i + 1..]);
                break;
            }
            let arg = if args[i] == "-h" { "--help" } else { &args[i] };
            if let Some(key) = arg.strip_prefix("--") {
                let value = if flags.contains(&key) {
                    "true".into()
                } else {
                    i += 1;
                    args.get(i)
                        .ok_or(Error::new("option_value_required"))?
                        .clone()
                };
                if out.values.insert(key.into(), value).is_some() {
                    return Err(Error::new("duplicate_option"));
                }
            } else {
                out.positions.push(args[i].clone());
            }
            i += 1;
        }
        Ok(out)
    }
    pub(super) fn get(&self, key: &str) -> Option<&str> {
        self.values.get(key).map(String::as_str)
    }
    pub(super) fn flag(&self, key: &str) -> bool {
        self.get(key) == Some("true")
    }
    pub(super) fn position(&self, index: usize) -> Result<&str> {
        self.positions
            .get(index)
            .map(String::as_str)
            .ok_or(Error::new("positional_argument_required"))
    }
    pub(super) fn check(&self, count: usize, keys: &[&str]) -> Result<()> {
        if self.positions.len() != count {
            return Err(Error::new("unexpected_arguments"));
        }
        if self.values.keys().any(|k| !keys.contains(&k.as_str())) {
            return Err(Error::new("unknown_option"));
        }
        Ok(())
    }
}

pub(super) fn arguments<'a>(command: &'a str, o: &Options) -> Result<(&'a str, Value)> {
    let count = match command {
        "search" | "deals" | "watch" => o.positions.len().max(1),
        "show" | "seller" | "edit" | "favorite" | "reserve" | "sold" | "delete" | "publish" => 1,
        "message" => usize::from(o.get("conversation").is_none()),
        "offer" => {
            if matches!(o.position(0)?, "accept" | "decline" | "counter") {
                2
            } else if o.position(0)? == "status" {
                o.positions.len().clamp(1, 2)
            } else {
                1
            }
        }
        _ => 0,
    };
    if o.positions.len() != count {
        return Err(Error::new("unexpected_arguments"));
    }
    let mut a = json!({});
    let name = match command {
        "session" => "session_status",
        "categories" => "list_categories",
        "items" => "list_own_items",
        "unread" => "unread_count",
        "search" => {
            a["keywords"] = o.positions.join(" ").into();
            "search"
        }
        "deals" | "watch" => {
            a["keywords"] = o.positions.join(" ").into();
            "find_deals"
        }
        "show" => {
            a["item_id"] = o.position(0)?.into();
            "get_item"
        }
        "seller" => {
            a["user_id"] = o.position(0)?.into();
            if o.get("include-reviews").is_none() {
                a["include_reviews"] = true.into();
            }
            "get_seller"
        }
        "replies" => {
            if o.get("conversation").is_some() {
                "get_conversation"
            } else {
                "list_inbox"
            }
        }
        "purchases" | "selling" => {
            a["tab"] = if o.flag("finished") {
                "finished"
            } else {
                "ongoing"
            }
            .into();
            if command == "purchases" {
                "list_purchases"
            } else {
                "list_sales"
            }
        }
        "message" => {
            if o.get("conversation").is_none() {
                a["item_id"] = o.position(0)?.into();
            }
            "send_message"
        }
        "offer" => match o.position(0)? {
            "accept" | "decline" => {
                a["offer_id"] = o.position(1)?.into();
                a["action"] = o.position(0)?.into();
                "respond_offer"
            }
            "counter" => {
                a["offer_id"] = o.position(1)?.into();
                "counter_offer"
            }
            "status" => {
                if let Some(id) = o.positions.get(1) {
                    a["conversation_id"] = id.clone().into();
                }
                "get_offer_status"
            }
            id => {
                a["item_id"] = id.into();
                "send_offer"
            }
        },
        "edit" | "favorite" | "reserve" | "sold" | "delete" => {
            a["item_id"] = o.position(0)?.into();
            match command {
                "edit" => "edit_item",
                "favorite" => {
                    a["favorited"] = (!o.flag("remove")).into();
                    "set_favorite"
                }
                "reserve" => {
                    a["reserved"] = (!o.flag("remove")).into();
                    "reserve_item"
                }
                "sold" => {
                    a["sold"] = (!o.flag("undo")).into();
                    "mark_sold"
                }
                _ => {
                    if !o.flag("yes") {
                        return Err(Error::new("delete_requires_yes"));
                    }
                    "delete_item"
                }
            }
        }
        "publish" => {
            let mut paths = std::fs::read_dir(o.position(0)?)
                .map_err(|_| Error::new("photos_directory_required"))?
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| {
                    p.extension().is_some_and(|s| {
                        s.eq_ignore_ascii_case("jpg") || s.eq_ignore_ascii_case("jpeg")
                    })
                })
                .collect::<Vec<_>>();
            paths.sort();
            a["photo_paths"] = paths.iter().map(|p| json!(p.to_string_lossy())).collect();
            "publish_item"
        }
        name if tools::definitions().iter().any(|t| t.name == name) => name,
        _ => return Err(Error::new("unknown_command")),
    };
    let schema = &tools::definitions()
        .iter()
        .find(|t| t.name == name)
        .unwrap()
        .schema;
    for (key, value) in &o.values {
        let mapped = match key.as_str() {
            "category" => "category_id",
            "subcat" => "subcategory_ids",
            "lat" => "latitude",
            "lon" => "longitude",
            "order" => "order_by",
            "ship-only" => "shippable",
            "conversation" => "conversation_id",
            "text" | "msg" => "message",
            "summary" => "title",
            "price" if matches!(name, "send_offer" | "counter_offer") => "amount",
            k => k,
        }
        .replace('-', "_");
        if let Some(s) = schema["properties"].get(&mapped) {
            if a.get(&mapped).is_some() {
                return Err(Error::new("conflicting_options"));
            }
            let value = if mapped == "order_by" {
                match value.as_str() {
                    "relevance" => "most_relevance",
                    "price_asc" => "price_low_to_high",
                    "price_desc" => "price_high_to_low",
                    s => s,
                }
            } else {
                value
            };
            a[&mapped] = if s["type"] == "string" {
                Value::String(value.into())
            } else {
                serde_json::from_str(value).map_err(|_| Error::new("invalid_option_value"))?
            };
        } else if !match key.as_str() {
            "json" => true,
            "csv" | "quiet" => matches!(name, "search" | "find_deals" | "list_own_items"),
            "city" => matches!(name, "search" | "find_deals"),
            "finished" => matches!(command, "purchases" | "selling"),
            "remove" => matches!(command, "favorite" | "reserve"),
            "undo" => command == "sold",
            "yes" => command == "delete",
            "dry-run" => {
                tools::definitions()
                    .iter()
                    .any(|t| t.name == name && t.write)
                    || matches!(command, "deals" | "watch")
            }
            "interval" | "once" => command == "watch",
            "pct" => name == "send_offer" && o.get("price").is_none() && o.get("amount").is_none(),
            "offer-pct" | "msg" | "contacted-file" => matches!(command, "deals" | "watch"),
            _ => false,
        } {
            return Err(Error::new("unknown_option"));
        }
    }
    if let Some(city) = o.get("city") {
        if a.get("latitude").is_some() || a.get("longitude").is_some() {
            return Err(Error::new("conflicting_options"));
        }
        let cities: Value = serde_json::from_str(include_str!("cities.json")).unwrap();
        let city = cities.get(city).ok_or(Error::new("unknown_city"))?;
        a["latitude"] = city[0].clone();
        a["longitude"] = city[1].clone();
    }
    Ok((name, a))
}
