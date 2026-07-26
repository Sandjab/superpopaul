# `client/` — Super Popaul, l'application graphique

App **Tauri 2** (Windows + macOS) : backend **Rust** (`src-tauri/`), frontend
**vanilla HTML/CSS/JS** (`src/`) — pas de bundler, pas de framework. Résolution
Peppol en masse sans terminal : un CSV d'adressages en entrée, un CSV enrichi
en sortie, avec cache local, reprise et cockpit temps réel.

Deux fonctions s'y sont greffées, absentes de l'API et de la CLI : le
**croisement avec les annuaires** Peppol et PPF, et le **plan de charge** —
la répartition des comptes de facturation sur un calendrier de Runs de
Facturation.

```bash
cd src-tauri
cargo test          # logique métier (aucune UI requise)
cargo tauri dev     # app en mode dev
cargo tauri build   # binaire de distribution
```

## Le wizard en 3 étapes

1. **Fichiers** — un triptyque : le **fichier principal à enrichir** (dépôt
   drag-drop ou parcours d'un CSV/TXT) et les deux **annuaires de référence**
   (Peppol, PPF), facultatifs et persistants. Pour le fichier principal, le
   backend détecte le séparateur (`;` `,` tab `|`) et l'encodage (UTF-8 /
   windows-1252), affiche un aperçu et **suggère la colonne des adressages**
   (celle dont la majorité des valeurs ressemblent à un PID) ; l'utilisateur
   confirme.
2. **Format** — le tableau d'aperçu *est* l'outil de configuration :
   glisser-déposer des en-têtes pour réordonner, écarter vers la zone de dépôt
   ou réintégrer (double-clic aussi). Colonnes disponibles : toutes les
   colonnes d'entrée + 13 champs calculés —
   présence et plateforme (`in_peppol`, `pa_code`, `pa_name`, `pa_country`),
   extension française (`ubl_extended`, `ctc_activation`, `ctc_expiration`,
   `ctc_status`), annuaires (`in_directory`, `annuaire_ppf`, `ppf_active`,
   `pdp_definie`, `ppf_usable`). Les en-têtes produits restent en snake_case,
   indépendamment des libellés affichés.
3. **Run** — analyse fichier ↔ cache (déjà résolus / à retenter / périmés /
   manquants) avec présélection du mode, **panneau de couverture** par les
   annuaires chargés, cockpit temps réel, puis écriture du CSV enrichi (une
   ligne de sortie par ligne d'entrée, jointure sur le PID canonique) et du
   **rapport HTML** de fin de run (`…_rapport.html`, agrégats seuls — jamais
   de liste d'adressages). C'est aussi d'ici qu'on ouvre le plan de charge.

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

## Annuaires de référence

Deux annuaires facultatifs, chargés depuis l'étape 1 et conservés en base
(fonctionnalités **client-only** : ni l'API ni la CLI ne les connaissent).

- **Annuaire Peppol** (`export-all-participants.csv`) : seuls les adressages
  0225 sont retenus. Alimente `in_directory`.
- **Annuaire PPF** (export B2B du Portail Public de Facturation) : chargement
  **cumulatif** — chaque fichier ajoute ses adresses, un doublon est reconnu
  par hash SHA-256 du contenu et ignoré. Alimente `annuaire_ppf`,
  `ppf_active`, `pdp_definie` et `ppf_usable` ; les motifs tenus pour actifs
  sont configurables (défaut C ou P).

Avant le run, le **panneau de couverture** dit quelle part des lignes du
fichier chaque annuaire couvre — gate indépendant par annuaire : chacun
n'apparaît que s'il est chargé.

## Persistance

Le « dossier données utilisateur » ci-dessous est `app_data_dir` (Tauri) —
sauf en **mode portable** (Windows) : si un marqueur `superpopaul.portable`
ou une base `superpopaul.db` existe à côté de `superpopaul.exe`, tout est lu
et écrit dans le dossier de l'exe (et les dialogues de profils s'y ouvrent).
C'est le mode du zip portable des releases. Jamais d'heuristique
d'inscriptibilité (l'install NSIS per-user vit dans `%LOCALAPPDATA%`,
inscriptible) ; sur macOS, mode installé inconditionnel (bundle signé).

- **Réglages** (`superpopaul.yaml`, dossier données utilisateur) : lus au
  démarrage, écrits à la fermeture du panneau ⚙. URL + clé d'API, mode
  api/direct, résolveur DNS et repli, `batch_size`, concurrence, proxy,
  `refresh_days`, réglages de sortie. La clé API y est stockée ; les
  **identifiants proxy jamais** (`#[serde(skip)]`, garanti par le test
  `config::proxy_creds_never_serialized` ; ils sont ressaisis via une modale).
  Écriture atomique (`.tmp` + rename).
- **Cache SQLite** (`superpopaul.db`, dossier données utilisateur, WAL) :
  table `resolutions` clé = PID canonique — chaque adressage unique est résolu
  une fois puis réutilisé entre fichiers et sessions. Les adressages **0225**
  (SIREN français) y sont stockés **nus**, sans le préfixe
  `iso6523-actorid-upis::0225:`, pour joindre directement les annuaires ; la
  conversion est confinée à la frontière SQL (`store.rs`), le pipeline
  manipule toujours la forme longue, et la migration est idempotente à
  l'ouverture. La même base porte les annuaires chargés (`peppol_directory`,
  `ppf_directory`, `ppf_files`) et le plan de charge enregistré (`plan_cf`,
  `plan_meta`).
- **Profils de chargement YAML** (boutons Charger…/Sauvegarder…) : fichier
  d'entrée (chemin **relatif au YAML**), colonne des adressages, colonnes de
  sortie. Ni clé API ni réglages. Les anciennes configs complètes restent
  chargeables (seul le profil en est repris).

## Modes de run

Calculés par `modes.rs::compute_todo` à partir du cache :

| Mode | Résout |
|---|---|
| **Full** | tout, en re-résolvant même ce qui est en cache |
| **Reprise** | les adressages absents du cache (+ option : re-tenter ce qui est à retenter) |
| **Refresh** | absents + à retenter + entrées plus vieilles que `refresh_days` |

« À retenter » (`modes.rs::a_retenter`) couvre deux états, pas un : l'échec
franc (`api_status` ≠ `ok`) **et** la résolution incomplète — présent dans
Peppol mais sans verdict CTC, seul cas où `exists=true` coexiste avec un
verdict absent. Cet état signe un catalogue SMP illisible (HTTP 503/404,
délai dépassé) et il est persisté `api_status="ok"` : sans ce second motif,
un incident SMP transitoire se figerait en « sans verdict » définitif que
plus aucun mode ne reprendrait. À l'inverse `Some(false)` — présent sans
support CTC, ou absent du réseau — est un verdict complet : le retenter
remartèlerait tout le parc.

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

## Plan de charge

Écran de **plein niveau** (pas une 4ᵉ étape du wizard), ouvert depuis l'étape
Run : il travaille sur les résolutions déjà en base, sans relancer de run.
Deux onglets — *Paramétrage* et *Comptes de facturation*.

- **Entrées** : un calendrier de **Runs de Facturation** (`runs.csv`) et le
  fichier principal, dont les colonnes compte de facturation (CF), jour de
  cycle (JJ) et raison sociale sont désignées une fois puis mémorisées par
  structure de fichier.
- **Pool éligible** : les comptes réellement basculables — statut CTC prêt
  **et** `ppf_usable`.
- **Allocation** : répartition sur les runs aux plus forts restes, quotas par
  plateforme, rampe de montée automatique ou reprise en main run par run,
  respect des jours de cycle de facturation.
- **Timeline calendaire** : chaque run replacé sur les jours civils —
  week-ends, onze jours fériés français calculés (computus de Meeus), jalons
  de mise en production, et le motif d'écart quand un run n'est pas retenu.
- **Retouche manuelle** : ajout de comptes à un run (fenêtre triable et
  filtrable), déplacement, retrait — une régénération préserve les décisions
  prises.
- **Livrables** : un fichier de comptes par mise en production
  (`…_plan_mep_<n>_<date>.txt`), le **rapport HTML** du plan (`…_plan.html`,
  livrable distinct du rapport de run) et le **classeur XLSX** du périmètre
  (`…_plan_comptes.xlsx` — l'union des comptes du fichier d'entrée et de ceux
  du plan que le fichier ne contient plus).

Le plan est persisté (`plan_cf`, `plan_meta`) : on le rouvre tel quel.

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
| `ctc.rs` | fenêtre temporelle du support CTC : dates stockées, **état calculé** (prêt / plus tard / expiré) |
| `directory.rs` | ingestion de l'annuaire Peppol (0225 seuls, stockés nus) |
| `ppf.rs` | ingestion cumulative de l'annuaire PPF, doublon par hash de contenu |
| `coverage.rs` | couverture du fichier par les annuaires chargés — agrégat pur |
| `securisation.rs` | croisement résolution × PPF `usable` × annuaire Peppol, par ligne — agrégat pur |
| `repartition.rs` | répartition des lignes par plateforme (Point d'Accès Peppol) — agrégat pur |
| `calendrier.rs` | Runs de Facturation et mises en production : parsing `runs.csv`, runs utilisables, rattachement |
| `plan.rs` | plan de charge : pool éligible, quotas, rampe, allocation aux runs — agrégat pur |
| `charge.rs` | charge par run : premières factures et récurrences mensuelles — module pur |
| `timeline.rs` | assemblage de la timeline calendaire (jours civils, fériés, jalons, motifs d'écart) — module pur |
| `charts.rs` | graphes SVG partagés (aire cumulée, barres empilées) — module pur |
| `report.rs` | rapport HTML de fin de run : agrégats seuls, zéro JavaScript |
| `plan_report.rs` | rapport HTML du plan, livrable distinct (style partagé avec `report.rs`) |
| `plan_xlsx.rs` | classeur XLSX du périmètre du plan (`rust_xlsxwriter`) |
| `commands.rs` | les 40 commandes Tauri + `AppState` |

### Frontend (`src/`)

- `app.js` — état global, wizard, annuaires, réglages, profils, calibration et
  tout l'écran Plan de charge ; helper **`h()`** de construction DOM
  (**jamais d'innerHTML** avec des données dynamiques : un CSV est une entrée
  non fiable).
- `columns.js` — étape 2 (drag-drop des colonnes).
- `cockpit.js` — étape 3 (rendu télémétrie, contrôle du run, panneau de
  couverture) et l'ouverture du plan.
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
cargo test                                   # 523 tests — toute la logique métier
cargo test -- --ignored                      # + 3 tests réseau réel (SML prod, DNS, DoH)
cargo run --release --example dns_stress -- hosts.txt 64   # banc DNS NAPTR sous rafale

cd ..                                        # depuis client/
node --test "tests/*.test.js"                # 38 tests JS — câblage UI (stdlib Node)
```

`tests/dom_shim.js` fournit un faux DOM qui exécute le **vrai** `src/app.js` :
réservé au câblage (un champ reconstruit qui perd sa valeur, un écouteur qui
ne se rebranche pas). Il prouve qu'un nœud existe, **jamais qu'il est
visible** — tout ce qui touche au rendu se vérifie dans l'application.

`examples/dns_stress.rs` rejoue les lookups NAPTR à forte concurrence sur une
liste de hostnames SML connus et compte found/nxdomain/failed (+ latences) :
il sert à valider un résolveur DNS contre le rate-limiting du SML.
