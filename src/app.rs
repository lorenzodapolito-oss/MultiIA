//! Interfaccia: due pagine. «Tutte le IA» (una colonna per ogni servizio) e «OpenRouter»
//! (una chat per ogni modello gratuito).

use crate::api::{self, Event, Message};
use crate::config::{self, Provider, Settings};
use crate::session::{duration_text, export_markdown, Panel, Status};
use crate::theme;
use eframe::egui::{self, Color32, RichText};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};

pub const APP_TITLE: &str = "MultiIA";
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
/// Quante richieste OpenRouter partono insieme: i modelli gratuiti hanno limiti al minuto.
const OR_WORKERS: usize = 4;
const OR_COLUMN_WIDTH: f32 = 340.0;

const COLORS: [Color32; 6] = [
    theme::BTN_BLUE,
    theme::BTN_MINT,
    theme::BTN_PEACH,
    theme::BTN_LILAC,
    theme::BTN_PINK,
    theme::BTN_YELLOW,
];

enum Msg {
    Chat {
        id: String,
        run: u64,
        ev: Event,
    },
    Models {
        id: String,
        res: Result<Vec<String>, String>,
    },
    FreeModels(Result<Vec<String>, String>),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    All,
    OpenRouter,
}

struct Job {
    p: Provider,
    msgs: Vec<Message>,
    run: u64,
}

pub struct App {
    settings: Settings,
    path: PathBuf,
    page: Page,
    panels: Vec<Panel>,
    or_models: Vec<String>,
    or_panels: Vec<Panel>,
    or_note: String,
    or_loading: bool,
    prompt: String,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    cancel: Arc<AtomicBool>,
    show_settings: bool,
    message: String,
    models: HashMap<String, Vec<String>>,
    model_note: HashMap<String, String>,
}

fn color_button(ui: &mut egui::Ui, text: &str, color: Color32) -> egui::Response {
    ui.add(egui::Button::new(RichText::new(text).color(theme::ON_COLOR).strong()).fill(color))
}

/// Provider "virtuale" per un modello gratuito di OpenRouter.
pub fn or_provider(model: &str, key: &str) -> Provider {
    Provider {
        id: model.to_string(),
        name: model.to_string(),
        base_url: api::OPENROUTER_URL.to_string(),
        model: model.to_string(),
        api_key: key.to_string(),
        enabled: true,
    }
}

/// Una colonna: intestazione colorata, stato, conversazione. Restituisce la risposta da copiare.
fn column_ui(
    ui: &mut egui::Ui,
    name: &str,
    model: &str,
    color: Color32,
    panel: &Panel,
    id: &str,
) -> Option<String> {
    let mut copy = None;
    egui::Frame::new()
        .fill(color)
        .corner_radius(10)
        .inner_margin(8)
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new(name)
                        .strong()
                        .size(15.0)
                        .color(theme::ON_COLOR),
                );
                if !model.is_empty() {
                    ui.label(RichText::new(model).color(theme::ON_COLOR));
                }
            });
            let mut st = match &panel.status {
                Status::Idle => String::new(),
                Status::Queued => "in coda".into(),
                Status::Running => format!("in corso · {}", duration_text(panel.elapsed())),
                Status::Done => format!("fatto in {}", duration_text(panel.elapsed())),
                Status::Failed(_) => "errore".into(),
            };
            if let Some((a, b)) = panel.usage {
                st.push_str(&format!(" · {a}+{b} token"));
            }
            ui.label(RichText::new(st).color(theme::ON_COLOR));
        });
    if let Some(t) = panel.history.iter().rev().find(|m| m.role == "assistant") {
        if ui.small_button("Copia ultima risposta").clicked() {
            copy = Some(t.content.clone());
        }
    }
    egui::ScrollArea::vertical()
        .id_salt(("col", id))
        .stick_to_bottom(panel.is_running())
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for m in &panel.history {
                if m.role == "user" {
                    ui.label(RichText::new(&m.content).italics().color(theme::MUTED));
                } else {
                    ui.add(egui::Label::new(&m.content).selectable(true));
                }
                ui.add_space(6.0);
            }
            if !panel.reasoning.is_empty() {
                egui::CollapsingHeader::new("Ragionamento")
                    .id_salt(("r", id))
                    .default_open(panel.status == Status::Running)
                    .show(ui, |ui| {
                        ui.label(RichText::new(&panel.reasoning).color(theme::MUTED));
                    });
            }
            if panel.status == Status::Running && !panel.text.is_empty() {
                ui.add(egui::Label::new(&panel.text).selectable(true));
            }
            if let Status::Failed(e) = &panel.status {
                ui.colored_label(theme::ERR, e);
            }
        });
    copy
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::apply(&cc.egui_ctx);
        let path = config::data_dir().join("impostazioni.json");
        let (tx, rx) = channel();
        let mut app = Self {
            settings: config::load(&path),
            path,
            page: Page::All,
            panels: Vec::new(),
            or_models: Vec::new(),
            or_panels: Vec::new(),
            or_note: String::new(),
            or_loading: false,
            prompt: String::new(),
            tx,
            rx,
            cancel: Arc::new(AtomicBool::new(false)),
            show_settings: false,
            message: String::new(),
            models: HashMap::new(),
            model_note: HashMap::new(),
        };
        app.sync_panels();
        if app
            .settings
            .providers
            .iter()
            .all(|p| p.needs_key() && p.api_key.is_empty())
        {
            app.show_settings = true;
            app.message = "Per cominciare inserisci le chiavi API delle IA che vuoi usare.".into();
        }
        app
    }

    fn sync_panels(&mut self) {
        let old = std::mem::take(&mut self.panels);
        self.panels = self
            .settings
            .providers
            .iter()
            .map(|p| {
                old.iter()
                    .find(|o| o.provider_id == p.id)
                    .cloned()
                    .unwrap_or_else(|| Panel::new(&p.id))
            })
            .collect();
    }

    fn sync_or_panels(&mut self) {
        let old = std::mem::take(&mut self.or_panels);
        self.or_panels = self
            .or_models
            .iter()
            .map(|m| {
                old.iter()
                    .find(|o| &o.provider_id == m)
                    .cloned()
                    .unwrap_or_else(|| Panel::new(m))
            })
            .collect();
    }

    fn or_enabled(&self, model: &str) -> bool {
        !self.settings.or_disabled.iter().any(|d| d == model)
    }

    fn page_panels(&self) -> &[Panel] {
        match self.page {
            Page::All => &self.panels,
            Page::OpenRouter => &self.or_panels,
        }
    }

    fn any_running(&self) -> bool {
        self.page_panels().iter().any(Panel::is_running)
    }

    fn save_settings(&mut self) {
        self.settings = self.settings.clone().sanitized();
        self.sync_panels();
        if let Err(e) = config::save(&self.path, &self.settings) {
            self.message = format!("Non riesco a salvare le impostazioni: {e}");
        }
    }

    /// Fa partire le richieste con al massimo `workers` thread; chi aspetta resta «in coda».
    fn run_jobs(&mut self, ctx: &egui::Context, jobs: Vec<Job>, workers: usize) {
        if jobs.is_empty() {
            return;
        }
        // Ogni invio ha il suo segnale di annullamento: i vecchi lavori in coda non ripartono.
        self.cancel = Arc::new(AtomicBool::new(false));
        let n = workers.clamp(1, jobs.len());
        let queue = Arc::new(Mutex::new(VecDeque::from(jobs)));
        for _ in 0..n {
            let (queue, tx, cancel, ctx) = (
                queue.clone(),
                self.tx.clone(),
                self.cancel.clone(),
                ctx.clone(),
            );
            let (system, timeout, max) = (
                self.settings.system_prompt.clone(),
                self.settings.timeout_secs,
                self.settings.max_tokens,
            );
            std::thread::spawn(move || loop {
                let job = queue.lock().ok().and_then(|mut q| q.pop_front());
                let Some(Job { p, msgs, run }) = job else {
                    break;
                };
                if cancel.load(Ordering::SeqCst) {
                    break;
                }
                let send = |ev: Event| {
                    let _ = tx.send(Msg::Chat {
                        id: p.id.clone(),
                        run,
                        ev,
                    });
                    ctx.request_repaint();
                };
                send(Event::Started);
                let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    api::stream_chat(&p, &system, &msgs, timeout, max, &cancel, &mut |ev| {
                        send(ev)
                    });
                }));
                if r.is_err() {
                    send(Event::Error("Errore imprevisto nella richiesta".into()));
                }
            });
        }
    }

    fn send(&mut self, ctx: &egui::Context) {
        let q = self.prompt.trim().to_string();
        if q.is_empty() || self.any_running() {
            return;
        }
        let mut jobs = Vec::new();
        let workers;
        match self.page {
            Page::All => {
                for (p, panel) in self.settings.providers.iter().zip(self.panels.iter_mut()) {
                    if !p.enabled {
                        continue;
                    }
                    if let Err(m) = p.ready() {
                        panel.fail_to_start(m);
                        continue;
                    }
                    let msgs = panel.begin(&q);
                    jobs.push(Job {
                        p: p.clone(),
                        msgs,
                        run: panel.run,
                    });
                }
                workers = jobs.len();
            }
            Page::OpenRouter => {
                if self.settings.openrouter_key.trim().is_empty() {
                    self.message = "Inserisci prima la chiave OpenRouter qui sopra.".into();
                    return;
                }
                let key = self.settings.openrouter_key.clone();
                let off: HashSet<&String> = self.settings.or_disabled.iter().collect();
                for (m, panel) in self.or_models.iter().zip(self.or_panels.iter_mut()) {
                    if off.contains(m) {
                        continue;
                    }
                    let msgs = panel.queue(&q);
                    jobs.push(Job {
                        p: or_provider(m, &key),
                        msgs,
                        run: panel.run,
                    });
                }
                workers = OR_WORKERS;
            }
        }
        self.message = if jobs.is_empty() {
            "Nessuna IA pronta: controlla le impostazioni.".into()
        } else {
            self.prompt.clear();
            String::new()
        };
        self.run_jobs(ctx, jobs, workers);
    }

    fn cancel_all(&mut self) {
        self.cancel.store(true, Ordering::SeqCst);
        let panels = match self.page {
            Page::All => &mut self.panels,
            Page::OpenRouter => &mut self.or_panels,
        };
        for p in panels {
            p.cancel();
        }
    }

    fn reset_chats(&mut self) {
        if self.any_running() {
            return;
        }
        let panels = match self.page {
            Page::All => &mut self.panels,
            Page::OpenRouter => &mut self.or_panels,
        };
        for p in panels {
            p.reset();
        }
    }

    fn fetch_free_models(&mut self, ctx: &egui::Context) {
        if self.or_loading {
            return;
        }
        self.or_loading = true;
        self.or_note = "Cerco i modelli gratuiti…".into();
        let (tx, ctx, t) = (self.tx.clone(), ctx.clone(), self.settings.timeout_secs);
        std::thread::spawn(move || {
            let _ = tx.send(Msg::FreeModels(api::list_free_models(t)));
            ctx.request_repaint();
        });
    }

    fn fetch_models(&mut self, ctx: &egui::Context, p: &Provider) {
        self.model_note
            .insert(p.id.clone(), "Cerco i modelli…".into());
        let (tx, ctx, p, t) = (
            self.tx.clone(),
            ctx.clone(),
            p.clone(),
            self.settings.timeout_secs,
        );
        std::thread::spawn(move || {
            let res = api::list_models(&p, t);
            let _ = tx.send(Msg::Models { id: p.id, res });
            ctx.request_repaint();
        });
    }

    fn pump(&mut self) {
        while let Ok(m) = self.rx.try_recv() {
            match m {
                Msg::Chat { id, run, ev } => {
                    if let Some(p) = self
                        .panels
                        .iter_mut()
                        .chain(self.or_panels.iter_mut())
                        .find(|p| p.provider_id == id && p.run == run)
                    {
                        p.apply(ev);
                    }
                }
                Msg::FreeModels(res) => {
                    self.or_loading = false;
                    match res {
                        Ok(list) if list.is_empty() => {
                            self.or_note = "OpenRouter non ha dato nessun modello gratuito.".into();
                        }
                        Ok(list) => {
                            self.or_note = format!("{} modelli gratuiti disponibili", list.len());
                            self.or_models = list;
                            self.sync_or_panels();
                        }
                        Err(e) => self.or_note = e,
                    }
                }
                Msg::Models { id, res } => match res {
                    Ok(list) if list.is_empty() => {
                        self.model_note.insert(
                            id,
                            "Il servizio non ha dato un elenco: scrivi il nome a mano.".into(),
                        );
                    }
                    Ok(list) => {
                        self.model_note
                            .insert(id.clone(), format!("{} modelli trovati", list.len()));
                        self.models.insert(id, list);
                    }
                    Err(e) => {
                        self.model_note.insert(id, e);
                    }
                },
            }
        }
    }

    fn export(&mut self) {
        let rows: Vec<(String, String, &Panel)> = match self.page {
            Page::All => self
                .settings
                .providers
                .iter()
                .zip(&self.panels)
                .filter(|(p, _)| p.enabled)
                .map(|(p, panel)| (p.name.clone(), p.model.clone(), panel))
                .collect(),
            Page::OpenRouter => self
                .or_models
                .iter()
                .zip(&self.or_panels)
                .filter(|(m, _)| self.or_enabled(m))
                .map(|(m, panel)| (m.clone(), "OpenRouter".to_string(), panel))
                .collect(),
        };
        let when = chrono::Local::now().format("%d/%m/%Y %H:%M").to_string();
        let md = export_markdown(&rows, &when);
        let Some(path) = rfd::FileDialog::new()
            .set_file_name("MultiIA-risposte.md")
            .add_filter("Markdown", &["md"])
            .save_file()
        else {
            return;
        };
        self.message = match std::fs::write(&path, md) {
            Ok(()) => format!("Salvato in {}", path.display()),
            Err(e) => format!("Non riesco a salvare: {e}"),
        };
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new(format!("{APP_TITLE} di Lorenzo D'Apolito"))
                    .size(24.0)
                    .strong()
                    .color(theme::PRIMARY),
            );
            ui.label(RichText::new(format!("v{APP_VERSION}")).color(theme::MUTED));
        });
        ui.horizontal_wrapped(|ui| {
            for (page, label, color) in [
                (Page::All, "Tutte le IA", theme::BTN_BLUE),
                (
                    Page::OpenRouter,
                    "OpenRouter (modelli gratuiti)",
                    theme::BTN_LILAC,
                ),
            ] {
                let sel = self.page == page;
                let text = RichText::new(label)
                    .color(theme::ON_COLOR)
                    .strong()
                    .size(if sel { 16.0 } else { 14.0 });
                let fill = if sel { color } else { theme::PRIMARY_LIGHT };
                if ui.add(egui::Button::new(text).fill(fill)).clicked() && !sel {
                    self.page = page;
                    if page == Page::OpenRouter && self.or_models.is_empty() {
                        self.fetch_free_models(&ctx);
                    }
                }
            }
            ui.separator();
            if self.any_running() && color_button(ui, "Annulla", theme::BTN_PINK).clicked() {
                self.cancel_all();
            }
            if color_button(ui, "Nuova chat", theme::BTN_MINT).clicked() {
                self.reset_chats();
            }
            if color_button(ui, "Esporta risposte", theme::BTN_YELLOW).clicked() {
                self.export();
            }
            if color_button(ui, "Impostazioni", theme::BTN_BLUE).clicked() {
                self.show_settings = !self.show_settings;
            }
        });
        if !self.message.is_empty() {
            ui.label(RichText::new(&self.message).color(theme::MUTED));
        }
    }

    fn prompt_bar(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let running = self.any_running();
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let w = (ui.available_width() - 150.0).max(200.0);
            let edit = egui::TextEdit::multiline(&mut self.prompt)
                .desired_rows(3)
                .desired_width(w)
                .hint_text("Scrivi la domanda (Ctrl+Invio per inviare a tutte)");
            ui.add(edit);
            ui.vertical(|ui| {
                let enter =
                    ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter));
                let label = if running {
                    "In corso…"
                } else {
                    "Invia a tutte"
                };
                let b = ui.add_enabled(
                    !running,
                    egui::Button::new(RichText::new(label).color(theme::ON_COLOR).strong())
                        .fill(theme::BTN_MINT),
                );
                if (b.clicked() || enter) && !running {
                    self.send(&ctx);
                }
            });
        });
        ui.add_space(6.0);
    }

    fn columns(&mut self, ui: &mut egui::Ui) {
        let items: Vec<(usize, &Provider, &Panel)> = self
            .settings
            .providers
            .iter()
            .zip(&self.panels)
            .enumerate()
            .filter(|(_, (p, _))| p.enabled)
            .map(|(i, (p, panel))| (i, p, panel))
            .collect();
        if items.is_empty() {
            ui.label("Nessuna IA attiva: aggiungine una in Impostazioni.");
            return;
        }
        let mut copy: Option<String> = None;
        ui.columns(items.len(), |cols| {
            for (col, (i, p, panel)) in cols.iter_mut().zip(&items) {
                let color = COLORS
                    .get(i % COLORS.len())
                    .copied()
                    .unwrap_or(theme::BTN_BLUE);
                if let Some(t) = column_ui(col, &p.name, &p.model, color, panel, &p.id) {
                    copy = Some(t);
                }
            }
        });
        if let Some(t) = copy {
            ui.ctx().copy_text(t);
            self.message = "Risposta copiata.".into();
        }
    }

    fn openrouter_page(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let mut save = false;
        ui.horizontal_wrapped(|ui| {
            ui.label("Chiave OpenRouter");
            let r = ui.add(
                egui::TextEdit::singleline(&mut self.settings.openrouter_key)
                    .password(true)
                    .desired_width(320.0),
            );
            if r.lost_focus() {
                save = true;
            }
            if color_button(ui, "Aggiorna elenco", theme::BTN_MINT).clicked() {
                self.fetch_free_models(&ctx);
            }
            ui.label(RichText::new(&self.or_note).color(theme::MUTED));
        });
        ui.label(
            RichText::new(
                "Solo modelli gratuiti, una chat per ciascuno. La chiave si crea su openrouter.ai/settings/keys. I gratuiti hanno limiti di richieste: ne partono 4 alla volta; spegni quelli che non ti servono.",
            )
            .color(theme::MUTED),
        );
        let n_on = self.or_models.iter().filter(|m| self.or_enabled(m)).count();
        let mut toggled = false;
        egui::CollapsingHeader::new(format!(
            "Modelli attivi ({n_on} su {})",
            self.or_models.len()
        ))
        .id_salt("or_toggle")
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                if ui.small_button("Tutti").clicked() {
                    self.settings.or_disabled.clear();
                    toggled = true;
                }
                if ui.small_button("Nessuno").clicked() {
                    self.settings.or_disabled = self.or_models.clone();
                    toggled = true;
                }
            });
            egui::ScrollArea::vertical()
                .max_height(160.0)
                .show(ui, |ui| {
                    for m in self.or_models.clone() {
                        let mut on = self.or_enabled(&m);
                        if ui.checkbox(&mut on, &m).changed() {
                            self.settings.or_disabled.retain(|d| d != &m);
                            if !on {
                                self.settings.or_disabled.push(m);
                            }
                            toggled = true;
                        }
                    }
                });
        });
        if save || toggled {
            self.save_settings();
        }
        ui.separator();
        let shown: Vec<usize> = (0..self.or_models.len())
            .filter(|i| self.or_models.get(*i).is_some_and(|m| self.or_enabled(m)))
            .collect();
        if shown.is_empty() {
            ui.label(
                "Nessun modello da mostrare: premi «Aggiorna elenco» o attiva qualche modello.",
            );
            return;
        }
        let mut copy: Option<String> = None;
        let h = ui.available_height();
        egui::ScrollArea::horizontal()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.horizontal_top(|ui| {
                    for (n, i) in shown.iter().enumerate() {
                        let (Some(m), Some(panel)) =
                            (self.or_models.get(*i), self.or_panels.get(*i))
                        else {
                            continue;
                        };
                        let color = COLORS
                            .get(n % COLORS.len())
                            .copied()
                            .unwrap_or(theme::BTN_BLUE);
                        ui.allocate_ui_with_layout(
                            egui::vec2(OR_COLUMN_WIDTH, h - 14.0),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                if let Some(t) = column_ui(ui, m, "", color, panel, m) {
                                    copy = Some(t);
                                }
                            },
                        );
                    }
                });
            });
        if let Some(t) = copy {
            ui.ctx().copy_text(t);
            self.message = "Risposta copiata.".into();
        }
    }

    fn settings_window(&mut self, ctx: &egui::Context) {
        let mut open = true;
        let mut delete: Option<usize> = None;
        let mut fetch: Option<Provider> = None;
        let mut add: Option<Provider> = None;
        egui::Window::new("Impostazioni").open(&mut open).default_width(560.0).show(ctx, |ui| {
            ui.label(RichText::new("Le chiavi restano solo su questo PC, nel file delle impostazioni di MultiIA.").color(theme::MUTED));
            egui::ScrollArea::vertical().max_height(420.0).show(ui, |ui| {
                for (i, p) in self.settings.providers.iter_mut().enumerate() {
                    egui::Frame::new().fill(theme::STRIPE).corner_radius(10).inner_margin(8).show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut p.enabled, "");
                            ui.add(egui::TextEdit::singleline(&mut p.name).desired_width(180.0));
                            if ui.small_button("Elimina").clicked() {
                                delete = Some(i);
                            }
                        });
                        egui::Grid::new(("g", &p.id)).num_columns(2).show(ui, |ui| {
                            ui.label("Indirizzo");
                            ui.add(egui::TextEdit::singleline(&mut p.base_url).desired_width(380.0));
                            ui.end_row();
                            ui.label("Modello");
                            ui.horizontal(|ui| {
                                ui.add(egui::TextEdit::singleline(&mut p.model).desired_width(220.0));
                                if let Some(list) = self.models.get(&p.id) {
                                    egui::ComboBox::from_id_salt(("m", &p.id)).selected_text("scegli").show_ui(ui, |ui| {
                                        for m in list {
                                            ui.selectable_value(&mut p.model, m.clone(), m);
                                        }
                                    });
                                }
                                if ui.small_button("Cerca modelli").clicked() {
                                    fetch = Some(p.clone());
                                }
                            });
                            ui.end_row();
                            ui.label("Chiave API");
                            ui.add(egui::TextEdit::singleline(&mut p.api_key).password(true).desired_width(380.0));
                            ui.end_row();
                        });
                        if let Some(n) = self.model_note.get(&p.id) {
                            ui.label(RichText::new(n).color(theme::MUTED));
                        }
                    });
                    ui.add_space(4.0);
                }
            });
            ui.menu_button("Aggiungi IA", |ui| {
                for pr in config::presets() {
                    if ui.button(&pr.name).clicked() {
                        add = Some(Provider::new(&pr.name, &pr.base_url, &pr.model));
                        ui.close();
                    }
                }
                if ui.button("Altra (compatibile OpenAI)").clicked() {
                    add = Some(Provider { id: config::new_id(), ..Provider::default() });
                    ui.close();
                }
            });
            ui.separator();
            ui.label("Istruzioni per tutte le IA (facoltative)");
            ui.add(egui::TextEdit::multiline(&mut self.settings.system_prompt).desired_rows(2).desired_width(f32::INFINITY));
            ui.add(egui::Slider::new(&mut self.settings.timeout_secs, 10..=600).text("Attesa massima (s)"));
            ui.add(egui::Slider::new(&mut self.settings.max_tokens, 0..=32000).text("Lunghezza massima risposta (token, 0 = nessun limite)"));
        });
        if let Some(i) = delete {
            if i < self.settings.providers.len() && !self.any_running() {
                self.settings.providers.remove(i);
            }
        }
        if let Some(p) = add {
            if self.settings.providers.len() < config::MAX_PROVIDERS {
                self.settings.providers.push(p);
            }
        }
        if let Some(p) = fetch {
            self.fetch_models(ctx, &p);
        }
        self.sync_panels();
        if !open {
            self.show_settings = false;
            self.save_settings();
            self.message = "Impostazioni salvate.".into();
        }
    }
}

impl eframe::App for App {
    fn clear_color(&self, _v: &egui::Visuals) -> [f32; 4] {
        theme::BG.to_normalized_gamma_f32()
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.pump();
        if self.any_running() {
            ctx.request_repaint_after(std::time::Duration::from_millis(200));
        }
        egui::Panel::top("barra").show(ui, |ui| {
            ui.add_space(6.0);
            self.top_bar(ui);
            ui.add_space(4.0);
        });
        egui::Panel::bottom("domanda").show(ui, |ui| self.prompt_bar(ui));
        egui::CentralPanel::default().show(ui, |ui| match self.page {
            Page::All => self.columns(ui),
            Page::OpenRouter => self.openrouter_page(ui),
        });
        if self.show_settings {
            self.settings_window(&ctx);
        }
    }
}
