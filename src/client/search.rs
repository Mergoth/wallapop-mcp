use super::{
    Client,
    models::{contains_category, normalize},
};
use crate::{
    Context, Error, Result, cents, find, first, identifier, text,
    transport::{Request, encode, query},
};
use serde_json::{Value, json};
use std::{collections::HashSet, time::Duration};

impl Client {
    pub(super) fn resolve(&self, ctx: &Context, reference: &str, fresh: bool) -> Result<String> {
        let reference = reference.trim();
        if reference.len() == 12
            && reference
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
            && reference.bytes().any(|b| b.is_ascii_lowercase())
        {
            return Ok(reference.into());
        }
        let slug = if reference.contains("://") {
            reference
                .strip_prefix("https://es.wallapop.com/item/")
                .ok_or(Error::new("invalid_item_url"))?
                .split(['?', '#'])
                .next()
                .unwrap_or("")
        } else {
            reference
        };
        identifier(slug)?;
        if slug.contains("..") {
            return Err(Error::new("invalid_item_reference"));
        }
        let url = format!("{}/item/{}", self.web, encode(slug));
        let r = self.transport.request(
            ctx,
            Request {
                method: "GET",
                url: &url,
                headers: &[],
                body: &[],
                read: true,
                ttl: if fresh {
                    Duration::ZERO
                } else {
                    Duration::from_secs(30)
                },
            },
        )?;
        let html = std::str::from_utf8(&r.body).map_err(|_| Error::new("invalid_page_data"))?;
        let marker = html
            .find("__NEXT_DATA__")
            .ok_or(Error::new("missing_page_data"))?;
        let start = html[marker..]
            .find('>')
            .map(|i| marker + i + 1)
            .ok_or(Error::new("invalid_page_data"))?;
        let end = html[start..]
            .find("</script>")
            .map(|i| start + i)
            .ok_or(Error::new("invalid_page_data"))?;
        let v: Value =
            serde_json::from_str(&html[start..end]).map_err(|_| Error::new("invalid_page_data"))?;
        Ok(identifier(v["props"]["pageProps"]["item"]["id"].as_str().unwrap_or(""))?.to_string())
    }

    pub(super) fn search(&self, ctx: &Context, a: &Value) -> Result<Value> {
        let fresh = a["fresh"] == true;
        if let (Some(lo), Some(hi)) = (a["min_price"].as_str(), a["max_price"].as_str()) {
            if cents(lo)? > cents(hi)? {
                return Err(Error::new("invalid_price_range"));
            }
        }
        let mut q = vec![
            ("keywords".into(), text(&a["keywords"])),
            ("source".into(), "search_box".into()),
            ("filters_source".into(), "search_box".into()),
            (
                "order_by".into(),
                text(&a["order_by"]).nonempty("most_relevance"),
            ),
            ("search_country".into(), "ES".into()),
            ("latitude".into(), text(&a["latitude"]).nonempty("40.0")),
            ("longitude".into(), text(&a["longitude"]).nonempty("-3.5")),
        ];
        if a.get("category_id").is_some() || a.get("subcategory_ids").is_some() {
            let cats = self.read(ctx, "list_categories", &json!({"fresh":fresh}))?;
            let leaf = first(a, &["category_id", "subcategory_ids"]);
            let root = cats["categories"]
                .as_array()
                .unwrap()
                .iter()
                .find(|v| contains_category(v, &leaf))
                .ok_or(Error::new("category_not_found"))?;
            q.push(("category_id".into(), text(&root["id"])));
            if let Some(sub) = a.get("subcategory_ids") {
                if !contains_category(root, sub) {
                    return Err(Error::new("subcategory_not_in_category"));
                }
                q.push(("subcategory_ids".into(), text(sub)));
            }
        }
        for (arg, key) in [
            ("condition", "condition"),
            ("min_price", "min_sale_price"),
            ("max_price", "max_sale_price"),
        ] {
            if let Some(v) = a.get(arg) {
                q.push((key.into(), text(v)));
            }
        }
        if a["shippable"] == true {
            q.push(("is_shippable".into(), "true".into()));
        }
        let v = self.get(ctx, &query("/search/components", &q), false, fresh)?;
        if !v.is_object() {
            return Err(Error::new("invalid_response"));
        }
        let sid = find(&v, "search_id").map(text).unwrap_or_default();
        let mut items = Vec::new();
        let mut seen = HashSet::new();
        let mut cursors = HashSet::new();
        let mut pages = 0;
        let mut more = false;
        let limit = a["limit"].as_u64().unwrap_or(20) as usize;
        let max_pages = a["pages"].as_u64().unwrap_or(1);
        if sid.is_empty() {
            q.retain(|(k, _)| k != "latitude" && k != "longitude");
            for (k, _) in &mut q {
                if k == "category_id" {
                    *k = "category_ids".into();
                }
            }
            let v = self.get(ctx, &query("/general/search", &q), false, fresh)?;
            let raw = v["search_objects"]
                .as_array()
                .ok_or(Error::new("invalid_response"))?;
            for v in raw {
                let item = normalize(v, &self.web, true)?;
                if seen.insert(text(&item["id"])) {
                    if items.len() == limit {
                        more = true;
                        break;
                    }
                    items.push(item);
                }
            }
            pages = 1;
        } else {
            q.retain(|(k, _)| k != "source");
            q.extend([
                ("source".into(), "deep_link".into()),
                ("search_id".into(), sid),
                ("section_type".into(), "organic_search_results".into()),
            ]);
            for hop in 0..20 {
                let v = self.get(ctx, &query("/search/section", &q), false, fresh)?;
                let raw = find(&v, "items")
                    .and_then(Value::as_array)
                    .ok_or(Error::new("invalid_response"))?;
                if !raw.is_empty() {
                    pages += 1;
                }
                let mut truncated = false;
                for v in raw {
                    let item = normalize(v, &self.web, true)?;
                    if seen.insert(text(&item["id"])) {
                        if items.len() == limit {
                            truncated = true;
                            break;
                        }
                        items.push(item);
                    }
                }
                let next = find(&v, "next_page").filter(|v| !v.is_null());
                if next.is_some_and(|v| !v.is_string()) {
                    return Err(Error::new("invalid_response"));
                }
                let next = next.map(text).unwrap_or_default();
                more = truncated || !next.is_empty();
                if next.is_empty() || truncated || pages >= max_pages || items.len() >= limit {
                    break;
                }
                if !cursors.insert(next.clone()) {
                    return Err(Error::new("cursor_loop"));
                }
                if hop == 19 {
                    return Err(Error::new("pagination_limit"));
                }
                q.retain(|(k, _)| k != "next_page");
                q.push(("next_page".into(), next));
            }
        }
        Ok(json!({"count":items.len(),"items":items,"pages_fetched":pages,"has_more":more}))
    }
}

impl Client {
    /// Build query params for a saved-search alert (same shape the web POSTs to searchalerts).
    pub(super) fn alert_params(
        &self,
        ctx: &Context,
        a: &Value,
    ) -> Result<Vec<(String, String)>> {
        let fresh = a["fresh"] == true;
        if let (Some(lo), Some(hi)) = (a["min_price"].as_str(), a["max_price"].as_str()) {
            if cents(lo)? > cents(hi)? {
                return Err(Error::new("invalid_price_range"));
            }
        }
        let mut q: Vec<(String, String)> = vec![
            ("keywords".into(), text(&a["keywords"])),
            ("source".into(), "search_box".into()),
            ("filters_source".into(), "search_box".into()),
            (
                "order_by".into(),
                text(&a["order_by"]).nonempty("most_relevance"),
            ),
        ];
        if a.get("category_id").is_some() || a.get("subcategory_ids").is_some() {
            let cats = self.read(ctx, "list_categories", &json!({"fresh":fresh}))?;
            let leaf = first(a, &["category_id", "subcategory_ids"]);
            let root = cats["categories"]
                .as_array()
                .unwrap()
                .iter()
                .find(|v| contains_category(v, &leaf))
                .ok_or(Error::new("category_not_found"))?;
            q.push(("category_id".into(), text(&root["id"])));
            if let Some(sub) = a.get("subcategory_ids") {
                if !contains_category(root, sub) {
                    return Err(Error::new("subcategory_not_in_category"));
                }
                q.push(("subcategory_ids".into(), text(sub)));
            }
        }
        for (arg, key) in [
            ("condition", "condition"),
            ("min_price", "min_sale_price"),
            ("max_price", "max_sale_price"),
        ] {
            if let Some(v) = a.get(arg) {
                q.push((key.into(), text(v)));
            }
        }
        if a["shippable"] == true {
            q.push(("is_shippable".into(), "true".into()));
        }
        if let (Some(lat), Some(lon)) = (a.get("latitude"), a.get("longitude")) {
            q.push(("latitude".into(), text(lat)));
            q.push(("longitude".into(), text(lon)));
        }
        Ok(q)
    }
}

pub(super) trait Nonempty {
    fn nonempty(self, default: &str) -> String;
}
impl Nonempty for String {
    fn nonempty(self, default: &str) -> String {
        if self.is_empty() {
            default.into()
        } else {
            self
        }
    }
}
