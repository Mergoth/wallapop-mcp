use super::{Client, claims, models::messages};
use crate::{
    Context, Error, Result, find, first, identifier, text,
    transport::{Request, encode, query},
    uuid,
};
use serde_json::{Value, json};
use std::time::Duration;

impl Client {
    pub(super) fn message(&self, ctx: &Context, a: &Value) -> Result<Value> {
        let message = a["message"].as_str().unwrap_or("");
        if message.trim().is_empty() {
            return Err(Error::new("message_required"));
        }
        let cid = if let Some(cid) = a["conversation_id"].as_str() {
            identifier(cid)?.to_string()
        } else {
            let id = self.resolve(ctx, &text(&a["item_id"]), true)?;
            let v = self.send(
                ctx,
                "POST",
                "/instant-messaging/conversation",
                &json!({"item_hash_id":id}),
            )?;
            let cid = find(&v, "conversation_id").map(text).unwrap_or_default();
            identifier(&cid)?.to_string()
        };
        let thread = self.get(
            ctx,
            &format!("/bff/messaging/conversation/{cid}"),
            true,
            true,
        )?;
        if messages(&thread)?
            .iter()
            .rev()
            .find(|v| v["from_self"] == true)
            .is_some_and(|v| v["text"] == message)
        {
            return Ok(json!({"conversation_id":cid,"status":"already_sent"}));
        }
        let channel = thread["channel"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or(Error::new("delivery_metadata_unavailable"))?;
        let to = thread["with_user"]["hash"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or(Error::new("delivery_metadata_unavailable"))?;
        let tok = self.get(ctx, "/instant-messaging/token", true, true)?;
        let auth = text(&first(&tok["data"], &["token"]));
        let auth = if auth.is_empty() {
            text(&tok["token"])
        } else {
            auth
        };
        if auth.is_empty() {
            return Err(Error::new("delivery_token_unavailable"));
        }
        let from = text(&claims(&self.session.lock().unwrap().access)["hashed_sub"]);
        if from.is_empty() {
            return Err(Error::new("delivery_metadata_unavailable"));
        }
        let payload = json!({"id":uuid()?,"payload":{"text":message}}).to_string();
        let meta =
            json!({"type":"text","sender":{"platform":{"app_version":"8.2656.0","os_version":"0"}},
            "to_user_hash":to,"from_user_hash":from,"conversation_hash":cid})
            .to_string();
        let url = query(
            &format!(
                "{}/publish/pub-c-255dc549-86f5-4abd-8b9e-921d5a02fde7/sub-c-89405e27-d4df-4d87-aca1-d6e9118f0a0d/0/{}/0/{}",
                self.pubnub,
                encode(channel),
                encode(&payload)
            ),
            &[
                ("meta".into(), meta),
                ("uuid".into(), from),
                ("requestid".into(), uuid()?),
                ("pnsdk".into(), "PubNub-JS-Web/10.2.6".into()),
                ("auth".into(), auth),
            ],
        );
        let result = self.transport.request(
            ctx,
            Request {
                method: "GET",
                url: &url,
                headers: &[],
                body: &[],
                read: false,
                ttl: Duration::ZERO,
            },
        );
        let ack = result
            .ok()
            .and_then(|r| serde_json::from_slice::<Value>(&r.body).ok());
        if !ack.is_some_and(|v| acknowledged(&v)) {
            return Err(Error::new("delivery_unknown")
                .unknown()
                .partial(json!({"conversation_id":cid,"outcome_unknown":true})));
        }
        Ok(json!({"conversation_id":cid,"status":"sent"}))
    }
}

pub fn acknowledged(v: &Value) -> bool {
    if let Some(s) = v.as_str() {
        return serde_json::from_str::<Value>(s)
            .is_ok_and(|v| v.is_array() && v[0] == 1 && v[1] == "Sent");
    }
    v.is_array() && v[0] == 1 && v[1] == "Sent"
}
