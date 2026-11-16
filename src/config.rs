//! Impostazioni: elenco delle IA (indirizzo, modello, chiave) e parametri generali.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const MAX_PROVIDERS: usize = 12;
/// Versione dell'elenco predefinito: se il file è più vecchio, si toglie ciò che è a pagamento.
const VERSION: u32 = 2;

fn zero() -> u32 {
    0
}

/// Servizi a pagamento tolti dall'elenco dalla versione 2.
const PAID_HOSTS: [&str; 6] = [
    "api.openai.com",
    "api.anthropic.com",
    "api.deepseek.com",
    "api.x.ai",
    "api.z.ai",
    "openrouter.ai",
];

fn host(url: &str) -> String {
    url.split("://")
        .last()
        .unwrap_or("")
        .split('/')
        .next()
        .unwrap_or("")
        .to_lowercase()
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Provider {
    pub id: String,
    pub name: String,
    /// Indirizzo base compatibile OpenAI, senza `/chat/completions`.
    pub base_url: String,
    pub model: String,
    pub api_key: String,
    pub enabled: bool,
}

impl Default for Provider {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: "Nuova IA".into(),
            base_url: "https://".into(),
            model: String::new(),
            api_key: String::new(),
            enabled: true,
        }
    }
}

impl Provider {
    pub fn new(name: &str, base_url: &str, model: &str) -> Self {
        Self {
            id: new_id(),
            name: name.into(),
            base_url: base_url.into(),
            model: model.into(),
            ..Self::default()
        }
    }

    /// I modelli locali (Ollama, LM Studio) non vogliono la chiave.
    pub fn is_local(&self) -> bool {
        let u = self.base_url.to_lowercase();
        u.contains("://localhost") || u.contains("://127.0.0.1") || u.contains("://[::1]")
    }

    pub fn needs_key(&self) -> bool {
        !self.is_local()
    }

    pub fn ready(&self) -> Result<(), &'static str> {
        if self.needs_key() && self.api_key.trim().is_empty() {
            return Err("Manca la chiave API: inseriscila in Impostazioni.");
        }
        if self.model.trim().is_empty() {
            return Err("Manca il nome del modello: scrivilo in Impostazioni.");
        }
        if !(self.base_url.starts_with("http://") || self.base_url.starts_with("https://")) {
            return Err("L'indirizzo deve iniziare con http:// o https://");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    #[serde(default = "zero")]
    pub version: u32,
    pub providers: Vec<Provider>,
    pub system_prompt: String,
    /// Secondi di attesa massima senza ricevere nulla da una IA.
    pub timeout_secs: u64,
    /// Lunghezza massima di ogni risposta in token (0 = nessun limite).
    pub max_tokens: u32,
    /// Chiave della pagina OpenRouter.
    pub openrouter_key: String,
    /// Modelli gratuiti di OpenRouter che l'utente ha spento.
    pub or_disabled: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            version: VERSION,
            providers: free_providers(),
            system_prompt: String::new(),
            timeout_secs: 120,
            max_tokens: 4000,
            openrouter_key: String::new(),
            or_disabled: Vec::new(),
        }
    }
}

/// Modelli già pronti da aggiungere con un clic.
pub fn presets() -> Vec<Provider> {
    let mut v = free_providers();
    v.push(Provider::new(
        "Ollama (sul tuo PC)",
        "http://localhost:11434/v1",
        "",
    ));
    v.push(Provider::new(
        "LM Studio (sul tuo PC)",
        "http://localhost:1234/v1",
        "",
    ));
    v
}

/// Servizi con un piano gratuito per l'API (limiti e nomi dei modelli possono cambiare:
/// «Cerca modelli» in Impostazioni mostra quelli disponibili).
pub fn free_providers() -> Vec<Provider> {
    vec![
        Provider::new(
            "Gemini (Google)",
            "https://generativelanguage.googleapis.com/v1beta/openai",
            "gemini-3.8-flash",
        ),
        Provider::new(
            "Groq",
            "https://api.groq.com/openai/v1",
            "llama-3.3-70b-versatile",
        ),
        Provider::new("Cerebras", "https://api.cerebras.ai/v1", "llama-3.3-70b"),
        Provider::new(
            "Mistral",
            "https://api.mistral.ai/v1",
            "mistral-small-latest",
        ),
        Provider::new(
            "NVIDIA",
            "https://integrate.api.nvidia.com/v1",
            "meta/llama-3.3-70b-instruct",
        ),
        Provider::new(
            "GitHub Models",
            "https://models.github.ai/inference",
            "openai/gpt-4.1-mini",
        ),
    ]
}

static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

pub fn new_id() -> String {
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    format!("p{t:x}-{n}")
}

impl Settings {
    /// Porta un file vecchio all'elenco attuale: toglie i servizi a pagamento (tenendo
    /// le chiavi dei servizi che restano) e aggiunge quelli gratuiti che mancano.
    pub fn migrated(mut self) -> Self {
        if self.version >= VERSION {
            return self;
        }
        self.providers
            .retain(|p| !PAID_HOSTS.iter().any(|h| host(&p.base_url).contains(h)));
        for d in free_providers() {
            if !self
                .providers
                .iter()
                .any(|p| host(&p.base_url) == host(&d.base_url))
            {
                self.providers.push(d);
            }
        }
        self.version = VERSION;
        self
    }

    /// Rimette in ordine ciò che arriva da un file modificato a mano.
    pub fn sanitized(mut self) -> Self {
        self.providers.truncate(MAX_PROVIDERS);
        let mut seen = Vec::new();
        for p in &mut self.providers {
            if p.id.trim().is_empty() || seen.contains(&p.id) {
                p.id = new_id();
            }
            seen.push(p.id.clone());
            p.name = p.name.trim().chars().take(40).collect();
            if p.name.is_empty() {
                p.name = "IA".into();
            }
            p.base_url = p.base_url.trim().trim_end_matches('/').to_string();
            p.model = p.model.trim().to_string();
            p.api_key = p.api_key.trim().to_string();
        }
        self.system_prompt = self.system_prompt.chars().take(4000).collect();
        self.timeout_secs = self.timeout_secs.clamp(10, 1800);
        self.max_tokens = self.max_tokens.min(200_000);
        self.openrouter_key = self.openrouter_key.trim().to_string();
        self.or_disabled.truncate(2000);
        self
    }
}

pub fn data_dir() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA")
        .or_else(|| std::env::var_os("XDG_DATA_HOME"))
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local").join("share")))
        .unwrap_or_else(std::env::temp_dir);
    base.join("MultiIA")
}

pub fn load(path: &std::path::Path) -> Settings {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str::<Settings>(&t).ok())
        .unwrap_or_default()
        .migrated()
        .sanitized()
}

pub fn save(path: &std::path::Path, s: &Settings) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let t = serde_json::to_string_pretty(s).map_err(std::io::Error::other)?;
    std::fs::write(path, t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_have_six_distinct_providers() {
        let s = Settings::default();
        assert_eq!(s.providers.len(), 6);
        let mut ids: Vec<_> = s.providers.iter().map(|p| p.id.clone()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), 6);
        assert!(s
            .providers
            .iter()
            .all(|p| p.base_url.starts_with("https://")));
    }

    #[test]
    fn sanitizing_fixes_hand_edited_files() {
        let mut s = Settings::default();
        s.providers[0].base_url = "  https://api.x.com/v1///  ".into();
        s.providers[0].name = "   ".into();
        s.providers[1].id = s.providers[0].id.clone();
        s.timeout_secs = 0;
        s.system_prompt = "x".repeat(10_000);
        let s = s.sanitized();
        assert_eq!(s.providers[0].base_url, "https://api.x.com/v1");
        assert_eq!(s.providers[0].name, "IA");
        assert_ne!(s.providers[0].id, s.providers[1].id);
        assert_eq!(s.timeout_secs, 10);
        assert_eq!(
            Settings {
                max_tokens: u32::MAX,
                ..Settings::default()
            }
            .sanitized()
            .max_tokens,
            200_000
        );
        assert_eq!(Settings::default().max_tokens, 4000);
        assert_eq!(s.system_prompt.chars().count(), 4000);
        let many = Settings {
            providers: (0..50).map(|_| Provider::default()).collect(),
            ..Settings::default()
        };
        assert_eq!(many.sanitized().providers.len(), MAX_PROVIDERS);
    }

    #[test]
    fn readiness_checks() {
        let mut p = Provider::new("x", "https://a.b/v1", "m");
        assert!(p.ready().is_err(), "serve la chiave");
        p.api_key = "k".into();
        assert!(p.ready().is_ok());
        p.model.clear();
        assert!(p.ready().is_err());
        p.model = "m".into();
        p.base_url = "ftp://x".into();
        assert!(p.ready().is_err());
        let local = Provider::new("o", "http://localhost:11434/v1", "llama");
        assert!(local.is_local() && local.ready().is_ok());
    }

    #[test]
    fn save_and_load_round_trip_and_broken_file() {
        let d = std::env::temp_dir().join(format!("mi_cfg_{}", std::process::id()));
        let f = d.join("s.json");
        let mut s = Settings::default();
        s.providers[0].api_key = "segreta".into();
        save(&f, &s).unwrap();
        assert_eq!(load(&f).providers[0].api_key, "segreta");
        std::fs::write(&f, "{rotto").unwrap();
        assert_eq!(load(&f).providers.len(), 6);
        assert_eq!(load(&d.join("manca.json")).providers.len(), 6);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn old_files_lose_paid_services_but_keep_keys_of_the_rest() {
        let mut old = Settings {
            version: 0,
            providers: vec![
                Provider::new("ChatGPT", "https://api.openai.com/v1", "gpt"),
                Provider::new("Grok", "https://api.x.ai/v1", "g"),
                Provider::new(
                    "Gemini",
                    "https://generativelanguage.googleapis.com/v1beta/openai",
                    "gemini-x",
                ),
                Provider::new("Mia", "https://example.org/v1", "m"),
            ],
            ..Settings::default()
        };
        old.providers[2].api_key = "chiave-gemini".into();
        let s = old.migrated();
        assert_eq!(s.version, VERSION);
        let names: Vec<_> = s.providers.iter().map(|p| p.name.as_str()).collect();
        assert!(!names.contains(&"ChatGPT") && !names.contains(&"Grok"));
        assert!(names.contains(&"Mia") && names.contains(&"Groq") && names.contains(&"Cerebras"));
        assert_eq!(
            s.providers
                .iter()
                .find(|p| p.name == "Gemini")
                .map(|p| p.api_key.as_str()),
            Some("chiave-gemini")
        );
        assert_eq!(
            s.providers
                .iter()
                .filter(|p| p.base_url.contains("generativelanguage"))
                .count(),
            1
        );
        // Un file già aggiornato non cambia.
        assert_eq!(s.clone().migrated(), s);
    }

    #[test]
    fn a_file_without_version_counts_as_old() {
        let s: Settings = serde_json::from_str(r#"{"providers":[]}"#).unwrap();
        assert_eq!(s.version, 0);
        assert!(!s.migrated().providers.is_empty());
    }

    #[test]
    fn no_default_or_preset_is_a_paid_service() {
        for p in free_providers().iter().chain(presets().iter()) {
            assert!(
                !PAID_HOSTS.iter().any(|h| host(&p.base_url).contains(h)),
                "{}",
                p.name
            );
        }
    }

    #[test]
    fn presets_are_valid_urls() {
        assert!(presets().iter().all(|p| p.base_url.starts_with("http")));
    }
}
