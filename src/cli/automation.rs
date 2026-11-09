use super::{args::Options, contacts::Contacts, duration, output};
use crate::{Context, Error, Result, client::Client, money, text};
use serde_json::{Value, json};
use std::{collections::HashSet, time::Duration};

pub(super) fn percentage(price: &Value, fraction: &str) -> Result<String> {
    let fraction = offer_fraction(fraction)?;
    let cents = crate::cents(&text(price))?;
    let value = format!("{:.2}", (cents as f64 * fraction).round() / 100.0);
    money(&json!(value))?;
    Ok(value)
}

fn offer_fraction(s: &str) -> Result<f64> {
    s.parse::<f64>()
        .ok()
        .filter(|n| n.is_finite() && *n > 0.0 && *n <= 1.0)
        .ok_or(Error::new("invalid_offer_fraction"))
}

pub(super) fn validate(client: &Client, o: &Options) -> Result<()> {
    for key in ["pct", "offer-pct"] {
        if let Some(value) = o.get(key) {
            offer_fraction(value)?;
        }
    }
    if (o.get("offer-pct").is_some() || o.get("msg").is_some())
        && !o.flag("dry-run")
        && !client.allow_writes
    {
        return Err(Error::new("writes_disabled"));
    }
    Ok(())
}

pub(super) fn act(client: &Client, o: &Options, result: &Value, timeout: Duration) -> Result<()> {
    if o.get("offer-pct").is_none() && o.get("msg").is_none() {
        return Ok(());
    }
    let mut contacts = Contacts::load(o.get("contacted-file"))?;
    for item in result["items"].as_array().into_iter().flatten() {
        let id = text(&item["id"]);
        if contacts.contains(&id) {
            continue;
        }
        if let Some((name, args)) = action(item, o)? {
            if o.flag("dry-run") {
                eprintln!("{}", json!({"dry_run":true,"tool":name,"arguments":args}));
            } else {
                crate::tools::definitions()
                    .iter()
                    .find(|t| t.name == name)
                    .unwrap()
                    .validate(&args)?;
                contacts.record(&id)?;
                client.invoke(&Context::new(timeout), name, &args)?;
            }
        }
    }
    Ok(())
}

fn action(item: &Value, o: &Options) -> Result<Option<(&'static str, Value)>> {
    let mut args = json!({"item_id":item["id"]});
    let name = if let Some(pct) = o.get("offer-pct") {
        args["amount"] = percentage(&item["price"], pct)?.into();
        "send_offer"
    } else if o.get("msg").is_some() {
        "send_message"
    } else {
        return Ok(None);
    };
    if let Some(template) = o.get("msg") {
        let mut message = template.to_string();
        for (key, value) in [
            ("title", &item["title"]),
            ("price", &item["price"]),
            ("link", &item["url"]),
            ("city", &item["city"]),
            ("offer", &args["amount"]),
        ] {
            message = message.replace(&format!("{{{key}}}"), &text(value));
        }
        args["message"] = message.into();
    }
    Ok(Some((name, args)))
}

pub(super) fn watch(client: &Client, o: &Options, mut a: Value, timeout: Duration) -> Result<()> {
    let interval = duration(o.get("interval").unwrap_or("300"))?;
    if interval < Duration::from_secs(5) {
        return Err(Error::new("watch_interval_minimum_5s"));
    }
    let mut seen = HashSet::new();
    a["fresh"] = true.into();
    loop {
        let mut result = client.invoke(&Context::new(timeout), "find_deals", &a)?;
        result["items"]
            .as_array_mut()
            .unwrap()
            .retain(|v| seen.insert(text(&v["id"])));
        result["count"] = result["items"].as_array().unwrap().len().into();
        act(client, o, &result, timeout)?;
        output(&result, o);
        if o.flag("once") {
            return Ok(());
        }
        if seen.len() > 10000 {
            return Err(Error::new("watch_seen_limit_restart_required"));
        }
        std::thread::sleep(interval);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn auto_offer_attaches_message_as_one_action_and_preserves_decimal_cents() {
        let options = Options::parse(
            &[
                "--offer-pct",
                "0.8",
                "--msg",
                "{title}: {offer} EUR ({price})",
            ]
            .map(String::from),
        )
        .unwrap();
        let (name, args) = action(
            &json!({"id":"item", "title":"Camera", "price":19.95}),
            &options,
        )
        .unwrap()
        .unwrap();
        assert_eq!(name, "send_offer");
        assert_eq!(args["amount"], "15.96");
        assert_eq!(args["message"], "Camera: 15.96 EUR (19.95)");
    }
    #[test]
    fn message_only_and_no_automatic_action() {
        let options = Options::parse(&["--msg", "Hola, {title}"].map(String::from)).unwrap();
        let (name, args) = action(&json!({"id":"item", "title":"Camera"}), &options)
            .unwrap()
            .unwrap();
        assert_eq!(name, "send_message");
        assert_eq!(args["message"], "Hola, Camera");
        assert!(action(&json!({}), &Options::default()).unwrap().is_none());
    }
}
