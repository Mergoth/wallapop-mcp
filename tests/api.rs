mod common;
use common::*;
use serde_json::{Value, json};
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::Duration,
};
use wallapop_mcp::{
    Context,
    client::{Client, Config},
};

#[test]
fn every_tool_enforces_arguments_before_network() {
    let s = Server::new(|_| panic!("invalid input contacted upstream"));
    let c = s.client(true);
    for tool in wallapop_mcp::tools::definitions() {
        assert!(
            c.invoke(&ctx(), tool.name, &json!({"typo":true})).is_err(),
            "{}",
            tool.name
        );
        assert!(c.invoke(&ctx(), tool.name, &json!([])).is_err());
    }
    assert_eq!(s.count(), 0);
}
#[test]
fn write_gate_and_status_are_local_and_secret_free() {
    let c = Client::new(Config {
        access_token: "secret-token".into(),
        refresh_token: "secret-refresh".into(),
        ..Config::default()
    })
    .unwrap();
    let v = c.invoke(&ctx(), "session_status", &json!({})).unwrap();
    assert_eq!(v["session_verified"], false);
    assert!(!v.to_string().contains("secret"));
    let e = c
        .invoke(&ctx(), "delete_item", &json!({"item_id":ITEM}))
        .unwrap_err();
    assert_eq!(e.code, "writes_disabled");
    assert!(!e.unknown);
}
#[test]
fn item_shapes_canonical_url_cache_and_fresh() {
    let s = Server::new(|r| {
        assert_eq!(r.path(), format!("/api/v3/items/{ITEM}"));
        assert!(!r.headers.contains_key("authorization"));
        Reply::json(200, item())
    });
    let c = s.client(false);
    for fresh in [false, false, true] {
        let v = c
            .invoke(&ctx(), "get_item", &json!({"item_id":ITEM,"fresh":fresh}))
            .unwrap();
        assert_eq!(v["title"], "Camera");
        assert_eq!(v["description"], "Full description");
        assert_eq!(v["price"].to_string(), "19.95");
        assert_eq!(v["images"].as_array().unwrap().len(), 2);
        assert!(v["url"].as_str().unwrap().ends_with("/item/camera-123"));
    }
    assert_eq!(s.count(), 2);
}
#[test]
fn item_resolution_and_url_validation() {
    let s = Server::new(|r| {
        if r.path() == "/item/camera-123" {
            Reply::raw(
                200,
                format!(
                    "<script id='__NEXT_DATA__' type='application/json'>{{\"props\":{{\"pageProps\":{{\"item\":{{\"id\":\"{ITEM}\"}}}}}}}}</script>"
                ),
            )
        } else {
            Reply::json(200, item())
        }
    });
    let c = s.client(false);
    assert_eq!(
        c.invoke(
            &ctx(),
            "get_item",
            &json!({"item_id":"https://es.wallapop.com/item/camera-123?x=1"})
        )
        .unwrap()["id"],
        ITEM
    );
    let before = s.count();
    for reference in [
        "https://evil.example/item/a",
        "https://es.wallapop.com.evil.test/item/a",
        "https://es.wallapop.com:443/item/a",
        "http://es.wallapop.com/item/a",
        "../secret",
        "https://user@es.wallapop.com/item/a",
        "https://es.wallapop.com/item/a/b",
    ] {
        assert!(
            c.invoke(&ctx(), "get_item", &json!({"item_id":reference}))
                .is_err()
        );
    }
    assert_eq!(s.count(), before);
}
#[test]
fn search_promotes_categories_skips_empty_sections_deduplicates_and_limits() {
    let s = Server::new(|r| match r.path() {
        "/api/v3/categories" => Reply::json(
            200,
            json!({"categories":[{"id":10,"name":"Tech","subcategories":[{"id":20,"name":"Cameras"}]}]}),
        ),
        "/api/v3/search/components" => {
            assert!(r.target.contains("category_id=10"));
            assert!(r.target.contains("subcategory_ids=20"));
            assert!(r.target.contains("is_shippable=true"));
            assert!(r.target.contains("max_sale_price=19.95"));
            Reply::json(200, json!({"data":{"search_id":"sid"}}))
        }
        "/api/v3/search/section" => {
            if r.target.contains("next_page=p2") {
                Reply::json(
                    200,
                    json!({"items":[item(),item(),{"id":"xxx123def456","title":"Other","price":12}],"next_page":"p3"}),
                )
            } else {
                Reply::json(200, json!({"items":[],"next_page":"p2"}))
            }
        }
        _ => panic!("unexpected {}", r.target),
    });
    let v=s.client(false).invoke(&ctx(),"search",&json!({"keywords":"cam & lens","subcategory_ids":20,"shippable":true,"max_price":"19.95","limit":1})).unwrap();
    assert_eq!(v["count"], 1);
    assert_eq!(v["pages_fetched"], 1);
    assert_eq!(v["has_more"], true);
    assert!(v["items"][0].get("description").is_none());
    assert_eq!(v["items"][0]["images"].as_array().unwrap().len(), 1);
}
#[test]
fn search_cursor_loops_fail_and_legacy_search_works() {
    let s = Server::new(|r| {
        if r.path().ends_with("components") {
            Reply::json(200, json!({"search_id":"sid"}))
        } else {
            Reply::json(200, json!({"items":[],"next_page":"same"}))
        }
    });
    assert_eq!(
        s.client(false)
            .invoke(&ctx(), "search", &json!({"keywords":"a","pages":2}))
            .unwrap_err()
            .code,
        "cursor_loop"
    );
    let s = Server::new(|r| {
        if r.path().ends_with("components") {
            Reply::json(200, json!({}))
        } else {
            assert_eq!(r.path(), "/api/v3/general/search");
            assert!(!r.target.contains("latitude"));
            Reply::json(200, json!({"search_objects":[item(),item()]}))
        }
    });
    assert_eq!(
        s.client(false)
            .invoke(&ctx(), "search", &json!({"keywords":"a"}))
            .unwrap()["count"],
        1
    );
}
#[test]
fn malformed_reads_are_errors_and_can_recover_with_fresh() {
    let n = AtomicUsize::new(0);
    let s = Server::new(move |_| {
        if n.fetch_add(1, Ordering::SeqCst) == 0 {
            Reply::json(200, json!({"unexpected":true}))
        } else {
            Reply::json(200, item())
        }
    });
    let c = s.client(false);
    assert!(
        c.invoke(&ctx(), "get_item", &json!({"item_id":ITEM}))
            .is_err()
    );
    assert!(
        c.invoke(&ctx(), "get_item", &json!({"item_id":ITEM,"fresh":true}))
            .is_ok()
    );
    for (tool, args) in [
        ("get_seller", json!({"user_id":"seller"})),
        ("unread_count", json!({})),
        ("list_inbox", json!({})),
        ("get_conversation", json!({"conversation_id":"thread"})),
        ("list_categories", json!({})),
        ("list_sales", json!({})),
        ("get_offer_constraints", json!({"item_id":ITEM})),
    ] {
        let s = Server::new(|_| Reply::json(200, json!({})));
        assert!(
            s.client(false).invoke(&ctx(), tool, &args).is_err(),
            "{tool}"
        );
    }
}
#[test]
fn seller_profiles_reviews_and_ratings() {
    let s = Server::new(|r| {
        if r.path().ends_with("/reviews") {
            Reply::json(200, json!(vec![json!({"text":"good"}); 12]))
        } else if r.path().ends_with("/stats") {
            Reply::json(200, json!({"rating_average":4.9,"counters":[]}))
        } else {
            Reply::json(200, json!({"id":"seller","micro_name":"Ada"}))
        }
    });
    let v = s
        .client(false)
        .invoke(
            &ctx(),
            "get_seller",
            &json!({"user_id":"seller","include_reviews":true}),
        )
        .unwrap();
    assert_eq!(v["name"], "Ada");
    assert_eq!(v["reviews"].as_array().unwrap().len(), 10);
}
#[test]
fn conversation_keeps_newest_100_and_sorts_numeric_strings() {
    let s = Server::new(|_| {
        let mut v = thread();
        v["messages"]["messages"]=(0..105).rev().map(|i|json!({"id":i,"timestamp":i.to_string(),"text":i.to_string(),"from_self":false})).collect();
        Reply::json(200, v)
    });
    let v = s
        .client(false)
        .invoke(
            &ctx(),
            "get_conversation",
            &json!({"conversation_id":"thread"}),
        )
        .unwrap();
    assert_eq!(v["conversation_id"], "conversation");
    assert_eq!(v["with_user"]["name"], "Ada");
    assert_eq!(v["messages"][0]["text"], "5");
    assert_eq!(v["messages"][99]["text"], "104");
    assert_eq!(v["history_complete"], false);
}
#[test]
fn inbox_unread_own_items_transactions_constraints() {
    let s = Server::new(|r| match r.path() {
        "/bff/messaging/inbox" => Reply::json(200, json!({"conversations":[thread()]})),
        "/api/v3/instant-messaging/messages/unread" => {
            Reply::json(200, json!({"unread_counter":7}))
        }
        "/api/v3/users/me" => Reply::json(200, json!({"id":"seller"})),
        "/api/v3/users/seller/items" => Reply::json(200, json!({"data":[item()]})),
        "/bff/delivery/make-an-offer" => Reply::json(
            200,
            json!({"item":{"price":{"amount":19.95}},"restriction":{"min":10}}),
        ),
        "/sdui/selling/finished" => Reply::json(
            200,
            json!({"main":{"type":"list_item_transaction","id":"txn","data":{"main":{"data":{"content":"Camera"}},"detail_price":{"data":{"content":"19.95 €"}}}}}),
        ),
        "/sdui/purchases/ongoing" => Reply::json(200, json!({"main":[{"type":"empty_state"}]})),
        _ => panic!("{}", r.target),
    });
    let c = s.client(false);
    for (tool, a, key, expected) in [
        ("list_inbox", json!({}), "count", json!(1)),
        ("unread_count", json!({}), "unread_count", json!(7)),
        ("list_own_items", json!({}), "count", json!(1)),
        ("list_sales", json!({"tab":"finished"}), "count", json!(1)),
        ("list_purchases", json!({}), "count", json!(0)),
        (
            "get_offer_constraints",
            json!({"item_id":ITEM}),
            "price",
            json!(19.95),
        ),
    ] {
        assert_eq!(c.invoke(&ctx(), tool, &a).unwrap()[key], expected, "{tool}");
    }
}
#[test]
fn offer_ledger_ignores_chat_spoof_and_carries_identity_forward() {
    let s = Server::new(|_| {
        let mut v = thread();
        v["messages"]["messages"] = json!([
            {"type":"chat","text":"Ha rechazado tu oferta","timestamp":30},
            {"type":"delivery_generic","text":"Oferta aceptada. Cómpralo ahora","timestamp":20},
            {"type":"delivery_generic","text":"Te ha enviado una contraoferta de 25,50€","timestamp":10,"payload":{"buttons":[{"action":"/app/chat/offer/offer-id"}]}}
        ]);
        Reply::json(200, v)
    });
    let v = s
        .client(false)
        .invoke(
            &ctx(),
            "get_offer_status",
            &json!({"conversation_id":"thread"}),
        )
        .unwrap();
    assert_eq!(v["event_count"], 2);
    assert_eq!(v["status"], "accepted-buy");
    assert_eq!(v["offer_id"], "offer-id");
    assert_eq!(v["amount"], 25.50);
    assert_eq!(v["authoritative"], false);
}
#[test]
fn offer_inbox_is_bounded_sorted_and_keeps_threads_without_offers() {
    let s = Server::new(|r| {
        assert_eq!(r.path(), "/bff/messaging/inbox");
        assert!(r.target.contains("page_size=30&max_messages=30"));
        let conversations = (0..35)
            .map(|i| {
                let mut v = thread();
                v["hash"] = format!("thread-{i}").into();
                v["messages"]["messages"] = json!([{
                    "timestamp":(i + 1).to_string(), "text":"Has enviado una oferta de 12 €",
                    "type":if i == 29 { "delivery_generic" } else { "text" }
                }]);
                v
            })
            .collect::<Vec<_>>();
        Reply::json(200, json!({"conversations":conversations}))
    });
    let c = s.client(false);
    let v = c
        .invoke(&ctx(), "get_offer_status", &json!({"last":2}))
        .unwrap();
    assert_eq!(v["count"], 2);
    assert_eq!(v["conversations"][0]["conversation_id"], "thread-29");
    assert_eq!(v["conversations"][0]["status"], "our-offer");
    assert_eq!(v["conversations"][1]["conversation_id"], "thread-28");
    assert_eq!(v["conversations"][1]["available"], false);
    assert_eq!(v["history_complete"], false);
    assert_eq!(v["authoritative"], false);
    assert_eq!(
        c.invoke(&ctx(), "get_offer_status", &json!({})).unwrap()["count"],
        30
    );
    assert_eq!(s.count(), 1);
}
#[test]
fn deals_exclude_sold_and_reserved_and_use_sample_median() {
    let s = Server::new(|r| {
        if r.path().ends_with("components") {
            Reply::json(200, json!({"search_id":"sid"}))
        } else {
            Reply::json(
                200,
                json!({"items":[{"id":"cheap","price":10},{"id":"median","price":30},{"id":"high","price":50},
                    {"id":"sold","price":1,"is_sold":true},{"id":"also-sold","price":2,"sold":true},
                    {"id":"reserved","price":3,"reserved":{"flag":true}}]}),
            )
        }
    });
    let v = s
        .client(false)
        .invoke(&ctx(), "find_deals", &json!({"keywords":"camera"}))
        .unwrap();
    assert_eq!(v["count"], 1);
    assert_eq!(v["items"][0]["id"], "cheap");
    assert_eq!(v["sample_size"], 3);
}
#[test]
fn mutations_preserve_exact_cents_use_expected_methods_and_invalidate_cache() {
    let s = Server::new(|r| {
        if r.method == "GET" {
            return Reply::json(200, item());
        }
        match r.path() {
            "/api/v3/delivery/buyer/offers" => {
                let b = r.json();
                assert_eq!(b["offer_price_amount"].to_string(), "19.95");
                assert_eq!(b["offer_price_currency"], "EUR");
                assert_eq!(b["offer_id"].as_str().unwrap().len(), 36);
                assert_eq!(r.method, "POST");
            }
            "/api/v3/delivery/offers/offer/status" => {
                assert_eq!(r.method, "PATCH");
                assert_eq!(r.json()["status"], "ACCEPTED");
            }
            "/api/v3/delivery/offers/offer/offer-price" => {
                assert_eq!(r.json()["offer_price_amount"].to_string(), "12.50")
            }
            _ => (),
        }
        Reply::raw(204, "")
    });
    let c = s.client(true);
    c.invoke(&ctx(), "get_item", &json!({"item_id":ITEM}))
        .unwrap();
    c.invoke(&ctx(), "get_item", &json!({"item_id":ITEM}))
        .unwrap();
    assert_eq!(s.count(), 1);
    for (tool, a) in [
        ("set_favorite", json!({"item_id":ITEM,"favorited":true})),
        ("reserve_item", json!({"item_id":ITEM,"reserved":false})),
        ("mark_sold", json!({"item_id":ITEM,"sold":true})),
        ("delete_item", json!({"item_id":ITEM})),
        ("send_offer", json!({"item_id":ITEM,"amount":"19.95"})),
        (
            "respond_offer",
            json!({"offer_id":"offer","action":"accept"}),
        ),
        (
            "counter_offer",
            json!({"offer_id":"offer","amount":"12.50"}),
        ),
    ] {
        assert!(c.invoke(&ctx(), tool, &a).is_ok(), "{tool}");
    }
    c.invoke(&ctx(), "get_item", &json!({"item_id":ITEM}))
        .unwrap();
    assert_eq!(s.count(), 10);
}
#[test]
fn unknown_write_never_retries_and_retains_offer_id() {
    let n = Arc::new(AtomicUsize::new(0));
    let count = n.clone();
    let s = Server::new(move |r| {
        if r.method == "GET" {
            Reply::json(200, item())
        } else {
            count.fetch_add(1, Ordering::SeqCst);
            Reply::raw(503, "")
        }
    });
    let e = s
        .client(true)
        .invoke(
            &ctx(),
            "send_offer",
            &json!({"item_id":ITEM,"amount":"19.95"}),
        )
        .unwrap_err();
    assert!(e.unknown);
    assert_eq!(n.load(Ordering::SeqCst), 1);
    assert_eq!(e.partial.unwrap()["offer_id"].as_str().unwrap().len(), 36);
}
#[test]
fn successful_write_with_invalid_json_is_unknown() {
    let s = Server::new(|_| Reply::raw(200, "<html>secret-token</html>"));
    let e = s
        .client(true)
        .invoke(
            &ctx(),
            "set_favorite",
            &json!({"item_id":ITEM,"favorited":true}),
        )
        .unwrap_err();
    assert!(e.unknown);
    assert!(!e.to_string().contains("secret"));
    assert_eq!(s.count(), 1);
}
#[test]
fn api_refresh_is_single_flight_and_persists_token_aliases_privately() {
    let dir = Temp::new();
    dir.file("session.json",json!({"accessToken":"old","refreshToken":"r","custom":"keep","cookies":{"accessToken":"old"}}).to_string());
    let n = Arc::new(AtomicUsize::new(0));
    let count = n.clone();
    let s = Server::new(move |r| {
        if r.path() == "/api/v3/access/refresh" {
            count.fetch_add(1, Ordering::SeqCst);
            thread::sleep(Duration::from_millis(30));
            Reply::json(
                200,
                json!({"data":{"access_token":"new","refresh_token":"new-refresh"}}),
            )
        } else if r.headers.get("authorization").map(String::as_str) != Some("Bearer new") {
            Reply::raw(401, "")
        } else {
            Reply::json(200, thread())
        }
    });
    let c = Arc::new(s.configured(false, "", "", Some(dir.0.clone())));
    let handles = (0..4)
        .map(|i| {
            let c = c.clone();
            thread::spawn(move || {
                c.invoke(
                    &ctx(),
                    "get_conversation",
                    &json!({"conversation_id":format!("thread{i}")}),
                )
            })
        })
        .collect::<Vec<_>>();
    for h in handles {
        h.join().unwrap().unwrap();
    }
    assert_eq!(n.load(Ordering::SeqCst), 1);
    let path = dir.0.join("session.json");
    let v: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    for key in ["access_token", "accessToken", "token"] {
        assert_eq!(v[key], "new");
    }
    assert_eq!(v["cookies"]["accessToken"], "new");
    assert_eq!(v["custom"], "keep");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}
#[test]
fn same_token_refresh_falls_back_to_web_cookie() {
    let dir = Temp::new();
    dir.file("session.json",json!({"access_token":"old","refresh_token":"r","__Secure-next-auth.session-token":"cookie"}).to_string());
    let s = Server::new(|r| match r.path() {
        "/api/v3/access/refresh" => Reply::json(200, json!({"access_token":"old"})),
        "/api/auth/session" => {
            assert!(r.headers["cookie"].contains("__Secure-next-auth.session-token=cookie"));
            assert!(!r.headers.contains_key("authorization"));
            Reply::json(200, json!({"accessToken":"new"}))
        }
        _ => {
            if r.headers.get("authorization").map(String::as_str) == Some("Bearer new") {
                Reply::json(200, thread())
            } else {
                Reply::raw(401, "")
            }
        }
    });
    assert!(
        s.configured(false, "", "", Some(dir.0.clone()))
            .invoke(
                &ctx(),
                "get_conversation",
                &json!({"conversation_id":"thread"})
            )
            .is_ok()
    );
    assert_eq!(s.count(), 4);
}
#[test]
fn failed_auth_does_not_replay_a_mutation_with_same_token() {
    let s = Server::new(|_| Reply::raw(401, "secret"));
    let c = s.configured(true, "old", "refresh", None);
    let e = c
        .invoke(
            &ctx(),
            "set_favorite",
            &json!({"item_id":ITEM,"favorited":true}),
        )
        .unwrap_err();
    assert!(!e.unknown);
    assert_eq!(s.count(), 2);
}
#[test]
fn session_validation_aliases_login_logout_and_no_implicit_loading() {
    for payload in [
        "null",
        "[]",
        "{broken",
        r#"{"cookies":"bad"}"#,
        r#"{"cookies":{"accessToken":42}}"#,
        r#"{"access_token":42}"#,
        r#"{"data":{"refreshToken":false}}"#,
    ] {
        let dir = Temp::new();
        let path = dir.file("session.json", payload);
        assert!(
            Client::new(Config {
                session: Some(path),
                ..Config::default()
            })
            .is_err()
        );
    }
    let s = Server::new(|r| {
        assert_eq!(r.json()["client_id"], "wallapop");
        Reply::json(200, json!({"data":{"token":"signed","refresh_token":"r"}}))
    });
    let dir = Temp::new();
    dir.file("credentials.json", r#"{"access_token":"legacy"}"#);
    let c = s.configured(false, "", "", Some(dir.0.clone()));
    assert_eq!(c.status()["logged_in"], true);
    c.login(&ctx(), "email", "password").unwrap();
    assert_eq!(c.status()["logged_in"], true);
    c.logout().unwrap();
    assert_eq!(
        s.configured(false, "", "", Some(dir.0.clone())).status()["logged_in"],
        false
    );
    assert_eq!(
        Client::new(Config::default()).unwrap().status()["source"],
        "none"
    );
}
#[test]
fn message_seller_reply_uses_existing_thread_and_skips_duplicate() {
    let s = Server::new(|r| {
        assert_ne!(r.path(), "/api/v3/instant-messaging/conversation");
        let mut v = thread();
        v["messages"]["messages"] = json!([
        {"from_self":true,"text":"older","timestamp":"9"},{"from_self":true,"text":"Hola","timestamp":"10"}]);
        Reply::json(200, v)
    });
    let v = s
        .client(true)
        .invoke(
            &ctx(),
            "send_message",
            &json!({"conversation_id":"thread","message":"Hola"}),
        )
        .unwrap();
    assert_eq!(v["status"], "already_sent");
    assert_eq!(s.count(), 1);
}
#[test]
fn new_message_uses_rest_publish_ack_and_sender_claims() {
    for ack in [
        json!([1, "Sent", "123"]),
        json!("[1,\"Sent\",\"123\"]"),
        json!([0, "Rejected"]),
    ] {
        let expected = ack == json!([0, "Rejected"]);
        let s = Server::new(move |r| match r.path() {
            "/api/v3/instant-messaging/conversation" => {
                Reply::json(200, json!({"conversation_id":"thread"}))
            }
            "/bff/messaging/conversation/thread" => Reply::json(200, thread()),
            "/api/v3/instant-messaging/token" => {
                Reply::json(200, json!({"data":{"token":"pub-auth"}}))
            }
            path if path.starts_with("/publish/") => {
                assert_eq!(r.method, "GET");
                assert!(!r.headers.contains_key("authorization"));
                assert!(r.target.contains("from_user_hash%22%3A%22sender"));
                Reply::json(200, ack.clone())
            }
            _ => panic!("{}", r.target),
        });
        let c = s.configured(true, &signed_token(), "", None);
        let result = c.invoke(
            &ctx(),
            "send_message",
            &json!({"item_id":ITEM,"message":"Hola"}),
        );
        if expected {
            let e = result.unwrap_err();
            assert!(e.unknown);
            assert_eq!(e.partial.unwrap()["conversation_id"], "thread");
        } else {
            assert_eq!(result.unwrap()["status"], "sent");
        }
        assert_eq!(s.count(), 4);
    }
}
#[test]
fn edit_preserves_location_attributes_shipping_and_picture_ids() {
    let s = Server::new(|r| {
        if r.method == "GET" {
            Reply::json(200, item())
        } else {
            let v = r.json();
            assert_eq!(v["attributes"]["price_amount"].to_string(), "12.50");
            assert_eq!(v["attributes"]["title"], "Camera");
            assert_eq!(v["attributes"]["brand"], "Brand");
            assert_eq!(v["location"]["latitude"], 41.2);
            assert_eq!(v["delivery"]["allowed_by_user"], false);
            assert_eq!(v["pictures"][1], json!({"id":"photo2","order":1}));
            Reply::json(200, json!({"ok":true}))
        }
    });
    assert!(
        s.client(true)
            .invoke(
                &ctx(),
                "edit_item",
                &json!({"item_id":ITEM,"price":"12.50"})
            )
            .is_ok()
    );
}
#[test]
fn edit_rejects_missing_state_before_writing() {
    for key in [
        "images",
        "shipping",
        "location",
        "title",
        "brand",
        "condition",
        "price",
        "category_leaf_id",
    ] {
        let mut v = item();
        v.as_object_mut().unwrap().remove(key);
        let s = Server::new(move |r| {
            assert_eq!(r.method, "GET");
            Reply::json(200, v.clone())
        });
        assert!(
            s.client(true)
                .invoke(
                    &ctx(),
                    "edit_item",
                    &json!({"item_id":ITEM,"description":"New description"})
                )
                .is_err(),
            "{key}"
        );
        assert_eq!(s.count(), 1);
    }
}
fn publish_args(dir: &Temp, count: usize) -> Value {
    let paths = (0..count)
        .map(|i| dir.file(&format!("photo{i}.jpg"), [255, 216, 255, 1]))
        .collect::<Vec<_>>();
    json!({"title":"Camera","description":"Description","price":"19.95","category_leaf_id":"10088","condition":"good","color":"blue","shipping_allowed":false,"photo_paths":paths})
}
#[test]
fn publish_validates_all_photos_before_network() {
    let s = Server::new(|_| panic!("bad photos contacted upstream"));
    let dir = Temp::new();
    let mut a = publish_args(&dir, 2);
    std::fs::write(a["photo_paths"][1].as_str().unwrap(), "invalid JPEG").unwrap();
    assert!(s.client(true).invoke(&ctx(), "publish_item", &a).is_err());
    a["photo_paths"] = json!([dir.0]);
    assert!(s.client(true).invoke(&ctx(), "publish_item", &a).is_err());
    assert_eq!(s.count(), 0);
}
#[test]
fn publish_multipart_extra_photos_partial_id_and_exact_cents() {
    let dir = Temp::new();
    let s = Server::new(|r| match r.path() {
        "/api/v3/users/me" => Reply::json(
            200,
            json!({"location":{"approximated_latitude":40.4,"approximated_longitude":-3.7}}),
        ),
        "/api/v3/items" => {
            let b = String::from_utf8_lossy(&r.body);
            assert!(b.contains("\"price_amount\":19.95"));
            assert!(b.contains("name=\"image\""));
            assert!(b.contains("name=\"item\""));
            Reply::json(201, json!({"id":ITEM}))
        }
        path if path.ends_with("/picture2") => {
            assert!(String::from_utf8_lossy(&r.body).contains("\r\n2\r\n"));
            Reply::raw(503, "")
        }
        _ => panic!("{}", r.target),
    });
    let out = s
        .client(true)
        .invoke(&ctx(), "publish_item", &publish_args(&dir, 2))
        .unwrap();
    assert_eq!(out["item_id"], ITEM);
    assert_eq!(out["completed_photos"], 1);
    assert_eq!(out["partial"], true);
    assert_eq!(out["failed_photos"], json!([2]));
    assert_eq!(out["unknown_outcome"], true);
    assert_eq!(s.count(), 3);
}
#[test]
fn publish_does_not_guess_shipping_configuration_and_requires_location() {
    let dir = Temp::new();
    let mut a = publish_args(&dir, 1);
    a["shipping_allowed"] = true.into();
    a["max_weight_kg"] = 1.into();
    let s = Server::new(|r| {
        if r.path().ends_with("/me") {
            Reply::json(
                200,
                json!({"id":"seller","location":{"approximated_latitude":40,"approximated_longitude":-3}}),
            )
        } else {
            Reply::json(200, json!({"data":[]}))
        }
    });
    assert_eq!(
        s.client(true)
            .invoke(&ctx(), "publish_item", &a)
            .unwrap_err()
            .code,
        "cost_configuration_id_required"
    );
    assert_eq!(s.count(), 2);
}
#[test]
fn publish_timeout_after_creation_keeps_item_id() {
    let dir = Temp::new();
    let s = Server::new(|r| {
        if r.path().ends_with("/me") {
            Reply::json(
                200,
                json!({"location":{"approximated_latitude":40,"approximated_longitude":-3}}),
            )
        } else if r.path().ends_with("/items") {
            Reply::json(201, json!({"id":ITEM}))
        } else {
            thread::sleep(Duration::from_millis(100));
            Reply::json(200, json!({"ok":true}))
        }
    });
    let result = s.client(true).invoke(
        &Context::new(Duration::from_millis(50)),
        "publish_item",
        &publish_args(&dir, 3),
    );
    let v = match result {
        Ok(v) => v,
        Err(e) => e.partial.unwrap(),
    };
    assert_eq!(v["item_id"], ITEM);
    assert_eq!(v["partial"], true);
}

#[test]
fn publish_accepts_plain_photo_ids_and_shipping_configuration() {
    let dir = Temp::new();
    let s = Server::new(|r| match r.path() {
        "/api/v3/users/me" => Reply::json(
            200,
            json!({"id":"seller","location":{"approximated_latitude":40,"approximated_longitude":-3}}),
        ),
        "/api/v3/users/seller/items" => Reply::json(
            200,
            json!({"data":[{"shipping":{"cost_configuration_id":"known-cost"}}]}),
        ),
        "/api/v3/items" => {
            let b = String::from_utf8_lossy(&r.body);
            assert!(b.contains("\"cost_configuration_id\":\"known-cost\""));
            assert!(b.contains("\"max_weight_kg\":2"));
            Reply::json(201, json!({"id":ITEM}))
        }
        _ => Reply::raw(200, "new-photo-id"),
    });
    let mut a = publish_args(&dir, 2);
    a["shipping_allowed"] = true.into();
    a["max_weight_kg"] = 2.into();
    let v = s.client(true).invoke(&ctx(), "publish_item", &a).unwrap();
    assert_eq!(v["partial"], false);
    assert_eq!(v["completed_photos"], 2);
    assert_eq!(s.count(), 4);
}

#[test]
fn publish_missing_created_id_is_unknown_and_does_not_upload_extras() {
    let dir = Temp::new();
    let s = Server::new(|r| {
        if r.path().ends_with("/me") {
            Reply::json(
                200,
                json!({"location":{"approximated_latitude":40,"approximated_longitude":-3}}),
            )
        } else {
            Reply::json(201, json!({"ok":true}))
        }
    });
    let e = s
        .client(true)
        .invoke(&ctx(), "publish_item", &publish_args(&dir, 2))
        .unwrap_err();
    assert!(e.unknown);
    assert_eq!(e.code, "missing_created_item_id");
    assert_eq!(s.count(), 2);
}
