# Super Popaul 🍿

Résolution Peppol en masse : un CSV d'adressages en entrée, un CSV enrichi en
sortie (existe dans Peppol, code PA, pays PA, support EXTENDED-CTC-FR).
Le repo contient tout l'écosystème — le serveur d'API et ses clients :

```
superpopaul/
├── client/    # app graphique Tauri 2 (Windows + macOS, Rust + vanilla JS)
├── server/    # peppol_api.py — API REST (résolveur SML+SMP derrière une clé)
├── cli/       # popaul.py / popaul.ps1 — clients batch en ligne de commande
└── docs/      # specs, plans, capture Swagger
```

Historiquement développé dans le monorepo privé peppolstat ; les copies du
serveur et des clients CLI qui y restent divergent librement. La parité de
canonicalisation `cli/popaul.py` ↔ `client/…/pid.rs` est maintenue **ici**
par tests miroir.

## `client/` — application graphique

- **Wizard 3 étapes** : fichier d'entrée → colonnes de sortie → run. La sortie
  (répertoire + suffixe), l'API et le proxy se règlent dans le panneau ⚙.
- **Réglages auto-persistés** (`superpopaul.yaml`, dossier données utilisateur) :
  lus au démarrage, écrits à la fermeture du panneau ⚙. La clé API y est
  stockée ; les identifiants proxy **jamais**.
- **Cache SQLite global** (dossier données utilisateur) : chaque adressage unique
  est résolu une fois ; modes **full / reprise / refresh** (seuil d'ancienneté).
- **Profils de chargement YAML** sauvegardés/chargés explicitement : fichier
  d'entrée (chemin relatif au YAML), colonne des adressages, colonnes de
  sortie. Ni clé API ni réglages ; les anciennes configs complètes restent
  chargeables (seul le profil en est repris).
- **Cockpit temps réel** : ring de progression + ETA, % Peppol, % CTC-FR,
  débits (req/s et adressages/s), codes HTTP, latences p50/p90/p99.
- **Pause/reprise** à chaud et entre sessions (détection de run incomplet).
- Erreurs intelligentes : 401 → suspension + ressaisie de clé ; 429 → backoff
  adaptatif (AIMD) ; 5xx en rafale → circuit breaker avec re-test automatique.

```bash
cd client/src-tauri
cargo test          # logique métier (aucune UI requise)
cargo tauri dev     # app en mode dev
cargo tauri build   # binaire de distribution
```

**Distribution** : binaires **non signés** — la procédure d'ouverture
(Gatekeeper macOS, SmartScreen Windows) est détaillée dans
`NOTICE-OUVERTURE.md`. macOS : build local. Windows : GitHub Actions
(`.github/workflows/windows.yml`, déclenché par les tags `v*`).

## `server/` — `peppol_api.py`, l'API REST

Expose le résolveur SML → SMP → cert X.509 (`peppol_resolver.py`, aucune API
tierce) derrière une petite API HTTP protégée par **clé d'API**. Serveur
`http.server` threadé, sans framework (dépendances : `dnspython`,
`cryptography`).

```bash
cd server
pip install -r requirements.txt

python peppol_api.py --gen-key                    # 1. générer une clé
python peppol_api.py --keys "MA_CLE" --port 8080  # 2. lancer (127.0.0.1:8080)

curl http://127.0.0.1:8080/health                              # public
curl -H "X-API-Key: MA_CLE" \
     http://127.0.0.1:8080/resolve/0225:000122308              # réponse simple
curl -X POST -H "X-API-Key: MA_CLE" -H "Content-Type: application/json" \
     -d '{"participants":["0225:000122308","0225:931153688"]}' \
     http://127.0.0.1:8080/resolve/batch                       # batch (≤ 50)
```

| Endpoint | Auth | Rôle |
|---|---|---|
| `GET /health` | non | liveness (healthcheck reverse-proxy / monitoring) |
| `GET /openapi.json` | non | spec OpenAPI 3.0 |
| `GET /docs` | non | **Swagger UI** interactive |
| `GET /resolve/{participant}` | clé | réponse simple (`?test=1`, `?detail=full`) |
| `GET /resolve?participant={id}` | clé | idem, PID en query |
| `POST /resolve/batch` | clé | résout une liste (≤ 50), erreurs isolées par item |

![Swagger UI de peppol_api.py](docs/swagger.png)

<sub>Capture régénérable après tout changement d'API :
`bash docs/make_swagger_png.sh` (rendu hors-ligne via Chromium, sans CDN).</sub>

**Sécurité / robustesse** : plusieurs clés nommées et révocables (comparaison
en temps constant), **rate-limiting** token-bucket par clé avec débit propre
(`label=CLE [rate] [burst]` dans `--keys`/`--keys-file`), **concurrence
bornée** (`--max-concurrency`). Config par CLI ou env (`PEPPOL_API_HOST`,
`PEPPOL_API_PORT`, `PEPPOL_API_KEYS[_FILE]`, `PEPPOL_API_RATE_LIMIT`,
`PEPPOL_API_RATE_BURST`, `PEPPOL_API_MAX_CONCURRENCY`) ; passe-plats réseau du
résolveur : `--proxy`, `--ca-bundle`, `--insecure`, `--dns-server`, `--doh`.

Le serveur embarque aussi deux résolveurs unitaires de debug :
`peppol_resolver.py` (pipeline complet en direct, flags `--full`, `--ap-only`,
`--debug`, `--test`) et `peppol_resolver_rest.py` (même sortie via l'API REST
publique de Helger, stdlib only — pratique derrière un proxy qui bloque
UDP/53, mais rate-limitée : inspections ponctuelles seulement).

## `cli/` — `popaul.py` / `popaul.ps1`, clients batch

Client léger (stdlib pure) qui résout une liste d'adressages par fournées via
`POST /resolve/batch` et écrit un CSV. Découpe en paquets ≤ 50, gère la clé
d'API (`--key` ou `PEPPOL_API_KEY`), les **429** (retry + backoff,
`Retry-After` respecté), `--resume`, et une barre de progression.

```bash
# Un PID par ligne -> CSV sur stdout
python cli/popaul.py adressages.txt --url https://api.example.com --key MA_CLE

# Colonne d'un CSV -> fichier de sortie
python cli/popaul.py entreprises.csv --column pid -o resultats.csv \
       --url https://api.example.com --key MA_CLE
```

Version **Windows / PowerShell** équivalente : `cli/popaul.ps1` (mêmes
fonctions, `Write-Progress`, `-Resume`) — compatible PowerShell 5.1 et 7+.

## Tests

```bash
(cd client/src-tauri && cargo test)                     # 126 tests Rust
(cd server && python3 -m unittest discover -s tests)    # API + résolveur
(cd cli    && python3 -m unittest discover -s tests)    # canonicalisation (miroir de pid.rs)
```

## Spec & plan

- Spec : [`docs/specs/2026-07-12-super-popaul-design.md`](docs/specs/2026-07-12-super-popaul-design.md)
- Plan : [`docs/plans/2026-07-12-super-popaul.md`](docs/plans/2026-07-12-super-popaul.md)
