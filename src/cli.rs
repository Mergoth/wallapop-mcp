mod args;
mod auth;
mod automation;
mod contacts;
mod output;

use crate::{
    Context, Error, Result,
    client::{Client, Config, read_file},
    protocol, tools,
};
use args::{Options, arguments};
pub use auth::cookies;
use auth::stdin_text;
use automation::{act, percentage, watch};
use output::output;
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

const HELP: &str = "wallapop-mcp — one native executable for MCP and CLI

  wallapop-mcp [--session PATH] [--allow-writes]          MCP over stdio
  wallapop-mcp tools                                    Tool names and schemas
  wallapop-mcp call TOOL '{JSON}'                        Call any tool
  wallapop-mcp call TOOL -                              Read JSON from stdin
  wallapop-mcp search 'switch' --city madrid --max-price 200
  wallapop-mcp deals 'switch' --min-discount 0.2 --top 5
  wallapop-mcp watch 'switch' --interval 300 [--once]
  wallapop-mcp show ITEM                                Listing details
  wallapop-mcp seller USER                              Profile and reviews
  wallapop-mcp replies [--conversation ID]               Inbox or thread
  wallapop-mcp purchases|selling [--finished]            Transactions
  wallapop-mcp session|categories|items|unread            Account/public reads
  wallapop-mcp --session PATH login --cookies-file FILE  Import session
  wallapop-mcp --session PATH login --token-stdin        Import token privately
  wallapop-mcp --session PATH login EMAIL --password-stdin
  wallapop-mcp --session PATH logout                    Clear saved credentials
  wallapop-mcp --allow-writes message ITEM --text 'Hola'
  wallapop-mcp --allow-writes message --conversation ID --text 'Hola'
  wallapop-mcp --allow-writes offer ITEM --price 19.95
  wallapop-mcp --allow-writes offer accept|decline ID
  wallapop-mcp --allow-writes offer counter ID --price 19.95
  wallapop-mcp offer status [ID] [--last 10]             Inbox or thread offers
  wallapop-mcp --allow-writes edit ITEM --price 25
  wallapop-mcp --allow-writes favorite|reserve ITEM [--remove]
  wallapop-mcp --allow-writes sold ITEM [--undo]
  wallapop-mcp --allow-writes delete ITEM --yes
  wallapop-mcp --allow-writes publish PHOTOS_DIR --summary TITLE
    --description TEXT --price 25 --category-leaf-id 10088 --condition good
    --color blue --shipping-allowed false

Search: --category ID --subcat ID --condition VALUE --min-price DECIMAL
  --max-price DECIMAL --lat N --lon N --city NAME --order relevance|newest|
  price_asc|price_desc|closest --pages 1..5 --limit 1..50 --ship-only --fresh
Output: JSON by default; --csv or --quiet for search/deals. --json is accepted.
Deals/watch: --offer-pct 0.8 --msg TEMPLATE --dry-run --contacted-file PATH.
  Writes require --allow-writes. Contact history defaults beside an explicit session.
Global: --session PATH --allow-writes --timeout 45s --min-interval 1s
  --concurrency 4 (1..16) --version --help. No account needed for public reads.
Environment: WALLAPOP_SESSION_PATH, WALLAPOP_ACCESS_TOKEN, WALLAPOP_REFRESH_TOKEN.
";

pub fn duration(s: &str) -> Result<Duration> {
    let (s, multiplier) = if let Some(s) = s.strip_suffix("ms") {
        (s, 0.001)
    } else if let Some(s) = s.strip_suffix('s') {
        (s, 1.0)
    } else {
        (s, 1.0)
    };
    let seconds = s
        .parse::<f64>()
        .map_err(|_| Error::new("invalid_duration"))?
        * multiplier;
    if !seconds.is_finite() || !(0.0..=86400.0).contains(&seconds) {
        return Err(Error::new("invalid_duration"));
    }
    Ok(Duration::from_secs_f64(seconds))
}

pub fn run(args: Vec<String>) -> Result<()> {
    let mut o = Options::parse(&args)?;
    if o.flag("help") {
        print!("{HELP}");
        return Ok(());
    }
    if o.flag("version") {
        println!("wallapop-mcp {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let mut config = Config {
        access_token: env("WALLAPOP_ACCESS_TOKEN"),
        refresh_token: env("WALLAPOP_REFRESH_TOKEN"),
        session: std::env::var_os("WALLAPOP_SESSION_PATH").map(PathBuf::from),
        allow_writes: o.values.remove("allow-writes").is_some(),
        ..Config::default()
    };
    if let Some(path) = o.values.remove("session") {
        config.session = Some(expand_home(&path));
    }
    config.interval = o
        .values
        .remove("min-interval")
        .as_deref()
        .map(duration)
        .transpose()?;
    let timeout = duration(&o.values.remove("timeout").unwrap_or("45s".into()))?;
    let concurrency = o
        .values
        .remove("concurrency")
        .unwrap_or("4".into())
        .parse::<usize>()
        .map_err(|_| Error::new("invalid_concurrency"))?;
    if timeout.is_zero() || !(1..=16).contains(&concurrency) {
        return Err(Error::new("invalid_limits"));
    }
    let command = if o.positions.is_empty() {
        "serve".into()
    } else {
        o.positions.remove(0)
    };
    let command = command.as_str();
    if matches!(command, "deals" | "watch") && o.get("contacted-file").is_none() {
        if let Some(path) = &config.session {
            let path = if path.is_dir() {
                path.join("contacted.json")
            } else {
                path.with_file_name("contacted.json")
            };
            o.values
                .insert("contacted-file".into(), path.to_string_lossy().into());
        }
    }
    if command == "tools" {
        o.check(0, &[])?;
        println!(
            "{}",
            serde_json::to_string_pretty(
                &tools::definitions()
                    .iter()
                    .filter(|t| !t.write || config.allow_writes)
                    .map(|t| t.value())
                    .collect::<Vec<_>>()
            )
            .unwrap()
        );
        return Ok(());
    }
    if command == "login" {
        o.check(
            usize::from(o.flag("password-stdin")),
            &["token-stdin", "password-stdin", "cookies-file", "cookies"],
        )?;
        if o.values.len() != 1 {
            return Err(Error::new("provide_one_login_method"));
        }
    }
    if command == "logout" {
        o.check(0, &[])?;
    }
    if matches!(command, "login" | "logout") && config.session.is_none() {
        config.session = Some(PathBuf::from(env("HOME")).join(".config/wallapop_cli/session.json"));
    }
    let client = Arc::new(Client::new(config)?);
    if command == "serve" {
        o.check(0, &[])?;
        return protocol::serve(
            std::io::stdin().lock(),
            std::io::stdout(),
            client,
            timeout,
            concurrency,
        );
    }
    if command == "call" {
        if !(1..=2).contains(&o.positions.len()) {
            return Err(Error::new("expected_tool_and_json"));
        }
        o.check(o.positions.len(), &[])?;
        let payload = match o.positions.get(1).map(String::as_str) {
            Some("-") => stdin_text()?,
            Some(s) => s.into(),
            None => "{}".into(),
        };
        let a: Value = serde_json::from_str(&payload).map_err(|_| Error::new("invalid_json"))?;
        println!(
            "{}",
            client.invoke(&Context::new(timeout), o.position(0)?, &a)?
        );
        return Ok(());
    }
    if command == "login" {
        if o.flag("token-stdin") {
            client.import_session(json!({"access_token":stdin_text()?.trim()}))?;
        } else if let Some(file) = o.get("cookies-file") {
            client.import_session(cookies(
                &String::from_utf8(read_file(Path::new(file), 65536)?)
                    .map_err(|_| Error::new("invalid_session_text"))?,
            )?)?;
        } else if let Some(s) = o.get("cookies") {
            client.import_session(cookies(s)?)?;
        } else if o.flag("password-stdin") {
            client.login(
                &Context::new(timeout),
                o.position(0)?,
                stdin_text()?.trim_end(),
            )?;
        } else {
            return Err(Error::new("use_cookies_file_token_stdin_or_password_stdin"));
        }
        println!("{}", client.status());
        return Ok(());
    }
    if command == "logout" {
        client.logout()?;
        println!("{}", client.status());
        return Ok(());
    }
    let (name, mut a) = arguments(command, &o)?;
    automation::validate(&client, &o)?;
    if command == "watch" {
        return watch(&client, &o, a, timeout);
    }
    if let Some(pct) = o.get("pct") {
        let item = client.invoke(
            &Context::new(timeout),
            "get_item",
            &json!({"item_id":a["item_id"],"fresh":true}),
        )?;
        a["amount"] = percentage(&item["price"], pct)?.into();
    }
    if o.flag("dry-run") {
        if let Some(tool) = tools::definitions()
            .iter()
            .find(|t| t.name == name && t.write)
        {
            tool.validate(&a)?;
            println!("{}", json!({"dry_run":true,"tool":name,"arguments":a}));
            return Ok(());
        }
    }
    let result = client.invoke(&Context::new(timeout), name, &a)?;
    if command == "deals" {
        act(&client, &o, &result, timeout)?;
    }
    output(&result, &o);
    Ok(())
}

fn env(key: &str) -> String {
    std::env::var(key).unwrap_or_default()
}

fn expand_home(path: &str) -> PathBuf {
    if let Some(p) = path.strip_prefix("~/") {
        PathBuf::from(env("HOME")).join(p)
    } else {
        path.into()
    }
}
