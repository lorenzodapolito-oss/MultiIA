//! Prova con un finto servizio locale che risponde come le IA vere (streaming SSE ed errori).
#![allow(clippy::unwrap_used)]
use multiia::api::{list_models, stream_chat, Event, Message};
use multiia::config::Provider;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::AtomicBool;

fn serve(response: &'static str) -> String {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap();
    std::thread::spawn(move || {
        if let Ok((mut s, _)) = l.accept() {
            let mut buf = [0u8; 8192];
            let _ = s.read(&mut buf);
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
    assert_eq!(ev.len(), 1);
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
