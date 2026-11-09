mod common;
use common::*;
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant, SystemTime},
};
use wallapop_mcp::{
    Context,
    transport::{MAX_BODY, Request, Transport, retry_after},
};

fn request<'a>(url: &'a str, headers: &'a [(String, String)], read: bool) -> Request<'a> {
    Request {
        method: "GET",
        url,
        headers,
        body: &[],
        read,
        ttl: Duration::from_secs(30),
    }
}
#[test]
fn coalesces_identical_reads_and_isolates_credentials() {
    let s = Server::new(|_| {
        thread::sleep(Duration::from_millis(40));
        Reply::raw(200, "ok")
    });
    let transport = Arc::new(Transport::new(Duration::ZERO));
    let handles = (0..6)
        .map(|_| {
            let t = transport.clone();
            let url = s.url.clone();
            thread::spawn(move || t.request(&ctx(), request(&url, &[], true)).unwrap())
        })
        .collect::<Vec<_>>();
    for h in handles {
        assert_eq!(h.join().unwrap().body, b"ok");
    }
    assert_eq!(s.count(), 1);
    for token in ["account-a", "account-b", "account-a"] {
        let headers = vec![("Authorization".into(), token.into())];
        transport
            .request(&ctx(), request(&s.url, &headers, true))
            .unwrap();
    }
    assert_eq!(s.count(), 3);
}
#[test]
fn fresh_reads_run_independently_and_do_not_replace_cached_reads() {
    let active = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let (inflight, maximum) = (active.clone(), peak.clone());
    let s = Server::new(move |_| {
        maximum.fetch_max(
            inflight.fetch_add(1, Ordering::SeqCst) + 1,
            Ordering::SeqCst,
        );
        thread::sleep(Duration::from_millis(60));
        inflight.fetch_sub(1, Ordering::SeqCst);
        Reply::raw(200, "ok")
    });
    let t = Arc::new(Transport::new(Duration::ZERO));
    t.request(&ctx(), request(&s.url, &[], true)).unwrap();
    let start = Arc::new(std::sync::Barrier::new(3));
    let handles = (0..2)
        .map(|_| {
            let (t, url, start) = (t.clone(), s.url.clone(), start.clone());
            thread::spawn(move || {
                start.wait();
                let mut r = request(&url, &[], true);
                r.ttl = Duration::ZERO;
                t.request(&ctx(), r).unwrap();
            })
        })
        .collect::<Vec<_>>();
    start.wait();
    for h in handles {
        h.join().unwrap();
    }
    assert_eq!(peak.load(Ordering::SeqCst), 2);
    t.request(&ctx(), request(&s.url, &[], true)).unwrap();
    assert_eq!(s.count(), 3);
}
#[test]
fn retries_transient_reads_but_never_write_shaped_get_or_403() {
    for (status, read, expected) in [
        (503, true, 3),
        (429, true, 3),
        (503, false, 1),
        (403, true, 1),
        (404, true, 1),
    ] {
        let s = Server::new(move |_| Reply::raw(status, "rejected"));
        let t = Transport::new(Duration::ZERO);
        let e = t.request(&ctx(), request(&s.url, &[], read)).unwrap_err();
        assert_eq!(s.count(), expected);
        assert_eq!(e.status, Some(status));
        assert_eq!(e.unknown, !read && status >= 500);
    }
}
#[test]
fn retry_after_obeys_total_budget_and_handles_large_values() {
    assert_eq!(retry_after(Some("-1")), Duration::ZERO);
    assert_eq!(retry_after(Some("oops")), Duration::ZERO);
    assert_eq!(retry_after(Some("0")), Duration::ZERO);
    assert_eq!(
        retry_after(Some("9999999999999999999999999999999")),
        Duration::from_secs(86400)
    );
    let date = httpdate::fmt_http_date(SystemTime::now() + Duration::from_secs(60));
    assert!(retry_after(Some(&date)) > Duration::from_secs(58));
    let s = Server::new(|_| Reply::raw(429, "").header("Retry-After", "3600"));
    let t = Transport::new(Duration::ZERO);
    let start = Instant::now();
    let e = t
        .request(
            &Context::new(Duration::from_millis(60)),
            request(&s.url, &[], true),
        )
        .unwrap_err();
    assert_eq!(e.code, "timeout");
    assert_eq!(s.count(), 1);
    assert!(start.elapsed() < Duration::from_secs(1));
}
#[test]
fn cancellation_while_pacing_does_not_reserve_future_slots() {
    let s = Server::new(|_| Reply::raw(200, "ok"));
    let t = Arc::new(Transport::new(Duration::from_millis(150)));
    t.request(&ctx(), request(&s.url, &[], false)).unwrap();
    let cancelled = Context::new(Duration::from_secs(2));
    cancelled.cancelled.store(true, Ordering::Relaxed);
    assert_eq!(
        t.request(&cancelled, request(&s.url, &[], false))
            .unwrap_err()
            .code,
        "cancelled"
    );
    let start = Instant::now();
    t.request(&ctx(), request(&s.url, &[], false)).unwrap();
    assert!(start.elapsed() < Duration::from_millis(400));
    assert_eq!(s.count(), 2);
}
#[test]
fn waiter_can_cancel_without_stopping_owner() {
    let s = Server::new(|_| {
        thread::sleep(Duration::from_millis(100));
        Reply::raw(200, "ok")
    });
    let t = Arc::new(Transport::new(Duration::ZERO));
    let owner = {
        let t = t.clone();
        let url = s.url.clone();
        thread::spawn(move || t.request(&ctx(), request(&url, &[], true)).unwrap())
    };
    while s.count() == 0 {
        thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(
        t.request(
            &Context::new(Duration::from_millis(20)),
            request(&s.url, &[], true)
        )
        .unwrap_err()
        .code,
        "timeout"
    );
    owner.join().unwrap();
    assert_eq!(s.count(), 1);
}
#[test]
fn invalidation_wins_against_inflight_cache_and_errors_are_not_cached() {
    let n = Arc::new(AtomicUsize::new(0));
    let count = n.clone();
    let s = Server::new(move |_| {
        count.fetch_add(1, Ordering::SeqCst);
        thread::sleep(Duration::from_millis(60));
        Reply::raw(200, "ok")
    });
    let t = Arc::new(Transport::new(Duration::ZERO));
    let owner = {
        let t = t.clone();
        let url = s.url.clone();
        thread::spawn(move || t.request(&ctx(), request(&url, &[], true)).unwrap())
    };
    while n.load(Ordering::SeqCst) == 0 {
        thread::sleep(Duration::from_millis(1));
    }
    t.invalidate();
    owner.join().unwrap();
    t.request(&ctx(), request(&s.url, &[], true)).unwrap();
    assert_eq!(s.count(), 2);
    let s = Server::new(|_| Reply::raw(403, "error"));
    for _ in 0..2 {
        assert!(t.request(&ctx(), request(&s.url, &[], true)).is_err());
    }
    assert_eq!(s.count(), 2);
}
#[test]
fn response_body_is_bounded_and_redirects_do_not_forward_credentials() {
    let s = Server::new(|_| Reply::raw(200, vec![b'x'; MAX_BODY + 1]));
    let t = Transport::new(Duration::ZERO);
    assert_eq!(
        t.request(&ctx(), request(&s.url, &[], true))
            .unwrap_err()
            .code,
        "response_too_large"
    );
    let sink = Server::new(|_| panic!("redirect followed"));
    let sink_url = sink.url.clone();
    let redirect = Server::new(move |_| Reply::raw(302, "").header("Location", &sink_url));
    let h = vec![("Authorization".into(), "secret".into())];
    let e = t
        .request(&ctx(), request(&redirect.url, &h, true))
        .unwrap_err();
    assert_eq!(e.status, Some(302));
    assert_eq!(sink.count(), 0);
}
#[test]
fn bounded_cache_evicts_old_entries() {
    let s = Server::new(|_| Reply::raw(200, "ok"));
    let t = Transport::new(Duration::ZERO);
    for i in 0..65 {
        t.request(&ctx(), request(&format!("{}/item{i}", s.url), &[], true))
            .unwrap();
    }
    t.request(&ctx(), request(&format!("{}/item0", s.url), &[], true))
        .unwrap();
    assert_eq!(s.count(), 66);
}
#[test]
fn network_failure_on_write_is_ambiguous_and_sanitized() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/secret-token", listener.local_addr().unwrap());
    let worker = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        use std::io::Read;
        let mut b = [0u8; 2048];
        let _ = stream.read(&mut b);
    });
    let e = Transport::new(Duration::ZERO)
        .request(&ctx(), request(&url, &[], false))
        .unwrap_err();
    assert!(e.unknown);
    assert!(!e.to_string().contains("secret"));
    worker.join().unwrap();
}
