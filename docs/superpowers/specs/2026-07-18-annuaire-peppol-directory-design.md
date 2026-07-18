# Annuaire Peppol — ingestion des participants 0225 — design

Validé le 2026-07-18 (maquette HTML « Bleu nuit & or » : carte sous la dropzone
principale + états jamais-chargé / téléchargement / analyse ; go sur toutes les
recommandations). Chantier **indépendant** de la résolution — ingestion seule.

## Objectif

Charger dans l'onglet **1. Fichiers** le fichier `export-all-participants.csv`
de l'annuaire Peppol (téléchargeable sur
`https://directory.peppol.eu/export/participants-csv`), par glisser-déposer,
par Parcourir…, ou par un bouton Télécharger. Tous les participants d'adressage
**0225** (SIRENE français) sont chargés dans une table `peppol_directory`
recréée à chaque chargement. La date/heure du dernier chargement est persistée
et affichée.

Un participant déclaré dans l'annuaire n'est **pas** forcément provisionné dans
le réseau (et inversement) : la déclaration est déclarative. Ce chantier ne fait
que constituer la table de référence.

## Périmètre

- **Rust** : nouveau module client-only `client/src-tauri/src/directory.rs` ;
  ajouts dans `store.rs` (persistance), `commands.rs` (3 commandes), `lib.rs`
  (enregistrement + `pub mod directory`).
- **Frontend** : `client/src/index.html`, `client/src/app.js`,
  `client/src/styles.css`.
- **Aucun** changement CLI/serveur. Fonction client-only : pas de contrainte de
  parité avec `cli/popaul.py`.

## Données (dans `superpopaul.db`)

```sql
CREATE TABLE peppol_directory (value TEXT PRIMARY KEY);      -- recréée à chaque chargement
CREATE TABLE IF NOT EXISTS peppol_directory_meta (           -- 1 seule ligne (id=1)
  id         INTEGER PRIMARY KEY CHECK (id = 1),
  loaded_at  INTEGER NOT NULL,   -- unix secondes
  count      INTEGER NOT NULL,   -- lignes distinctes réellement en table
  source     TEXT NOT NULL       -- 'file' | 'download'
);
```

- `value` = ce qui suit le préfixe `iso6523-actorid-upis::0225:`, **verbatim**
  (les suffixes `_replyto`, `_cdv_…`, `_SIRET`, etc. sont conservés).
- **PRIMARY KEY** sur `value` : déduplication naturelle (INSERT OR IGNORE) et
  forme « ensemble d'adressages » pour un lookup ultérieur.
- La recréation `DROP TABLE IF EXISTS` + `CREATE` + tous les `INSERT` + l'upsert
  meta se font dans **une seule transaction** : un échec en cours laisse la table
  précédente intacte (atomicité), l'horodatage ne peut pas diverger du contenu.
- `count` = `SELECT COUNT(*)` après insertion (valeur après dédup), pas le nombre
  de lignes 0225 lues.

## Rust — `directory.rs` (module étanche, TDD)

Constante de préfixe construite à partir de `pid::DEFAULT_SCHEME` (invariant du
scheme, cohérent avec la canonicalisation) et du littéral `0225` (l'exigence est
explicitement « type 0225 ») :

```rust
// format!("{}::0225:", pid::DEFAULT_SCHEME) == "iso6523-actorid-upis::0225:"
```

- `fn parse_0225_value(participant_id: &str) -> Option<String>` — **fonction
  pure**, cœur testable :
  - trim de l'entrée ;
  - si elle commence par le préfixe 0225 → `Some(reste)` ; le reste vide →
    `None` (préfixe seul n'est pas un adressage) ;
  - tout autre scheme (`0002`, `0007`, `0009`, `0088`, …) → `None`.
- `fn stream_0225_values<R: Read>(reader, on_progress: impl FnMut(u64)) ->
  Result<Vec<String>, String>` — lit le CSV en flux (crate `csv`, `has_headers`,
  colonne unique `Participant ID`), applique `parse_0225_value` à chaque
  enregistrement, appelle `on_progress(lignes_lues)` périodiquement (ex. tous les
  100 000), renvoie les valeurs 0225 dans l'ordre de lecture. Testable avec un
  `Cursor` sur un mini-CSV.
- `async fn download_to_temp(url, proxy, creds, on_progress) ->
  Result<NamedTempFile, String>` — téléchargement `reqwest` en streaming
  (`bytes_stream`) écrit dans un fichier temporaire (`tempfile`), progression sur
  `bytes_reçus / content-length`. Honore le proxy configuré comme `api.rs`
  (`reqwest::Proxy::all` + creds si présents). Le temp est **supprimé** à la fin
  (Drop de `NamedTempFile`) : le brut 214 Mo n'est jamais conservé.

Le parsing (~30–50 Mo de valeurs en mémoire, transitoire) est fait **sans** tenir
le verrou du `Store` ; seule l'écriture en base prend le verrou.

## Rust — `store.rs` (propriété SQLite, convention)

- `fn replace_peppol_directory(&self, values: &[String], source: &str,
  loaded_at: i64) -> Result<usize, String>` — une transaction : `DROP`+`CREATE`
  de `peppol_directory`, `INSERT OR IGNORE` par lots (statement préparé, motif
  `upsert_batch`), upsert de `peppol_directory_meta`, renvoie
  `SELECT COUNT(*)`. La table meta est créée au `init` (migration idempotente
  comme les colonnes CTC).
- `fn peppol_directory_status(&self) -> Result<Option<DirStatus>, String>` — lit
  la ligne meta ; `None` si jamais chargé. `DirStatus { loaded_at, count,
  source }` (`serde::Serialize`).

## Commandes Tauri (`commands.rs`, enregistrées dans `lib.rs`)

Motif async + `tokio::task::spawn_blocking` + `store.clone()` puis `lock()` (déjà
utilisé par `scan_unique_pids`/`start_run`).

- `directory_status(state) -> Option<DirStatus>` — synchrone, lit le meta au
  démarrage et après chaque chargement.
- `load_directory_file(app, state, path) -> DirLoadResult` — `spawn_blocking` :
  ouvre le fichier local, `stream_0225_values` (progression émise), puis
  `replace_peppol_directory(source = "file")`.
- `download_directory(app, state) -> DirLoadResult` — `download_to_temp`
  (progression), puis `spawn_blocking` parse + `replace_peppol_directory(source =
  "download")` ; le temp est supprimé ensuite.

`DirLoadResult { loaded_at: i64, count: usize }`. Une erreur remonte en `Err`
(bannière côté UI).

## Progression (événements)

Événement Tauri `directory://progress`, payload
`{ phase: "download" | "parse", done: u64, total: u64 | null }` :
- `download` : `done`/`total` = octets reçus / `content-length` (barre en %) ;
- `parse` : `done` = lignes lues, `total = null` (barre indéterminée) — comme la
  maquette.

L'UI écoute pendant l'opération, se rafraîchit à la résolution de la commande.

## UI — onglet 1. Fichiers

Carte **« Annuaire Peppol (référence) »** insérée sous `#dropzone` (structure
statique dans `index.html`, styles repris de la maquette) :

- en-tête : titre + puce « référence » + phrase déclaratif ≠ provisionné ;
- zone de dépôt `export-all-participants.csv` + boutons **Parcourir…** et
  **⤓ Télécharger** (doré, `.btn-primary`) ;
- ligne d'état : « Dernier chargement : `<date locale>` — `<N>` adressages
  0225 (depuis le fichier / téléchargé) » ou « Jamais chargé. » ;
- pendant l'op : boutons grisés + barre de progression (label + `%` ou
  indéterminée).

`app.js` :
- au démarrage, `directory_status` → rendu de la ligne d'état ;
- **Parcourir…** : `open({ filters: csv/txt })` → `load_directory_file` ;
- **Télécharger** : `download_directory` ;
- écoute `directory://progress` → maj barre ;
- **routage du drop** : dans le listener `tauri://drag-drop` existant, quand
  l'étape courante est `file`, hit-test de `e.payload.position` (pixels
  physiques → diviser par `devicePixelRatio`) contre `getBoundingClientRect()`
  de `#dropzone` et de la zone annuaire → route vers `pickInput` (principal) ou
  `load_directory_file` (annuaire). `.over` géré par listeners `dragover`/
  `dragleave` sur la zone, comme l'existant.
- date formatée côté JS depuis `loaded_at` (`new Date(loaded_at*1000)
  .toLocaleString('fr-FR', …)`).

## Sécurité

- **Jamais d'innerHTML avec des données dynamiques** : la valeur affichée
  (compteur, source, date) passe par `textContent` / le helper `h()`. Un CSV est
  une entrée non fiable, même si seuls des nombres/énumérés backend transitent
  ici.
- Identifiants proxy inchangés : non sérialisés, jamais écrits (test
  `config::proxy_creds_never_serialized` non contourné). Le download les utilise
  en mémoire uniquement.

## Hors périmètre

- **Croisement** avec la résolution/sortie (signal « déclaré mais non
  provisionné » et inverse) : chantier séparé, sémantique à définir.
- Pas de filtrage des suffixes techniques (`_replyto`, `_cdv_…`) : conservés
  verbatim.
- Pas de conservation du CSV brut téléchargé.
- Autres schemes (0002, 0007, 0088, …) non chargés.
- Pas d'index secondaire (SIREN dérivé) : ajouté avec le chantier croisement s'il
  l'exige.

## Tests (TDD Rust d'abord)

`directory::tests` :
- `parse_0225_value` : 0225 nominal → `Some` ; 0225 avec suffixe `_replyto` /
  `_cdv_…` → `Some` verbatim ; scheme `0002`/`0009` → `None` ; préfixe seul sans
  valeur → `None` ; entrée espacée trimmée.
- `stream_0225_values` : `Cursor` sur un CSV `Participant ID` + en-tête + mix de
  schemes → seules les valeurs 0225 dans l'ordre, en-tête ignoré, `on_progress`
  appelé.

`store::tests` :
- `replace_peppol_directory` : insère, `count` correct, relu via
  `peppol_directory_status` ;
- **recréation** : un 2ᵉ appel remplace intégralement (l'ancien contenu
  disparaît) ;
- **dédup** : doublons en entrée → une seule ligne (PK) et `count` dédupliqué ;
- **meta** : `loaded_at` / `count` / `source` persistés et relus ;
- migration : ouverture d'une base sans `peppol_directory_meta` la crée.

Le CSV réel (214 Mo, 5,2 M lignes) n'est pas testé unitairement : vérification
manuelle en fin de chantier — charger `../deaddrop/in/export-all-participants.csv`,
attendre `count ≈ 1 542 342`, vérifier l'horodatage affiché.
