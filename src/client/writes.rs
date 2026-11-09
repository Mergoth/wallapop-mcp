use super::{
    Call, Client,
    publishing::{edit_body, vendor},
};
use crate::{Context, Error, Result, identifier, money, text, uuid};
use crate::transport::query;
use serde_json::{Value, json};

impl Client {
    pub(super) fn write(&self, ctx: &Context, name: &str, a: &Value) -> Result<Value> {
        match name {
            "send_message" => self.message(ctx, a),
            "publish_item" => self.publish(ctx, a),
            "edit_item" => {
                let id = self.resolve(ctx, &text(&a["item_id"]), true)?;
                let v = self.execute(
                    ctx,
                    Call {
                        method: "GET",
                        path: format!("/items/{id}"),
                        auth: true,
                        fresh: true,
                        body: Vec::new(),
                        headers: vendor(),
                    },
                )?;
                let body = edit_body(&v, a)?;
                self.execute(
                    ctx,
                    Call {
                        method: "PUT",
                        path: format!("/items/{id}"),
                        auth: true,
                        fresh: true,
                        body: body.to_string().into_bytes(),
                        headers: vec![
                            ("Accept".into(), "application/vnd.upload-v2+json".into()),
                            ("X-AppVersion".into(), "826800".into()),
                            ("Content-Type".into(), "application/json".into()),
                        ],
                    },
                )
            }
            "respond_offer" | "counter_offer" => {
                let id = identifier(a["offer_id"].as_str().unwrap_or(""))?;
                let (suffix, body) = if name == "respond_offer" {
                    (
                        "status",
                        json!({"status":if a["action"]=="accept" {"ACCEPTED"} else {"DECLINED"}}),
                    )
                } else {
                    (
                        "offer-price",
                        json!({"offer_price_amount":money(&a["amount"])? ,"offer_price_currency":"EUR"}),
                    )
                };
                self.send(
                    ctx,
                    "PATCH",
                    &format!("/delivery/offers/{id}/{suffix}"),
                    &body,
                )
            }
            "send_offer" => {
                let price = money(&a["amount"])?;
                let id = self.resolve(ctx, &text(&a["item_id"]), true)?;
                let item = self.read(ctx, "get_item", &json!({"item_id":id,"fresh":true}))?;
                let currency = item["currency"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .ok_or(Error::new("currency_unavailable"))?;
                let offer = uuid()?;
                let mut body = json!({"offer_id":offer,"offer_price_amount":price,"offer_price_currency":currency,"item_ids":[id]});
                if let Some(m) = a.get("message") {
                    body["message"] = m.clone();
                }
                self.send(ctx, "POST", "/delivery/buyer/offers", &body)
                    .map_err(|e| {
                        if e.unknown {
                            e.partial(json!({"offer_id":offer,"outcome_unknown":true}))
                        } else {
                            e
                        }
                    })
            }
            "set_favorite" | "reserve_item" | "mark_sold" | "delete_item" => {
                let id = self.resolve(ctx, &text(&a["item_id"]), true)?;
                let (method, suffix, body) = match name {
                    "set_favorite" => ("PUT", "/favorite", json!({"favorited":a["favorited"]})),
                    "reserve_item" => ("PUT", "/reserve", json!({"reserved":a["reserved"]})),
                    "mark_sold" => (
                        "PUT",
                        "/sold",
                        if a["sold"] == true {
                            json!({})
                        } else {
                            json!({"sold":false})
                        },
                    ),
                    _ => ("DELETE", "", Value::Null),
                };
                self.send(ctx, method, &format!("/items/{id}{suffix}"), &body)
            }
            "pause_item" => {
                let id = self.resolve(ctx, &text(&a["item_id"]), true)?;
                self.send(ctx, "PUT", &format!("/items/{id}/inactivate"), &Value::Null)
            }
            "save_search" => {
                let params = self.alert_params(ctx, a)?;
                let path = query("/searchalerts/savedsearch", &params);
                self.transport.invalidate();
                let (_, location) = self.authed_with_location(ctx, "POST", &path)?;
                let id = location
                    .as_deref()
                    .and_then(|loc| loc.trim_end_matches('/').rsplit('/').next())
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned);
                Ok(json!({"ok": true, "id": id}))
            }
            "remove_saved_search" => {
                let id = identifier(a["search_id"].as_str().unwrap_or(""))?;
                self.send(
                    ctx,
                    "DELETE",
                    &format!("/searchalerts/savedsearch/{id}"),
                    &Value::Null,
                )
            }
            _ => Err(Error::new("unknown_tool")),
        }
    }
}
