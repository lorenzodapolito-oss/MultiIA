//! Impostazioni: elenco delle IA (indirizzo, modello, chiave) e parametri generali.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const MAX_PROVIDERS: usize = 12;

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
    pub providers: Vec<Provider>,
    pub system_prompt: String,
    /// Secondi di attesa massima senza ricevere nulla da una IA.
    pub timeout_secs: u64,
    /// Lunghezza massima di ogni risposta in token (0 = nessun limite).
    pub max_tokens: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            providers: vec![
                Provider::new(
                    "ChatGPT (OpenAI)",
                    "https://api.openai.com/v1",
                    "gpt-6.1-sol",
                ),
                Provider::new(
                    "Claude (Anthropic)",
                    "https://api.anthropic.com/v1",
                    "claude-sonnet-5-5",
                ),
                Provider::new(
                    "Gemini (Google)",
                    "https://generativelanguage.googleapis.com/v1beta/openai",
                    "gemini-2.5-pro",
                ),
                Provider::new("DeepSeek", "https://api.deepseek.com", "deepseek-flash"),
                Provider::new("Grok (xAI)", "https://api.x.ai/v1", "grok-4.7"),
                Provider::new("GLM (Z.ai)", "https://api.z.ai/api/paas/v4", "glm-5.3"),
            ],
            system_prompt: String::new(),
            timeout_secs: 120,
            max_tokens: 4000,
        }
    }
}

/// Modelli già pronti da aggiungere con un clic.
pub fn presets() -> Vec<Provider> {
    vec![
        Provider::new(
            "ChatGPT (OpenAI)",
            "https://api.openai.com/v1",
            "gpt-6.1-sol",
        ),
        Provider::new(
            "Claude (Anthropic)",
            "https://api.anthropic.com/v1",
            "claude-sonnet-5-5",
        ),
        Provider::new(
            "Gemini (Google)",
            "https://generativelanguage.googleapis.com/v1beta/openai",
            "gemini-2.5-pro",
        ),
        Provider::new(
            "Mistral",
            "https://api.mistral.ai/v1",
            "mistral-large-latest",
        ),
        Provider::new("OpenRouter", "https://openrouter.ai/api/v1", ""),
        Provider::new("Ollama (sul tuo PC)", "http://localhost:11434/v1", ""),
        Provider::new("LM Studio (sul tuo PC)", "http://localhost:1234/v1", ""),
        Provider::new("DeepSeek", "https://api.deepseek.com", "deepseek-flash"),
        Provider::new("Grok (xAI)", "https://api.x.ai/v1", "grok-4.7"),
        Provider::new("GLM (Z.ai)", "https://api.z.ai/api/paas/v4", "glm-5.3"),
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
    fn presets_are_valid_urls() {
        assert!(presets().iter().all(|p| p.base_url.starts_with("http")));
    }
}
