//! Prova con un finto servizio locale che risponde come le IA vere (streaming SSE ed errori).
#![allow(clippy::unwrap_used, clippy::while_let_loop)]
use multiia::api::{list_models, stream_chat, Event, Message};
use multiia::config::Provider;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::AtomicBool;

/// Legge tutta la richiesta (intestazioni e corpo): se il finto servizio chiudesse prima,
/// il client riceverebbe un reset invece della risposta.
fn read_request(s: &mut std::net::TcpStream) -> String {
    let mut data = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        let Ok(n) = s.read(&mut buf) else { break };
        if n == 0 {
            break;
        }
        data.extend_from_slice(&buf[..n]);
        let text = String::from_utf8_lossy(&data).to_string();
        if let Some(i) = text.find("\r\n\r\n") {
            let len = text[..i]
                .lines()
                .find_map(|l| {
                    l.to_lowercase()
                        .strip_prefix("content-length:")
                        .and_then(|v| v.trim().parse::<usize>().ok())
                })
                .unwrap_or(0);
            if data.len() >= i + 4 + len {
                break;
            }
        }
    }
    String::from_utf8_lossy(&data).to_string()
}

fn serve(response: &'static str) -> String {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap();
    std::thread::spawn(move || {
        if let Ok((mut s, _)) = l.accept() {
            read_request(&mut s);
            let _ = s.write_all(response.as_bytes());
        }
    });
    format!("http://{addr}/v1")
}

fn run(url: String) -> Vec<Event> {
    let p = Provider::new("t", &url, "m");
    let mut out = Vec::new();
    stream_chat(
        &p,
        "",
        &[Message::new("user", "ciao")],
        10,
        1000,
        &AtomicBool::new(false),
        &mut |e| out.push(e),
    );
    out
}

const OK: &str = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n\
: keep-alive\n\n\
data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"penso\"}}]}\n\n\
data: {\"choices\":[{\"delta\":{\"content\":\"Cia\"}}]}\n\n\
data: {\"choices\":[{\"delta\":{\"content\":\"o!\"}}]}\n\n\
data: {\"choices\":[],\"usage\":{\"prompt_tokens\":2,\"completion_tokens\":3}}\n\n\
data: [DONE]\n\n";

#[test]
fn streams_text_reasoning_and_usage() {
    let ev = run(serve(OK));
    assert_eq!(
        ev,
        vec![
            Event::Reasoning("penso".into()),
            Event::Text("Cia".into()),
            Event::Text("o!".into()),
            Event::Usage {
                prompt: 2,
                completion: 3
            },
            Event::Done
        ]
    );
}

#[test]
fn http_errors_become_one_error_event() {
    let ev = run(serve("HTTP/1.1 401 Unauthorized\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{\"error\":{\"message\":\"bad key\"}}"));
    assert_eq!(ev.len(), 1, "{ev:?}");
    assert!(matches!(&ev[0], Event::Error(e) if e.contains("Chiave API") && e.contains("bad key")));
}

#[test]
fn unreachable_server_is_an_error_not_a_hang() {
    let ev = run("http://127.0.0.1:1/v1".into());
    assert!(matches!(ev.last(), Some(Event::Error(e)) if e.contains("Connessione")));
}

#[test]
fn a_stream_cut_without_done_still_ends() {
    let ev = run(serve("HTTP/1.1 200 OK\r\nConnection: close\r\n\r\ndata: {\"choices\":[{\"delta\":{\"content\":\"x\"}}]}\n\n"));
    assert_eq!(ev.last(), Some(&Event::Done));
}

#[test]
fn lists_models() {
    let url = serve("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{\"data\":[{\"id\":\"b\"},{\"id\":\"a\"}]}");
    let p = Provider::new("t", &url, "m");
    assert_eq!(list_models(&p, 10).unwrap(), vec!["a", "b"]);
}

/// Servizio che risponde 503 la prima volta e bene la seconda.
fn serve_flaky() -> String {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap();
    std::thread::spawn(move || {
        for (i, conn) in l.incoming().take(2).enumerate() {
            let Ok(mut s) = conn else { continue };
            read_request(&mut s);
            let r = if i == 0 {
                "HTTP/1.1 503 Service Unavailable\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n[{\"error\":{\"message\":\"overloaded\"}}]"
            } else {
                OK
            };
            let _ = s.write_all(r.as_bytes());
        }
    });
    format!("http://{addr}/v1")
}

#[test]
fn an_overloaded_service_is_retried_automatically() {
    let ev = run(serve_flaky());
    assert_eq!(ev.first(), Some(&Event::Retry { attempt: 1, max: 2 }));
    assert!(ev.contains(&Event::Text("Cia".into())));
    assert_eq!(ev.last(), Some(&Event::Done));
}

/// Servizio che accetta più richieste: i modelli con «bad» nel nome danno 503, gli altri vanno.
fn serve_models() -> String {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap();
    std::thread::spawn(move || {
        for conn in l.incoming().take(6) {
            let Ok(mut s) = conn else { continue };
            let req = read_request(&mut s);
            let r = if req.contains("bad") {
                "HTTP/1.1 503 Service Unavailable\r\nConnection: close\r\n\r\n{}"
            } else {
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{\"choices\":[]}"
            };
            let _ = s.write_all(r.as_bytes());
        }
    });
    format!("http://{addr}/v1")
}

#[test]
fn the_newest_model_that_really_answers_is_picked() {
    let p = Provider::new("t", &serve_models(), "x");
    let ranked: Vec<String> = ["bad-3", "bad-2", "buono-1", "buono-0"]
        .map(String::from)
        .to_vec();
    assert_eq!(
        multiia::api::pick_working(&p, &ranked, 10),
        Some("buono-1".to_string())
    );
}

#[test]
fn no_working_model_gives_none() {
    let p = Provider::new("t", &serve_models(), "x");
    let ranked: Vec<String> = ["bad-1", "bad-2"].map(String::from).to_vec();
    assert_eq!(multiia::api::pick_working(&p, &ranked, 10), None);
}
