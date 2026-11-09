mod common;
use common::*;
use serde_json::{Value, json};
use wallapop_mcp::client::{Client, Config, token};

#[test]
fn credentials_skip_empty_aliases_and_report_refresh_only_sessions() {
    assert_eq!(
        token(&json!({"access_token":"", "accessToken":"valid"})),
        "valid"
    );
    assert_eq!(
        token(&json!({"data":{"access_token":"", "token":"nested"}})),
        "nested"
    );
    assert_eq!(token(&json!({"access_token":42})), "");
    let c = Client::new(Config::default()).unwrap();
    for data in [
        json!({"refreshToken":"refresh"}),
        json!({"cookies":{"__Secure-next-auth.session-token":"cookie"}}),
    ] {
        c.import_session(data).unwrap();
        let status = c.status();
        assert_eq!(status["credential_present"], true);
        assert_eq!(status["logged_in"], false);
        assert_eq!(status["session_verified"], false);
    }
}

#[test]
fn importing_an_account_and_password_login_discard_previous_refresh_credentials() {
    for password_login in [false, true] {
        let s = Server::new(|r| match r.path() {
            "/api/v3/access/login" => Reply::json(200, json!({"access_token":"new-account"})),
            "/api/v3/access/refresh" => {
                Reply::json(200, json!({"access_token":"previous-account"}))
            }
            _ if r.headers.get("authorization").map(String::as_str)
                == Some("Bearer previous-account") =>
            {
                Reply::json(200, thread())
            }
            _ => Reply::raw(401, ""),
        });
        let dir = Temp::new();
        dir.file("credentials.json", json!({"data":{"refresh_token":"old-refresh"},"__Secure-next-auth.session-token":"old-cookie"}).to_string());
        let c = s.configured(false, "old-account", "old-refresh", Some(dir.0.clone()));
        if password_login {
            c.login(&ctx(), "new@example.test", "password").unwrap();
        } else {
            c.import_session(json!({"access_token":"new-account"}))
                .unwrap();
        }
        let saved: Value =
            serde_json::from_slice(&std::fs::read(dir.0.join("session.json")).unwrap()).unwrap();
        assert_eq!(saved["refresh_token"], "");
        assert!(!saved.to_string().contains("old-account"));
        assert!(
            c.invoke(
                &ctx(),
                "get_conversation",
                &json!({"conversation_id":"thread"})
            )
            .is_err()
        );
        assert_eq!(s.count(), if password_login { 2 } else { 1 });
        let reloaded = s.configured(false, "", "", Some(dir.0.clone()));
        assert!(
            reloaded
                .invoke(
                    &ctx(),
                    "get_conversation",
                    &json!({"conversation_id":"thread"})
                )
                .is_err()
        );
        assert_eq!(s.count(), if password_login { 3 } else { 2 });
    }
}

#[test]
fn cookie_import_refreshes_the_new_account_and_keeps_its_credentials_on_reload() {
    let s = Server::new(|r| match r.path() {
        "/api/auth/session" => {
            assert!(r.headers["cookie"].contains("__Secure-next-auth.session-token=new-cookie"));
            Reply::json(200, json!({"accessToken":"new-account"}))
        }
        _ => {
            assert_eq!(r.headers["authorization"], "Bearer new-account");
            Reply::json(200, thread())
        }
    });
    let dir = Temp::new();
    let c = s.configured(false, "old-account", "old-refresh", Some(dir.0.clone()));
    c.import_session(json!({"cookies":{"__Secure-next-auth.session-token":"new-cookie"}}))
        .unwrap();
    assert_eq!(c.status()["credential_present"], true);
    c.invoke(
        &ctx(),
        "get_conversation",
        &json!({"conversation_id":"thread"}),
    )
    .unwrap();
    let reloaded = s.configured(false, "", "", Some(dir.0.clone()));
    assert_eq!(reloaded.status()["logged_in"], true);
    reloaded
        .invoke(
            &ctx(),
            "get_conversation",
            &json!({"conversation_id":"thread"}),
        )
        .unwrap();
    assert_eq!(s.count(), 3);
}
