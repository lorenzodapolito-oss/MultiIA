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
    /// Il servizio era sovraccarico: si riprova (tentativo `attempt` di `max`).
    Retry {
        attempt: u32,
        max: u32,
    },
    /// Il turno di questa IA è arrivato e la richiesta parte.
    Started,
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

/// Quante volte si riprova dopo un errore passeggero.
pub const RETRIES: u32 = 2;

/// 429 (limite), 502/503/504 (servizio sovraccarico o non raggiungibile): passano da soli.
pub fn is_retryable(status: u16) -> bool {
    matches!(status, 429 | 502 | 503 | 504)
}

/// Pausa prima del tentativo `n`: 2 secondi, poi 4.
pub fn retry_delay(n: u32) -> Duration {
    Duration::from_secs(2 * u64::from(n.max(1)))
}

/// Aspetta controllando ogni 100 ms se l'utente ha annullato. Falso se annullato.
fn wait(d: Duration, cancel: &AtomicBool) -> bool {
    let end = std::time::Instant::now() + d;
    while std::time::Instant::now() < end {
        if cancel.load(Ordering::SeqCst) {
            return false;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    !cancel.load(Ordering::SeqCst)
}

pub fn chat_url(base: &str) -> String {
    format!("{}/chat/completions", base.trim().trim_end_matches('/'))
}

pub fn models_url(base: &str) -> String {
    format!("{}/models", base.trim().trim_end_matches('/'))
}

pub fn build_body(model: &str, system: &str, msgs: &[Message], max_tokens: u32) -> String {
    let mut m: Vec<Value> = Vec::new();
    if !system.trim().is_empty() {
        m.push(json!({"role": "system", "content": system}));
    }
    m.extend(
        msgs.iter()
            .map(|x| json!({"role": x.role, "content": x.content})),
    );
    let mut body = json!({
        "model": model,
        "messages": m,
        "stream": true,
        "stream_options": {"include_usage": true},
    });
    // Senza un limite alcuni servizi (OpenRouter) riservano il massimo possibile e rifiutano
    // la richiesta se il credito non basta.
    if max_tokens > 0 {
        body["max_tokens"] = json!(max_tokens);
    }
    body.to_string()
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
    // Alcuni servizi (Gemini) mandano l'errore dentro una lista: `[{"error": {...}}]`.
    let parsed = serde_json::from_str::<Value>(body).ok().map(|v| match v {
        Value::Array(mut a) if !a.is_empty() => a.remove(0),
        other => other,
    });
    let detail = parsed
        .as_ref()
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
    max_tokens: u32,
    cancel: &AtomicBool,
    emit: &mut dyn FnMut(Event),
) {
    let body = build_body(&p.model, system, msgs, max_tokens);
    let mut attempt = 0;
    let resp = loop {
        let mut req = agent(timeout)
            .post(&chat_url(&p.base_url))
            .set("Content-Type", "application/json");
        if !p.api_key.trim().is_empty() {
            req = req.set("Authorization", &format!("Bearer {}", p.api_key.trim()));
        }
        if p.base_url.contains("openrouter.ai") {
            req = req.set("X-Title", "MultiIA");
        }
        match req.send_string(&body) {
            Ok(r) => break r,
            Err(ureq::Error::Status(code, r)) => {
                // Servizio sovraccarico o limite al minuto: si riprova dopo una pausa.
                if is_retryable(code) && attempt < RETRIES {
                    attempt += 1;
                    emit(Event::Retry {
                        attempt,
                        max: RETRIES,
                    });
                    if !wait(retry_delay(attempt), cancel) {
                        emit(Event::Done);
                        return;
                    }
                    continue;
                }
                let text = r.into_string().unwrap_or_default();
                emit(Event::Error(http_error(code, &text)));
                return;
            }
            Err(e) => {
                emit(Event::Error(transport_error(&e)));
                return;
            }
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
                Event::Usage { .. } | Event::Started | Event::Retry { .. } => {}
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
            Ok(if p.base_url.contains("openrouter.ai") {
                parse_free_models(&body)
            } else {
                parse_models(&body)
            })
        }
        Err(ureq::Error::Status(c, r)) => Err(http_error(c, &r.into_string().unwrap_or_default())),
        Err(e) => Err(transport_error(&e)),
    }
}

pub const OPENROUTER_URL: &str = "https://openrouter.ai/api/v1";

/// Un modello si può scegliere solo se è attivo (non dismesso), scrive testo ed è gratuito.
/// `explicit_free`: il servizio mescola modelli a pagamento e gratuiti (OpenRouter), quindi la
/// gratuità deve essere dichiarata; negli altri, senza prezzi nell'elenco, vale il piano gratuito.
fn is_active_and_free(m: &Value, explicit_free: bool) -> bool {
    let id = m.get("id").and_then(Value::as_str).unwrap_or("");
    if id.is_empty()
        || id.ends_with(":batch")
        || id.starts_with("openrouter/")
        || !is_chat_model(id)
    {
        return false;
    }
    // Inattivo o dismesso: Groq usa "active", altri "deprecated", Mistral una data in "deprecation".
    if m.get("active").and_then(Value::as_bool) == Some(false)
        || m.get("deprecated").and_then(Value::as_bool) == Some(true)
        || m.get("deprecation").is_some_and(|d| !d.is_null())
    {
        return false;
    }
    let text_out = m
        .pointer("/architecture/output_modalities")
        .and_then(Value::as_array)
        .is_none_or(|o| o.iter().any(|x| x.as_str() == Some("text")));
    if !text_out {
        return false;
    }
    let price = |k: &str| {
        m.pointer(&format!("/pricing/{k}")).and_then(|v| {
            v.as_str()
                .and_then(|s| s.trim().parse::<f64>().ok())
                .or_else(|| v.as_f64())
        })
    };
    if id.ends_with(":free") || m.get("free").and_then(Value::as_bool) == Some(true) {
        return true;
    }
    match (price("prompt"), price("completion")) {
        (Some(p), Some(c)) => p == 0.0 && c == 0.0,
        _ => !explicit_free,
    }
}

/// Modelli gratuiti e attivi di OpenRouter che rispondono con testo.
pub fn parse_free_models(body: &str) -> Vec<String> {
    collect_models(body, true)
}

fn collect_models(body: &str, explicit_free: bool) -> Vec<String> {
    let Ok(v) = serde_json::from_str::<Value>(body) else {
        return Vec::new();
    };
    let mut ids: Vec<String> = v
        .get("data")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter(|m| is_active_and_free(m, explicit_free))
                .filter_map(|m| m.get("id").and_then(Value::as_str).map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    // Gemini elenca i nomi come «models/nome»; la chat li accetta anche senza prefisso.
    for m in &mut ids {
        if let Some(r) = m.strip_prefix("models/") {
            *m = r.to_string();
        }
    }
    // I ":free" vanno in cima.
    ids.sort_by(|a, b| (!a.ends_with(":free"), a).cmp(&(!b.ends_with(":free"), b)));
    ids.dedup();
    ids
}

/// Elenco dei modelli gratuiti di OpenRouter (non serve la chiave).
pub fn list_free_models(timeout: u64) -> Result<Vec<String>, String> {
    match agent(timeout.min(30))
        .get(&models_url(OPENROUTER_URL))
        .call()
    {
        Ok(r) => Ok(parse_free_models(
            &r.into_string().map_err(|e| e.to_string())?,
        )),
        Err(ureq::Error::Status(c, r)) => Err(http_error(c, &r.into_string().unwrap_or_default())),
        Err(e) => Err(transport_error(&e)),
    }
}

const NOT_CHAT: [&str; 15] = [
    "tts",
    "embed",
    "imagen",
    "image",
    "veo",
    "audio",
    "live",
    "whisper",
    "moderation",
    "rerank",
    "transcribe",
    "aqa",
    "guard",
    "dall-e",
    "speech",
];

/// Falso per i modelli che non sono pensati per la chat testuale.
pub fn is_chat_model(id: &str) -> bool {
    let l = id.to_lowercase();
    !NOT_CHAT.iter().any(|w| l.contains(w))
}

/// Modelli attivi e utilizzabili gratuitamente offerti dal servizio.
pub fn parse_models(body: &str) -> Vec<String> {
    collect_models(body, false)
}

const FAST: [&str; 5] = ["flash", "small", "mini", "instant", "versatile"];

/// Prime due cifre del nome («gemini-3.8-flash» → [3, 8]): il numero di versione.
fn version_key(id: &str) -> Vec<u64> {
    id.split(|c: char| !c.is_ascii_digit())
        .filter(|g| !g.is_empty())
        .filter_map(|g| g.parse::<u64>().ok())
        .take(2)
        .collect()
}

/// Modelli attivi e gratuiti dal più recente al meno recente. Conta, nell'ordine: la stessa
/// famiglia del modello attuale (`family`, per esempio «gemini»), la data di uscita (`created`)
/// se il servizio la dà, il numero di versione, le versioni veloci (flash, small, mini...) che
/// reggono meglio il piano gratuito, poi le versioni stabili prima di preview ed esperimenti.
pub fn newest_first(body: &str, explicit_free: bool, family: &str) -> Vec<String> {
    let Ok(v) = serde_json::from_str::<Value>(body) else {
        return Vec::new();
    };
    let fam = family.trim().to_lowercase();
    let fam = fam
        .split(['-', '/', ':', '.'])
        .next()
        .unwrap_or("")
        .to_string();
    let mut items: Vec<(String, i64)> = v
        .get("data")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter(|m| is_active_and_free(m, explicit_free))
                .filter_map(|m| {
                    let id = m.get("id").and_then(Value::as_str)?;
                    let id = id.strip_prefix("models/").unwrap_or(id).to_string();
                    Some((id, m.get("created").and_then(Value::as_i64).unwrap_or(0)))
                })
                .collect()
        })
        .unwrap_or_default();
    let score = |id: &str| {
        let l = id.to_lowercase();
        (
            !fam.is_empty() && l.starts_with(&fam),
            l.split(|c: char| !c.is_ascii_alphanumeric())
                .any(|t| FAST.contains(&t)),
            !(l.contains("preview") || l.contains("exp")),
        )
    };
    items.sort_by(|(a, ca), (b, cb)| {
        let (sa, sb) = (score(a), score(b));
        sb.0.cmp(&sa.0)
            .then(cb.cmp(ca))
            .then(version_key(b).cmp(&version_key(a)))
            .then(sb.1.cmp(&sa.1))
            .then(sb.2.cmp(&sa.2))
            .then(a.len().cmp(&b.len()))
            .then(a.cmp(b))
    });
    items.dedup_by(|a, b| a.0 == b.0);
    items.into_iter().map(|(id, _)| id).collect()
}

/// Elenco per il menu e, separato, lo stesso elenco dal più recente (`GET /models`).
pub fn list_models_ranked(
    p: &Provider,
    timeout: u64,
) -> Result<(Vec<String>, Vec<String>), String> {
    let mut req = agent(timeout.min(30)).get(&models_url(&p.base_url));
    if !p.api_key.trim().is_empty() {
        req = req.set("Authorization", &format!("Bearer {}", p.api_key.trim()));
    }
    match req.call() {
        Ok(r) => {
            let body = r.into_string().map_err(|e| e.to_string())?;
            let or = p.base_url.contains("openrouter.ai");
            let list = if or {
                parse_free_models(&body)
            } else {
                parse_models(&body)
            };
            Ok((list, newest_first(&body, or, &p.model)))
        }
        Err(ureq::Error::Status(c, r)) => Err(http_error(c, &r.into_string().unwrap_or_default())),
        Err(e) => Err(transport_error(&e)),
    }
}

/// Prova il modello con una richiesta minuscola. Un 429 (limite al minuto) conta come
/// «funziona»: il modello c'è, è solo il momento sbagliato.
pub fn probe_model(p: &Provider, model: &str, timeout: u64) -> Result<(), String> {
    let body = json!({
        "model": model,
        "messages": [{"role": "user", "content": "ciao"}],
        "max_tokens": 8,
    })
    .to_string();
    let mut req = agent(timeout.min(30))
        .post(&chat_url(&p.base_url))
        .set("Content-Type", "application/json");
    if !p.api_key.trim().is_empty() {
        req = req.set("Authorization", &format!("Bearer {}", p.api_key.trim()));
    }
    match req.send_string(&body) {
        Ok(_) => Ok(()),
        Err(ureq::Error::Status(429, _)) => Ok(()),
        Err(ureq::Error::Status(c, r)) => Err(http_error(c, &r.into_string().unwrap_or_default())),
        Err(e) => Err(transport_error(&e)),
    }
}

/// Il primo modello che risponde davvero tra i più recenti (se ne provano al massimo 4).
pub fn pick_working(p: &Provider, ranked: &[String], timeout: u64) -> Option<String> {
    ranked
        .iter()
        .take(4)
        .find(|m| probe_model(p, m, timeout).is_ok())
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_passing_errors_are_retried() {
        for c in [429, 502, 503, 504] {
            assert!(is_retryable(c), "{c}");
        }
        for c in [400, 401, 402, 404, 500] {
            assert!(!is_retryable(c), "{c}");
        }
        assert_eq!(retry_delay(1), Duration::from_secs(2));
        assert_eq!(retry_delay(2), Duration::from_secs(4));
    }

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
        let b: Value = serde_json::from_str(&build_body("m", "", &m, 0)).unwrap();
        assert_eq!(b["messages"].as_array().map(Vec::len), Some(1));
        assert_eq!(b["stream"], true);
        let b: Value = serde_json::from_str(&build_body("m", "sii breve", &m, 500)).unwrap();
        assert_eq!(b["messages"][0]["role"], "system");
        assert_eq!(b["messages"][1]["content"], "ciao");
        assert_eq!(b["max_tokens"], 500);
        let none: Value = serde_json::from_str(&build_body("m", "", &m, 0)).unwrap();
        assert!(none.get("max_tokens").is_none());
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
    fn only_free_text_models_are_kept() {
        let b = r#"{"data":[
          {"id":"a/free:free","pricing":{"prompt":"0","completion":"0"}},
          {"id":"b/zero","pricing":{"prompt":"0","completion":"0"}},
          {"id":"c/paid","pricing":{"prompt":"0.000001","completion":"0.000002"}},
          {"id":"d/batch:batch","pricing":{"prompt":"0","completion":"0"}},
          {"id":"e/img:free","architecture":{"output_modalities":["image"]}},
          {"id":"openrouter/auto","pricing":{"prompt":"0","completion":"0"}},
          {"id":"f/mix:free","architecture":{"output_modalities":["text","image"]}},
          {"x":1}]}"#;
        assert_eq!(
            parse_free_models(b),
            vec!["a/free:free", "f/mix:free", "b/zero"]
        );
        assert!(parse_free_models("rotto").is_empty());
    }

    #[test]
    fn batch_models_are_hidden_and_free_ones_come_first() {
        let b = r#"{"data":[{"id":"x/m"},{"id":"x/m:batch"},{"id":"a/z:free"},{"id":"b/y:free"}]}"#;
        assert_eq!(parse_models(b), vec!["a/z:free", "b/y:free", "x/m"]);
    }

    #[test]
    fn errors_wrapped_in_a_list_are_read_too() {
        let e = http_error(404, r#"[{"error":{"code":404,"message":"model gone"}}]"#);
        assert!(e.contains("model gone") && !e.contains("[{"));
    }

    #[test]
    fn non_chat_models_are_hidden_and_gemini_prefix_removed() {
        let b = r#"{"data":[{"id":"models/gemini-3.8-flash"},{"id":"models/gemini-3.8-flash-tts"},{"id":"models/text-embedding-9"},{"id":"models/imagen-5"},{"id":"gpt-image-2"},{"id":"whisper-large"},{"id":"llama-3.3-70b"}]}"#;
        assert_eq!(parse_models(b), vec!["gemini-3.8-flash", "llama-3.3-70b"]);
        assert!(is_chat_model("mistral-small-latest") && !is_chat_model("x-TTS-y"));
    }

    #[test]
    fn inactive_deprecated_and_paid_models_cannot_be_chosen() {
        let b = r#"{"data":[
          {"id":"ok"},
          {"id":"spento","active":false},
          {"id":"vecchio","deprecated":true},
          {"id":"mistral-old","deprecation":"2026-12-01T00:00:00Z"},
          {"id":"mistral-new","deprecation":null},
          {"id":"costa","pricing":{"prompt":"0.001","completion":"0.002"}},
          {"id":"gratis","pricing":{"prompt":0,"completion":0}}]}"#;
        assert_eq!(parse_models(b), vec!["gratis", "mistral-new", "ok"]);
        // Su OpenRouter la gratuità deve essere dichiarata: senza prezzi non vale.
        assert_eq!(parse_free_models(b), vec!["gratis"]);
    }

    #[test]
    fn newest_models_come_first() {
        // Con la data di uscita vince la più recente.
        let b = r#"{"data":[{"id":"vecchio","created":100},{"id":"nuovo","created":900},{"id":"medio","created":500}]}"#;
        assert_eq!(
            newest_first(b, false, ""),
            vec!["nuovo", "medio", "vecchio"]
        );
        // Senza data conta la versione; a pari versione la veloce e stabile prima.
        let g = r#"{"data":[{"id":"models/gemini-2.5-flash"},{"id":"models/gemini-3.8-pro"},{"id":"models/gemini-3.8-flash"},{"id":"models/gemini-3.8-flash-preview"},{"id":"models/gemma-9-it"}]}"#;
        let r = newest_first(g, false, "gemini-2.5-flash");
        assert_eq!(r[0], "gemini-3.8-flash");
        assert!(
            r.iter().position(|m| m == "gemini-3.8-flash")
                < r.iter().position(|m| m == "gemini-3.8-pro")
        );
        assert!(
            r.iter().position(|m| m == "gemini-3.8-flash")
                < r.iter().position(|m| m == "gemini-3.8-flash-preview")
        );
        // La famiglia del modello attuale ha la precedenza.
        assert_eq!(r.last().map(String::as_str), Some("gemma-9-it"));
        assert!(newest_first("rotto", false, "x").is_empty());
        assert_eq!(version_key("gemini-3.8-flash"), vec![3, 8]);
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
