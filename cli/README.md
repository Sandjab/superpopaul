# `cli/` — `popaul.py` / `popaul.ps1` 🍿, clients batch

Clients **en ligne de commande** de l'API `server/peppol_api.py` : ils lisent
une liste de Participant IDs (fichier texte, un par ligne, ou colonne d'un
CSV), l'envoient par fournées à `POST /resolve/batch`, et écrivent un CSV :
existe ? / code PA / nom PA / pays / support EXTENDED-CTC-FR.

## `popaul.py` — Python, stdlib pure

Aucune dépendance. Gère la clé d'API (`--key` ou env `PEPPOL_API_KEY`), les
**429** (retry + backoff, `Retry-After` respecté), la reprise (`--resume`), une
**barre de progression** sur le terminal et un récap final.

```bash
# Un PID par ligne -> CSV sur stdout
python popaul.py adressages.txt --url https://api.example.com --key MA_CLE

# Colonne 'pid' d'un CSV -> fichier de sortie
python popaul.py entreprises.csv --column pid -o resultats.csv \
       --url https://api.example.com --key MA_CLE

# Depuis stdin, SML de test
cat pids.txt | python popaul.py - --url http://127.0.0.1:8080 --key MA_CLE --test
```

Colonnes du CSV de sortie : `participant, exists, pa_code, pa_name,
pa_country, supports_extended_ctc_fr, note`.

| Option | Rôle |
|---|---|
| `input` | fichier de PID (un par ligne), CSV avec `--column`, ou `-` pour stdin |
| `--url` | base URL de l'API (obligatoire) |
| `--key` | clé d'API (sinon env `PEPPOL_API_KEY`) |
| `-o, --output` | CSV de sortie (défaut : stdout) |
| `--column` | nom d'en-tête (ou index 0-based) de la colonne à lire dans un CSV |
| `--batch-size` | taille des fournées (défaut 50, max 500 = limite serveur) |
| `--test` | interroge le SML de test (SMK) |
| `--resume` | reprend un CSV existant en sautant les PID déjà résolus |
| `--timeout` | timeout par requête, en secondes (défaut 60) |
| `--max-retries` | retries sur 429/5xx/réseau (défaut 4) |

**Pourquoi 50 par défaut alors que le serveur accepte 500 ?** Un batch coûte un
jeton de rate-limit **par adressage** : grouper davantage n'accélère rien, mais
un paquet de 500 tient la requête ouverte assez longtemps pour frôler le
timeout. Monter via `--batch-size` au besoin.

### Canonicalisation des adressages

`popaul.py::canonical` normalise chaque entrée comme l'API la renvoie :

- `scheme::icd:x` : déjà canonique, inchangé ;
- `icd:x` : scheme par défaut (`iso6523-actorid-upis`) ajouté ;
- adressage brut sans `:` (SIREN, SIREN_SIRET, SIREN_SIRET_CODEROUTAGE,
  SIREN_SUFFIXELIBRE) : préfixé de l'ICD français `0225` — sans lui, le hash
  SML porterait sur la valeur nue et tout ressortirait « absent de Peppol ».

Cette fonction est maintenue en **parité stricte** avec le client graphique
(`client/src-tauri/src/pid.rs::canonical`) par tests miroir
(`tests/test_popaul.py` ↔ `pid::tests`) : toute évolution d'un côté est
reportée de l'autre.

## `popaul.ps1` — Windows / PowerShell

Version équivalente pour Windows, **compatible PowerShell 5.1 et 7+**, sans
dépendance : mêmes fonctions, `Write-Progress`, reprise avec `-Resume`.
Fournées bornées à 50 (`-BatchSize`).

```powershell
.\popaul.ps1 adressages.txt -Url https://api.example.com -Key MA_CLE -Output resultats.csv
.\popaul.ps1 entreprises.csv -Column pid -Url https://api.example.com -Key MA_CLE -Output out.csv -Resume
```

Paramètres : `-Path` (positionnel, ou `-` pour stdin), `-Url`, `-Key` (sinon
`$env:PEPPOL_API_KEY`), `-Output`, `-Column`, `-BatchSize`, `-Test`, `-Resume`,
`-TimeoutSec`, `-MaxRetries`.

## Tests

```bash
python3 -m unittest discover -s tests    # stdlib pure, aucune installation
```

Ils couvrent notamment la canonicalisation (miroir de `pid.rs`).
