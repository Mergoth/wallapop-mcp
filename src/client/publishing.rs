use super::{Call, Client, models::attribute, read_file};
use crate::{Context, Error, Result, first, identifier, money, text, uuid};
use crate::transport::MAX_BODY;
use serde_json::{Value, json};
use std::path::Path;

fn fetch_photo(path: &str) -> crate::Result<Vec<u8>> {
    if path.starts_with("http://") || path.starts_with("https://") {
        use std::io::Read as _;
        let mut resp = ureq::get(path)
            .call()
            .map_err(|_| Error::new("photo_fetch_failed"))?;
        if resp.status() != 200 {
            return Err(Error::new("photo_fetch_failed"));
        }
        let mut buf = Vec::new();
        resp.body_mut()
            .as_reader()
            .take(MAX_BODY as u64 + 1)
            .read_to_end(&mut buf)
            .map_err(|_| Error::new("photo_fetch_failed"))?;
        if buf.len() > MAX_BODY {
            return Err(Error::new("photo_too_large"));
        }
        Ok(buf)
    } else {
        read_file(Path::new(path), 8 << 20)
    }
}

impl Client {
    pub(super) fn publish(&self, ctx: &Context, a: &Value) -> Result<Value> {
        for key in ["title", "description", "color"] {
            if a[key].as_str().unwrap_or("").trim().is_empty() {
                return Err(Error::new("listing_field_required"));
            }
        }
        numeric_category(&a["category_leaf_id"])?;
        let price = money(&a["price"])?;
        let shipping = a["shipping_allowed"] == true;
        if shipping && a.get("max_weight_kg").is_none() {
            return Err(Error::new("shipping_weight_required"));
        }
        // Validate every photo before creating the listing, including byte limits.
        let mut photos = Vec::new();
        let mut total = 0;
        for path in a["photo_paths"]
            .as_array()
            .ok_or(Error::new("photos_required"))?
        {
            let b = fetch_photo(path.as_str().unwrap_or(""))?;
            if !b.starts_with(&[255, 216, 255]) || b.len() < 4 {
                return Err(Error::new("jpeg_required"));
            }
            total += b.len();
            if total > 32 << 20 {
                return Err(Error::new("photos_exceed_32_mib"));
            }
            photos.push(b);
        }
        let me = self.get(ctx, "/users/me", true, true)?;
        let lat = &me["location"]["approximated_latitude"];
        let lon = &me["location"]["approximated_longitude"];
        location(lat, lon)?;
        let mut delivery = json!({"allowed_by_user":shipping});
        if shipping {
            let mut cost = text(&a["cost_configuration_id"]);
            if cost.is_empty() {
                let id = text(&me["id"]);
                identifier(&id)?;
                let owned = self.get(
                    ctx,
                    &format!("/users/{id}/items?page=0&max_items=25"),
                    true,
                    true,
                )?;
                cost = owned["data"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|v| text(&v["shipping"]["cost_configuration_id"]))
                    .find(|s| !s.is_empty())
                    .unwrap_or_default();
            }
            if cost.is_empty() {
                return Err(Error::new("cost_configuration_id_required"));
            }
            delivery["max_weight_kg"] = a["max_weight_kg"].clone();
            delivery["cost_configuration_id"] = cost.into();
        }
        let mut attributes = crate::pick(a, &["title", "description", "condition", "color"]);
        attributes["brand"] = a.get("brand").cloned().unwrap_or(json!(""));
        attributes["model"] = a.get("model").cloned().unwrap_or(json!(""));
        attributes["price_amount"] = price;
        attributes["suggested_data_banner"] = Value::Null;
        let body = json!({
            "attributes": attributes, "category_leaf_id": a["category_leaf_id"],
            "apply_discount": false, "delivery": delivery, "upload_id": uuid()?,
            "location": {
                "latitude": lat, "longitude": lon,
                "approximated": me["location"]["approximated_location"] == true
            }
        });
        let (bytes, content_type) = multipart(&photos[0], "item", &body.to_string())?;
        let v = self.upload(ctx, "/items", bytes, content_type)?;
        let id = text(&v["id"]);
        if identifier(&id).is_err() {
            return Err(Error::new("missing_created_item_id").unknown());
        }
        let mut out = json!({
            "item_id": id, "completed_photos": 1, "failed_photos": [],
            "failed_photos_count": 0, "partial": false, "unknown_outcome": false
        });
        for (i, photo) in photos.iter().enumerate().skip(1) {
            let result = multipart(photo, "order", &(i + 1).to_string())
                .and_then(|(b, ct)| self.upload(ctx, &format!("/items/{id}/picture2"), b, ct));
            if let Err(e) = result {
                out["failed_photos"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!(i + 1));
                if e.unknown {
                    out["unknown_outcome"] = true.into();
                }
                out["partial"] = true.into();
                if matches!(e.code, "timeout" | "cancelled") {
                    for j in i + 1..photos.len() {
                        out["failed_photos"]
                            .as_array_mut()
                            .unwrap()
                            .push(json!(j + 1));
                    }
                    out["failed_photos_count"] =
                        out["failed_photos"].as_array().unwrap().len().into();
                    return Err(e.partial(out));
                }
            } else {
                out["completed_photos"] = (out["completed_photos"].as_u64().unwrap() + 1).into();
            }
        }
        out["failed_photos_count"] = out["failed_photos"].as_array().unwrap().len().into();
        Ok(out)
    }

    fn upload(
        &self,
        ctx: &Context,
        path: &str,
        body: Vec<u8>,
        content_type: String,
    ) -> Result<Value> {
        let mut headers = vendor();
        headers.push(("Content-Type".into(), content_type));
        self.execute(
            ctx,
            Call {
                method: "POST",
                path: path.into(),
                body,
                auth: true,
                fresh: true,
                headers,
            },
        )
    }
}

pub(super) fn vendor() -> Vec<(String, String)> {
    vec![("Accept".into(), "application/vnd.upload-v2+json".into())]
}

fn numeric_category(v: &Value) -> Result<()> {
    if v.as_str()
        .is_none_or(|s| s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()))
    {
        return Err(Error::new("numeric_category_required"));
    }
    Ok(())
}

fn location(lat: &Value, lon: &Value) -> Result<()> {
    if !lat.as_f64().is_some_and(|n| (-90.0..=90.0).contains(&n))
        || !lon.as_f64().is_some_and(|n| (-180.0..=180.0).contains(&n))
    {
        return Err(Error::new("location_unavailable"));
    }
    Ok(())
}

fn multipart(photo: &[u8], field: &str, value: &str) -> Result<(Vec<u8>, String)> {
    let boundary = format!("wallapop-{}", uuid()?);
    let mut b =
        Vec::with_capacity(photo.len() + value.len() + field.len() + boundary.len() * 3 + 256);
    b.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"image\"; filename=\"photo.jpg\"\r\nContent-Type: image/jpeg\r\n\r\n").as_bytes());
    b.extend_from_slice(photo);
    b.extend_from_slice(format!("\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"{field}\"\r\n\r\n{value}\r\n--{boundary}--\r\n").as_bytes());
    Ok((b, format!("multipart/form-data; boundary={boundary}")))
}

pub(super) fn edit_body(v: &Value, a: &Value) -> Result<Value> {
    let mut attributes = json!({});
    for key in [
        "brand",
        "model",
        "color",
        "title",
        "description",
        "condition",
    ] {
        let value = a.get(key).cloned().unwrap_or_else(|| attribute(v, key));
        if !value.is_string() {
            return Err(Error::new("current_attribute_unavailable"));
        }
        if matches!(key, "title" | "description" | "color")
            && value.as_str().unwrap().trim().is_empty()
        {
            return Err(Error::new("listing_field_required"));
        }
        attributes[key] = value;
    }
    let price = a.get("price").cloned().unwrap_or_else(|| {
        let p = v
            .pointer("/price/cash/amount")
            .or_else(|| v.pointer("/price/amount"))
            .or_else(|| v.get("price_amount"));
        p.map(|v| Value::String(text(v))).unwrap_or(Value::Null)
    });
    attributes["price_amount"] = money(&price)?;
    let category = a.get("category_leaf_id").cloned().unwrap_or_else(|| {
        v.get("category_leaf_id").cloned().unwrap_or_else(|| {
            v["taxonomy"]
                .as_array()
                .and_then(|a| a.last())
                .map(|v| Value::String(text(&first(v, &["id", "category_id"]))))
                .unwrap_or(Value::Null)
        })
    });
    numeric_category(&category)?;
    let ship = &v["shipping"];
    let allowed = a
        .get("shipping_allowed")
        .cloned()
        .unwrap_or_else(|| first(ship, &["user_allows_shipping", "allowed_by_user"]));
    let allowed = allowed
        .as_bool()
        .ok_or(Error::new("current_shipping_permission_unavailable"))?;
    let mut delivery = json!({"allowed_by_user":allowed});
    if allowed {
        let weight = a
            .get("max_weight_kg")
            .or_else(|| ship.get("max_weight_kg"))
            .or_else(|| v["type_attributes"].get("up_to_kg"));
        let weight = weight
            .and_then(|v| text(v).parse::<f64>().ok())
            .filter(|n| n.fract() == 0.0 && (1.0..=30.0).contains(n))
            .ok_or(Error::new("current_shipping_weight_unavailable"))?;
        let cost = a
            .get("cost_configuration_id")
            .unwrap_or(&ship["cost_configuration_id"]);
        if cost.as_str().is_none_or(str::is_empty) {
            return Err(Error::new("cost_configuration_id_required"));
        }
        delivery["max_weight_kg"] = (weight as u64).into();
        delivery["cost_configuration_id"] = cost.clone();
    }
    let lat = first(&v["location"], &["latitude", "lat"]);
    let lon = first(&v["location"], &["longitude", "lon"]);
    location(&lat, &lon)?;
    let images = v["images"]
        .as_array()
        .filter(|a| !a.is_empty())
        .ok_or(Error::new("current_images_unavailable"))?;
    let pictures = images
        .iter()
        .enumerate()
        .map(|(i, p)| Ok(json!({"id":identifier(p["id"].as_str().unwrap_or(""))?,"order":i})))
        .collect::<Result<Vec<_>>>()?;
    Ok(
        json!({"attributes":attributes,"category_leaf_id":category,"apply_discount":v["apply_discount"]==true,
        "location":{"latitude":lat,"longitude":lon,"approximated":v["location"]["approximated"]==true},"delivery":delivery,"pictures":pictures}),
    )
}
