# MultiIA di Lorenzo D'Apolito

Una domanda, più IA insieme: ogni IA risponde in una colonna, in streaming, e puoi confrontarle fianco a fianco. Scritto in Rust con egui.

- **Pronte**: DeepSeek, Grok (xAI), GLM (Z.ai). Si aggiungono con un clic ChatGPT, Mistral, OpenRouter, Ollama (sul tuo PC) o qualunque servizio compatibile OpenAI.
- Risposte in tempo reale, con tempo impiegato, token usati e «ragionamento» dei modelli che lo mostrano.
- Conversazione continua: ogni IA ricorda i messaggi precedenti. «Nuova chat» riparte da zero.
- **Annulla** ferma tutte le IA tenendo ciò che è già arrivato; un errore su una IA non blocca le altre.
- «Cerca modelli» legge l'elenco dei modelli dal servizio; «Esporta risposte» salva tutto in Markdown.
- Colori solo pastello, mai bianco né grigio.

## Chiavi API

Servono le tue chiavi (si creano nei siti dei servizi). Si scrivono in *Impostazioni* e restano solo sul tuo PC, in `%LOCALAPPDATA%\MultiIA\impostazioni.json` (in chiaro: non condividere quel file). I modelli locali (Ollama) non hanno bisogno di chiave.

## Costruire

`costruisci-multiia.cmd` (nella cartella di AmbienteRust) scarica, compila e crea `dist\MultiIA-portable`.
