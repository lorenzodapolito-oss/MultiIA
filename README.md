# MultiIA di Lorenzo D'Apolito

Una domanda, più IA insieme: ogni IA risponde in una colonna, in streaming, e puoi confrontarle fianco a fianco. Scritto in Rust con egui.

- **Solo IA con piano gratuito per l'API** (serve comunque una chiave, ma gratuita): Gemini, Groq, Mistral, NVIDIA e GitHub Models. Si possono aggiungere anche Ollama e LM Studio (sul tuo PC) o qualunque servizio compatibile OpenAI. I limiti gratuiti e i nomi dei modelli cambiano: «Cerca modelli» in Impostazioni mostra quelli disponibili.
- **Due pagine**: «Tutte le IA» (una colonna per servizio) e «OpenRouter», con una chat separata per **ogni modello gratuito** disponibile (l'elenco si legge da OpenRouter; i modelli a pagamento non compaiono). Si possono spegnere i modelli che non servono; le richieste partono 4 alla volta per rispettare i limiti dei piani gratuiti.
- Risposte in tempo reale, con tempo impiegato, token usati e «ragionamento» dei modelli che lo mostrano.
- Conversazione continua: ogni IA ricorda i messaggi precedenti. «Nuova chat» riparte da zero.
- **Annulla** ferma tutte le IA tenendo ciò che è già arrivato; un errore su una IA non blocca le altre.
- Il **modello più recente che funziona** si sceglie da solo all'avvio (si può spegnere per ogni IA): MultiIA prova i più recenti con una richiesta minuscola e prende il primo che risponde; «Scegli il migliore» lo fa a richiesta. La **chiave API** si scrive anche direttamente nella colonna di ogni IA.
- «Cerca modelli» legge l'elenco dei modelli dal servizio; «Esporta risposte» salva tutto in Markdown.
- Colori solo pastello, mai bianco né grigio.

## Chiavi API

Servono le tue chiavi (si creano nei siti dei servizi). Si scrivono in *Impostazioni* e restano solo sul tuo PC, in `%LOCALAPPDATA%\MultiIA\impostazioni.json` (in chiaro: non condividere quel file). I modelli locali (Ollama) non hanno bisogno di chiave.

## Costruire

`costruisci-multiia.cmd` (nella cartella di AmbienteRust) scarica, compila e crea `dist\MultiIA-portable`.
