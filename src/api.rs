//! Chiamate alle IA con l'interfaccia compatibile OpenAI (`/chat/completions`, risposta in streaming).

use crate::config::Provider;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

#[derive(Clone, Debug, PartialEq)]
pub struct Message {
    pub role: String,
    pub content: String,
}

impl Message {
    pub fn new(role: &str, content: &str) -> Self {
        Self {
            role: role.into(),
            content: content.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Text(String),
    /// Il "ragionamento" dei modelli che lo mostrano (DeepSeek, GLM).
    Reasoning(String),
    Usage {
        prompt: u64,
        completion: u64,
    },
    Done,
    Error(String),
}

pub fn chat_url(base: &str) -> String {
    format!("{}/chat/completions", base.trim().trim_end_matches('/'))
}

pub fn models_url(base: &str) -> String {
    format!("{}/models", base.trim().trim_end_matches('/'))
}

pub fn build_body(model: &str, system: &str, msgs: &[Message]) -> String {
    let mut m: Vec<Value> = Vec::new();
    if !system.trim().is_empty() {
        m.push(json!({"role": "system", "content": system}));
    }
    m.extend(
        msgs.iter()
            .map(|x| json!({"role": x.role, "content": x.content})),
    );
    json!({
        "model": model,
        "messages": m,
        "stream": true,
        "stream_options": {"include_usage": true},
    })
    .to_string()
}

/// Eventi contenuti in una riga `data: …` della risposta.
pub fn parse_data(data: &str) -> Vec<Event> {
    let data = data.trim();
    if data == "[DONE]" {
        return vec![Event::Done];
    }
    let Ok(v) = serde_json::from_str::<Value>(data) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    if let Some(e) = v.get("error") {
        out.push(Event::Error(error_text(e)));
        return out;
    }
    if let Some(delta) = v.pointer("/choices/0/delta") {
        if let Some(r) = delta
            .get("reasoning_content")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
        {
            out.push(Event::Reasoning(r.to_string()));
        }
        if let Some(t) = delta
            .get("content")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
        {
            out.push(Event::Text(t.to_string()));
        }
    }
    if let Some(u) = v.get("usage").filter(|u| u.is_object()) {
        let g = |k: &str| u.get(k).and_then(Value::as_u64).unwrap_or(0);
        out.push(Event::Usage {
            prompt: g("prompt_tokens"),
            completion: g("completion_tokens"),
        });
    }
    out
}

fn error_text(e: &Value) -> String {
    e.get("message")
        .and_then(Value::as_str)
        .or_else(|| e.as_str())
        .unwrap_or("errore sconosciuto")
        .chars()
        .take(300)
        .collect()
}

/// Messaggio in italiano per un errore HTTP, con il motivo dato dal servizio.
pub fn http_error(status: u16, body: &str) -> String {
    let detail = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| {
            v.get("error")
                .map(error_text)
                .or_else(|| v.get("message").and_then(Value::as_str).map(str::to_string))
        })
        .unwrap_or_else(|| body.trim().chars().take(200).collect());
    let what = match status {
        400 => "Richiesta non valida (controlla il nome del modello)",
        401 | 403 => "Chiave API non valida o senza permessi",
        402 => "Credito esaurito",
        404 => "Indirizzo o modello non trovato",
        408 => "Tempo scaduto",
        429 => "Troppe richieste o limite raggiunto",
        500..=599 => "Il servizio ha avuto un problema",
        _ => "Errore",
    };
    if detail.is_empty() {
        format!("{what} (HTTP {status})")
    } else {
        format!("{what} (HTTP {status}): {detail}")
    }
}

fn agent(timeout: u64) -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(20))
        .timeout_read(Duration::from_secs(timeout.clamp(10, 1800)))
        .timeout_write(Duration::from_secs(30))
        .build()
}

fn transport_error(e: &ureq::Error) -> String {
    let t = e.to_string();
    if t.to_lowercase().contains("timed out") {
        "Nessuna risposta in tempo".into()
    } else {
        format!("Connessione non riuscita: {t}")
    }
}

/// Invia la conversazione e passa a `emit` ogni pezzo di risposta appena arriva.
/// Finisce sempre con `Done` oppure `Error` (anche se annullata: `Done`).
pub fn stream_chat(
    p: &Provider,
    system: &str,
    msgs: &[Message],
    timeout: u64,
    cancel: &AtomicBool,
    emit: &mut dyn FnMut(Event),
) {
    let mut req = agent(timeout)
        .post(&chat_url(&p.base_url))
        .set("Content-Type", "application/json");
    if !p.api_key.trim().is_empty() {
        req = req.set("Authorization", &format!("Bearer {}", p.api_key.trim()));
    }
    let resp = match req.send_string(&build_body(&p.model, system, msgs)) {
        Ok(r) => r,
        Err(ureq::Error::Status(code, r)) => {
            let body = r.into_string().unwrap_or_default();
            emit(Event::Error(http_error(code, &body)));
            return;
        }
        Err(e) => {
            emit(Event::Error(transport_error(&e)));
            return;
        }
    };
    let mut got_text = false;
    for line in BufReader::new(resp.into_reader()).lines() {
        if cancel.load(Ordering::SeqCst) {
            emit(Event::Done);
            return;
        }
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                emit(Event::Error(if got_text {
                    format!("Risposta interrotta: {e}")
                } else {
                    "Nessuna risposta in tempo".into()
                }));
                return;
            }
        };
        let Some(data) = line.strip_prefix("data:") else {
            continue;
        };
        for ev in parse_data(data) {
            match &ev {
                Event::Text(_) | Event::Reasoning(_) => got_text = true,
                Event::Done => {
                    emit(Event::Done);
                    return;
                }
                Event::Error(_) => {
                    emit(ev);
                    return;
                }
                Event::Usage { .. } => {}
            }
            emit(ev);
        }
    }
    emit(Event::Done);
}

/// Elenco dei modelli offerti dal servizio (`GET /models`).
pub fn list_models(p: &Provider, timeout: u64) -> Result<Vec<String>, String> {
    let mut req = agent(timeout.min(30)).get(&models_url(&p.base_url));
    if !p.api_key.trim().is_empty() {
        req = req.set("Authorization", &format!("Bearer {}", p.api_key.trim()));
    }
    match req.call() {
        Ok(r) => {
            let body = r.into_string().map_err(|e| e.to_string())?;
            Ok(parse_models(&body))
        }
        Err(ureq::Error::Status(c, r)) => Err(http_error(c, &r.into_string().unwrap_or_default())),
        Err(e) => Err(transport_error(&e)),
    }
}

pub fn parse_models(body: &str) -> Vec<String> {
    let Ok(v) = serde_json::from_str::<Value>(body) else {
        return Vec::new();
    };
    let mut ids: Vec<String> = v
        .get("data")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|m| m.get("id").and_then(Value::as_str).map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    // I modelli ":batch" non si possono usare in chat; quelli ":free" vanno in cima.
    ids.retain(|m| !m.ends_with(":batch"));
    ids.sort_by(|a, b| (!a.ends_with(":free"), a).cmp(&(!b.ends_with(":free"), b)));
    ids.dedup();
    ids
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_are_joined_without_double_slashes() {
        assert_eq!(
            chat_url("https://a.b/v1/"),
            "https://a.b/v1/chat/completions"
        );
        assert_eq!(models_url("https://a.b"), "https://a.b/models");
    }

    #[test]
    fn body_has_system_prompt_only_when_present() {
        let m = [Message::new("user", "ciao")];
        let b: Value = serde_json::from_str(&build_body("m", "", &m)).unwrap();
        assert_eq!(b["messages"].as_array().map(Vec::len), Some(1));
        assert_eq!(b["stream"], true);
        let b: Value = serde_json::from_str(&build_body("m", "sii breve", &m)).unwrap();
        assert_eq!(b["messages"][0]["role"], "system");
        assert_eq!(b["messages"][1]["content"], "ciao");
    }

    #[test]
    fn parses_text_reasoning_usage_done_and_errors() {
        assert_eq!(
            parse_data(r#"{"choices":[{"delta":{"content":"Ci"}}]}"#),
            vec![Event::Text("Ci".into())]
        );
        assert_eq!(
            parse_data(r#"{"choices":[{"delta":{"reasoning_content":"penso","content":""}}]}"#),
            vec![Event::Reasoning("penso".into())]
        );
        assert_eq!(
            parse_data(r#"{"choices":[],"usage":{"prompt_tokens":5,"completion_tokens":7}}"#),
            vec![Event::Usage {
                prompt: 5,
                completion: 7
            }]
        );
        assert_eq!(parse_data(" [DONE] "), vec![Event::Done]);
        assert_eq!(
            parse_data(r#"{"error":{"message":"boom"}}"#),
            vec![Event::Error("boom".into())]
        );
        assert!(parse_data("non json").is_empty());
        assert!(parse_data(r#"{"choices":[{"delta":{"role":"assistant"}}]}"#).is_empty());
        assert!(parse_data(r#"{"choices":[{"delta":{"content":null}}],"usage":null}"#).is_empty());
    }

    #[test]
    fn http_errors_are_explained_in_italian() {
        let e = http_error(401, r#"{"error":{"message":"Incorrect API key"}}"#);
        assert!(e.contains("Chiave API") && e.contains("Incorrect API key") && e.contains("401"));
        assert!(http_error(402, "").contains("Credito"));
        assert!(http_error(429, "slow down").contains("slow down"));
        assert!(http_error(503, "").contains("servizio"));
        assert!(http_error(418, "").contains("418"));
    }

    #[test]
    fn batch_models_are_hidden_and_free_ones_come_first() {
        let b = r#"{"data":[{"id":"x/m"},{"id":"x/m:batch"},{"id":"a/z:free"},{"id":"b/y:free"}]}"#;
        assert_eq!(parse_models(b), vec!["a/z:free", "b/y:free", "x/m"]);
    }

    #[test]
    fn model_lists_are_sorted_and_unique() {
        assert_eq!(
            parse_models(r#"{"data":[{"id":"b"},{"id":"a"},{"id":"b"},{"x":1}]}"#),
            vec!["a", "b"]
        );
        assert!(parse_models("rotto").is_empty());
        assert!(parse_models("{}").is_empty());
    }
}
