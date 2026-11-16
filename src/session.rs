//! Stato di una colonna (una IA): conversazione, risposta in arrivo, tempo e consumo.

use crate::api::{Event, Message};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Idle,
    /// In attesa del suo turno (si parte pochi modelli alla volta).
    Queued,
    Running,
    Done,
    Failed(String),
}

#[derive(Clone, Debug)]
pub struct Panel {
    pub provider_id: String,
    pub history: Vec<Message>,
    pub text: String,
    pub reasoning: String,
    pub status: Status,
    pub usage: Option<(u64, u64)>,
    /// Avviso breve mostrato accanto allo stato (per esempio «riprovo»).
    pub note: Option<String>,
    /// Numero dell'invio in corso: gli eventi di invii vecchi si ignorano.
    pub run: u64,
    started: Option<Instant>,
    elapsed: Duration,
}

impl Panel {
    pub fn new(provider_id: &str) -> Self {
        Self {
            provider_id: provider_id.into(),
            history: Vec::new(),
            text: String::new(),
            reasoning: String::new(),
            status: Status::Idle,
            usage: None,
            note: None,
            run: 0,
            started: None,
            elapsed: Duration::ZERO,
        }
    }

    /// In coda o in corso: il pannello è occupato.
    pub fn is_running(&self) -> bool {
        matches!(self.status, Status::Queued | Status::Running)
    }

    /// Comincia un nuovo invio e restituisce la conversazione da mandare.
    pub fn begin(&mut self, question: &str) -> Vec<Message> {
        self.history.push(Message::new("user", question));
        self.text.clear();
        self.reasoning.clear();
        self.usage = None;
        self.note = None;
        self.status = Status::Running;
        self.run += 1;
        self.started = Some(Instant::now());
        self.elapsed = Duration::ZERO;
        self.history.clone()
    }

    /// Come `begin`, ma il pannello aspetta il suo turno finché non arriva `Event::Started`.
    pub fn queue(&mut self, question: &str) -> Vec<Message> {
        let msgs = self.begin(question);
        self.status = Status::Queued;
        self.started = None;
        msgs
    }

    /// Segna l'invio come non partito (per esempio manca la chiave).
    pub fn fail_to_start(&mut self, msg: &str) {
        self.status = Status::Failed(msg.into());
    }

    pub fn elapsed(&self) -> Duration {
        match (self.status == Status::Running, self.started) {
            (true, Some(s)) => s.elapsed(),
            _ => self.elapsed,
        }
    }

    fn finish(&mut self) {
        if let Some(s) = self.started.take() {
            self.elapsed = s.elapsed();
        }
    }

    fn drop_pending_question(&mut self) {
        if self.history.last().is_some_and(|m| m.role == "user") {
            self.history.pop();
        }
    }

    pub fn apply(&mut self, ev: Event) {
        if self.status == Status::Queued {
            if ev == Event::Started {
                self.status = Status::Running;
                self.started = Some(Instant::now());
            }
            return;
        }
        if self.status != Status::Running {
            return;
        }
        match ev {
            Event::Started => {}
            Event::Retry { attempt, max } => {
                self.note = Some(format!("servizio occupato, riprovo ({attempt}/{max})"));
            }
            Event::Text(t) => {
                self.note = None;
                self.text.push_str(&t);
            }
            Event::Reasoning(t) => self.reasoning.push_str(&t),
            Event::Usage { prompt, completion } => self.usage = Some((prompt, completion)),
            Event::Done => {
                self.note = None;
                self.finish();
                if self.text.trim().is_empty() {
                    self.drop_pending_question();
                    self.status = Status::Failed("Risposta vuota".into());
                } else {
                    self.history.push(Message::new("assistant", &self.text));
                    self.status = Status::Done;
                }
            }
            Event::Error(e) => {
                self.note = None;
                self.finish();
                self.drop_pending_question();
                self.status = Status::Failed(e);
            }
        }
    }

    /// L'utente ha premuto Annulla: si tiene ciò che è già arrivato.
    pub fn cancel(&mut self) {
        if !self.is_running() {
            return;
        }
        if self.text.trim().is_empty() {
            self.finish();
            self.drop_pending_question();
            self.status = Status::Failed("Annullato".into());
        } else {
            self.apply(Event::Done);
        }
        self.run += 1;
    }

    /// Cancella la conversazione (nuova chat).
    pub fn reset(&mut self) {
        let run = self.run + 1;
        *self = Self::new(&self.provider_id.clone());
        self.run = run;
    }
}

/// Tutte le conversazioni in un file Markdown, una sezione per IA.
pub fn export_markdown(panels: &[(String, String, &Panel)], when: &str) -> String {
    let mut o = format!("# MultiIA - {when}\n\n");
    for (name, model, p) in panels {
        o.push_str(&format!("## {name} ({model})\n\n"));
        let mut msgs = p.history.clone();
        if p.is_running() && !p.text.is_empty() {
            msgs.push(Message::new("assistant", &p.text));
        }
        if msgs.is_empty() {
            o.push_str("_Nessuna risposta._\n\n");
        }
        for m in &msgs {
            let who = if m.role == "user" {
                "Domanda"
            } else {
                "Risposta"
            };
            o.push_str(&format!("**{who}:**\n\n{}\n\n", m.content.trim()));
        }
        if let Status::Failed(e) = &p.status {
            o.push_str(&format!("_Errore: {e}_\n\n"));
        }
    }
    o
}

/// «1,2 s» oppure «1 min 05 s».
pub fn duration_text(d: Duration) -> String {
    let s = d.as_secs_f64();
    if s < 60.0 {
        format!("{s:.1} s").replace('.', ",")
    } else {
        format!("{} min {:02} s", d.as_secs() / 60, d.as_secs() % 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_full_answer_is_added_to_the_history() {
        let mut p = Panel::new("a");
        let sent = p.begin("ciao");
        assert_eq!(sent.len(), 1);
        p.apply(Event::Text("Ci".into()));
        p.apply(Event::Text("ao!".into()));
        p.apply(Event::Reasoning("hmm".into()));
        p.apply(Event::Usage {
            prompt: 3,
            completion: 4,
        });
        p.apply(Event::Done);
        assert_eq!(p.status, Status::Done);
        assert_eq!(p.history.len(), 2);
        assert_eq!(p.history[1].content, "Ciao!");
        assert_eq!((p.reasoning.as_str(), p.usage), ("hmm", Some((3, 4))));
        // La domanda successiva porta con sé tutta la conversazione.
        assert_eq!(p.begin("e poi?").len(), 3);
    }

    #[test]
    fn errors_and_empty_answers_remove_the_pending_question() {
        let mut p = Panel::new("a");
        p.begin("x");
        p.apply(Event::Error("no".into()));
        assert_eq!(p.status, Status::Failed("no".into()));
        assert!(p.history.is_empty());
        p.begin("y");
        p.apply(Event::Done);
        assert_eq!(p.status, Status::Failed("Risposta vuota".into()));
        assert!(p.history.is_empty());
    }

    #[test]
    fn retry_notice_is_shown_then_cleared_by_text() {
        let mut p = Panel::new("a");
        p.begin("q");
        p.apply(Event::Retry { attempt: 1, max: 2 });
        assert!(p.note.as_deref().is_some_and(|n| n.contains("1/2")));
        p.apply(Event::Text("x".into()));
        assert!(p.note.is_none());
        p.apply(Event::Retry { attempt: 2, max: 2 });
        p.apply(Event::Error("e".into()));
        assert!(p.note.is_none());
    }

    #[test]
    fn queued_panels_wait_for_started_and_can_be_cancelled() {
        let mut p = Panel::new("a");
        assert_eq!(p.queue("q").len(), 1);
        assert_eq!(p.status, Status::Queued);
        assert!(p.is_running());
        p.apply(Event::Text("troppo presto".into()));
        assert!(p.text.is_empty());
        p.apply(Event::Started);
        assert_eq!(p.status, Status::Running);
        p.apply(Event::Text("ok".into()));
        p.apply(Event::Done);
        assert_eq!(p.status, Status::Done);
        let mut q = Panel::new("b");
        q.queue("q");
        q.cancel();
        assert_eq!(q.status, Status::Failed("Annullato".into()));
        assert!(q.history.is_empty());
    }

    #[test]
    fn events_after_the_end_are_ignored() {
        let mut p = Panel::new("a");
        p.apply(Event::Text("x".into()));
        assert!(p.text.is_empty());
        p.begin("q");
        p.apply(Event::Text("r".into()));
        p.apply(Event::Done);
        p.apply(Event::Text("tardivo".into()));
        assert_eq!(p.text, "r");
    }

    #[test]
    fn cancel_keeps_partial_text_and_ignores_late_events() {
        let mut p = Panel::new("a");
        p.begin("q");
        p.apply(Event::Text("meta".into()));
        let run = p.run;
        p.cancel();
        assert_eq!(p.status, Status::Done);
        assert_eq!(p.history.len(), 2);
        assert!(p.run > run);
        let mut q = Panel::new("b");
        q.begin("q");
        q.cancel();
        assert_eq!(q.status, Status::Failed("Annullato".into()));
        assert!(q.history.is_empty());
        q.cancel();
    }

    #[test]
    fn reset_clears_the_chat_but_invalidates_old_runs() {
        let mut p = Panel::new("a");
        p.begin("q");
        let run = p.run;
        p.reset();
        assert!(p.history.is_empty() && p.status == Status::Idle);
        assert!(p.run > run);
        assert_eq!(p.provider_id, "a");
    }

    #[test]
    fn markdown_export_lists_every_provider() {
        let mut a = Panel::new("a");
        a.begin("domanda");
        a.apply(Event::Text("risposta".into()));
        a.apply(Event::Done);
        let mut b = Panel::new("b");
        b.begin("domanda");
        b.apply(Event::Error("chiave".into()));
        let md = export_markdown(
            &[
                ("Uno".into(), "m1".into(), &a),
                ("Due".into(), "m2".into(), &b),
            ],
            "oggi",
        );
        assert!(
            md.contains("## Uno (m1)") && md.contains("risposta") && md.contains("## Due (m2)")
        );
        assert!(md.contains("Nessuna risposta") && md.contains("Errore: chiave"));
    }

    #[test]
    fn durations_read_well() {
        assert_eq!(duration_text(Duration::from_millis(1250)), "1,2 s");
        assert_eq!(duration_text(Duration::from_secs(65)), "1 min 05 s");
    }
}
