# `client/` — Super Popaul, l'application graphique

App **Tauri 2** (Windows + macOS) : backend **Rust** (`src-tauri/`), frontend
**vanilla HTML/CSS/JS** (`src/`) — pas de bundler, pas de framework. Résolution
Peppol en masse sans terminal : un CSV d'adressages en entrée, un CSV enrichi
en sortie, avec cache local, reprise et cockpit temps réel.

```bash
cd src-tauri
cargo test          # logique métier (aucune UI requise)
cargo tauri dev     # app en mode dev
cargo tauri build   # binaire de distribution
```

## Le wizard en 3 étapes

1. **Fichier** — dépôt (drag-drop) ou parcours d'un CSV/TXT. Le backend
   détecte le séparateur (`;` `,` tab `|`) et l'encodage (UTF-8 / windows-1252),
   affiche un aperçu et **suggère la colonne des adressages** (celle dont la
   majorité des valeurs ressemblent à un PID) ; l'utilisateur confirme.
2. **Colonnes** — le tableau d'aperçu *est* l'outil de configuration :
   glisser-déposer des en-têtes pour réordonner, écarter vers la zone de dépôt
   ou réintégrer (double-clic aussi). Colonnes disponibles : toutes les
   colonnes d'entrée + 5 champs Peppol calculés — `in_peppol`, `pa_code`,
   `pa_name`, `pa_country`, `ubl_extended` (CTC-FR).
3. **Run** — analyse fichier ↔ cache (déjà résolus / échecs / périmés /
   manquants) avec présélection du mode, cockpit temps réel, puis écriture du
   CSV enrichi (une ligne de sortie par ligne d'entrée, jointure sur le PID
   canonique).

La sortie (répertoire, suffixe, encodage, séparateur), l'API et le proxy se
règlent dans le panneau **⚙ Réglages** (pas une étape du wizard).

## Deux modes de résolution

- **API** (défaut) : requêtes `POST /resolve/batch` vers `server/peppol_api.py`
  (clé d'API requise).
- **Direct** : résolution **SML + SMP en direct, sans API ni clé**
  (`direct.rs`, parité avec `server/peppol_resolver.py`) — DNS NAPTR sur le
  SML, fetch SMP, parse du certificat X.509. Résolveur DNS configurable :
  système, IP (avec IP de secours en failover), ou **DoH** (RFC 8484, passe
  par le proxy — utile derrière un proxy d'entreprise). Rafale DNS bornée par
  sémaphore : le SML autoritaire fait du Response Rate Limiting, et un
  NXDOMAIN sous rafale serait un **faux « absent de Peppol »** — seul un
  NXDOMAIN authentique vaut `exists=false`, toute erreur transitoire reste une
  erreur d'item. Le réglage « Rafale DNS » (32 lookups simultanés par défaut)
  correspond à ≈ 1 250 req/s, sous le rate-limit des résolveurs publics
  (~1 500 req/s par IP chez Google) ; monter au-delà expose à des timeouts
  sans rien gagner, le débit d'un run étant dominé par les requêtes SMP.

## Persistance

- **Réglages** (`superpopaul.yaml`, dossier données utilisateur) : lus au
  démarrage, écrits à la fermeture du panneau ⚙. URL + clé d'API, mode
  api/direct, résolveur DNS et repli, `batch_size`, concurrence, proxy,
  `refresh_days`, réglages de sortie. La clé API y est stockée ; les
  **identifiants proxy jamais** (`#[serde(skip)]`, garanti par le test
  `config::proxy_creds_never_serialized` ; ils sont ressaisis via une modale).
  Écriture atomique (`.tmp` + rename).
- **Cache SQLite** (`superpopaul.db`, dossier données utilisateur, WAL) :
  table `resolutions` clé = PID canonique — chaque adressage unique est résolu
  une fois puis réutilisé entre fichiers et sessions.
- **Profils de chargement YAML** (boutons Charger…/Sauvegarder…) : fichier
  d'entrée (chemin **relatif au YAML**), colonne des adressages, colonnes de
  sortie. Ni clé API ni réglages. Les anciennes configs complètes restent
  chargeables (seul le profil en est repris).

## Modes de run

Calculés par `modes.rs::compute_todo` à partir du cache :

| Mode | Résout |
|---|---|
| **Full** | tout, en re-résolvant même ce qui est en cache |
| **Reprise** | les adressages absents du cache (+ option : re-tenter les échecs) |
| **Refresh** | absents + échecs + entrées plus vieilles que `refresh_days` |

Un run incomplet est détecté à la réouverture du fichier → reprise entre
sessions. Pendant un run : **pause/reprise** à chaud, garde de fermeture de
fenêtre.

## Cockpit temps réel

Alimenté par l'événement `telemetry` (4×/s) : anneau de progression + ETA,
mini-anneaux % Peppol et % CTC-FR, débits (req/s et adressages/s, fenêtre
glissante 10 s), latences min/p50/p90/p99/max + histogramme, histogramme des
codes HTTP, top PA et top erreurs, temps actif hors pauses. Les compteurs
existent en adressages uniques **et** en équivalent lignes de fichier
(pondérés par multiplicité).

## Erreurs réseau intelligentes

Pilotées par le moteur (`resolver.rs`), typées dans `api.rs` :

- **401/403** → suspension du run + ressaisie de la clé dans l'UI, reprise à
  chaud (`update_api_key`).
- **407** (proxy) → suspension + modale d'identifiants, client HTTP reconstruit
  puis reprise.
- **429** → backoff (`Retry-After` respecté) + **concurrence adaptative AIMD** :
  divisée par 2 au 429, +1 après 50 succès consécutifs, bornée au plafond
  configuré.
- **5xx / réseau en rafale** → **circuit breaker** (ouvre après 5 échecs
  consécutifs, backoff 30 s doublé à chaque réouverture, re-test automatique,
  bouton « Réessayer maintenant »).
- **4xx** autre → échec définitif de l'item, tracé en base, sans retry.

## Calibration

Depuis les réglages, un banc d'essai (`calibrate_api`) envoie des salves à
concurrence croissante (1, 2, 4, …) et retient le palier optimal : arrêt au
premier 429 ou quand le gain devient marginal. Consomme du quota d'API ;
annulable ; sans objet en mode direct (SMP distribués).

## Architecture

### Backend Rust (`src-tauri/src/`) — modules étanches, testables sans UI

| Module | Rôle |
|---|---|
| `pid.rs` | canonicalisation des adressages — **parité stricte** avec `cli/popaul.py::canonical` (tests miroir) |
| `config.rs` | réglages, profils, migrations d'alias legacy, écriture atomique |
| `store.rs` | cache SQLite (`rusqlite` bundled, WAL, upsert par lots) |
| `modes.rs` | calcul de la liste à résoudre (full / reprise / refresh) |
| `csv_io.rs` | détection séparateur + encodage, aperçu, lecture streaming, suggestion de colonne |
| `api.rs` | client HTTP : façade commune aux transports API et direct, typage des erreurs |
| `direct.rs` | résolution SML+SMP directe (NAPTR, SMP, X.509), DNS système/IP/DoH |
| `resolver.rs` | moteur de run : workers tokio, AIMD, circuit breaker, suspensions, calibration |
| `telemetry.rs` | agrégation du Snapshot (latences, histogrammes, débits, ETA) |
| `output.rs` | CSV enrichi : BOM, windows-1252, écriture atomique, refus d'écraser l'entrée |
| `commands.rs` | les 19 commandes Tauri + `AppState` |

### Frontend (`src/`)

- `app.js` — état global, wizard, réglages, profils, calibration ; helper
  **`h()`** de construction DOM (**jamais d'innerHTML** avec des données
  dynamiques : un CSV est une entrée non fiable).
- `columns.js` — étape 2 (drag-drop des colonnes).
- `cockpit.js` — étape 3 (rendu télémétrie, contrôle du run).
- `vendor/Sortable.min.js` — SortableJS 1.15.6 (MIT), **seule dépendance
  frontend vendorisée** : le DnD HTML5 est avalé par le handler drag-drop de
  Tauri (requis pour le dépôt de fichier), le mode `forceFallback` de Sortable
  n'émet que des événements pointeur.

Événements Rust → UI : `telemetry`, `calibrate-step`, `run-suspended`,
`run-resumed`, `run-finished`.

## Distribution

Binaires **non signés** — procédure d'ouverture (Gatekeeper macOS, SmartScreen
Windows) : [`NOTICE-OUVERTURE.md`](../NOTICE-OUVERTURE.md) à la racine.
macOS : build local. Windows : GitHub Actions
([`.github/workflows/windows.yml`](../.github/workflows/windows.yml),
déclenché manuellement ou par les tags `v*`, avec contrôle de taille < 20 Mo
et release automatique sur tag).

## Tests & outillage

```bash
cd src-tauri
cargo test                                   # toute la logique métier
cargo test -- --ignored                      # + 3 tests réseau réel (SML prod, DNS, DoH)
cargo run --release --example dns_stress -- hosts.txt 64   # banc DNS NAPTR sous rafale
```

`examples/dns_stress.rs` rejoue les lookups NAPTR à forte concurrence sur une
liste de hostnames SML connus et compte found/nxdomain/failed (+ latences) :
il sert à valider un résolveur DNS contre le rate-limiting du SML.
