# Annuaire PPF — chargement, table cumulative, historique des fichiers

Spec de conception — 2026-07-19.

## 1. Objectif

Ajouter, dans l'onglet **Fichiers**, le chargement d'un nouveau type de fichier :
l'**« Annuaire PPF »** (export B2B du Portail Public de Facturation). À la
différence de l'annuaire Peppol (annule-remplace mono-colonne), le PPF alimente
une table **cumulative** par upsert : on peut charger autant de fichiers que
voulu ; pour repartir de zéro on fait **Reset** (avec confirmation). La liste des
fichiers ayant servi à constituer la table est **persistée et affichée**, chacun
avec son nombre de lignes, d'adressages uniques, et d'adressages **réellement
ajoutés** à la table.

Fonctionnalité **CLIENT-ONLY** : aucune parité avec `cli/popaul.py` (comme
`directory.rs`).

## 2. Format du fichier source

CSV `;`-séparé, encodé UTF-8 **avec BOM**, en-tête sur la première ligne :

```
SIREN;IDENTIFIANT;MOTIF_PRESENCE;UTILISE_PDP_FICTIVE
005520176;005520176;C;1
005520242;005520242;C;1
005520242;005520242;V;0
```

- Un même `IDENTIFIANT` apparaît sur **plusieurs lignes** (un motif par ligne).
- On conserve **tous les champs sauf `SIREN`** (colonne 0 ignorée).
- Exemple de référence : ~124 000 lignes.

## 3. Décisions (arbitrées avec l'utilisateur)

| Sujet | Décision |
|---|---|
| Granularité de la table | 1 ligne par `(identifiant, motif)`, PK composite, colonne `pdp_fictive` |
| Sémantique « adressage » | l'`identifiant` distinct (l'entreprise), pas la ligne |
| « uniques » (par fichier) | nombre d'identifiants distincts **dans le fichier** |
| « ajoutés » (par fichier) | identifiants **jamais présents en table** avant l'ingestion de ce fichier |
| Recharge d'un fichier | chaque chargement = **une entrée** dans l'historique ; doublon détecté par **hash SHA-256 du contenu** (pas le nom) → entrée marquée « (doublon) » |
| `UTILISE_PDP_FICTIVE` hors {0,1} | **rejet du fichier** (fail loud), rien n'est écrit |
| Format de l'`identifiant` | **verbatim** (trim), aucun filtre/validation |
| Colonne date | affichée (« Chargé le ») |
| Portée | chargement + tracking + UI. **Pas** de croisement PPF ↔ résolutions, **pas** de colonne « présent PPF ». Le schéma le permettra plus tard. |

## 4. Modèle de données (`store.rs`)

Deux tables, ajoutées au `const SCHEMA` (donc `CREATE TABLE IF NOT EXISTS`,
créées à l'ouverture — migration idempotente comme `peppol_directory_meta`,
sans `ALTER`) :

```sql
CREATE TABLE IF NOT EXISTS ppf_directory (
  identifiant  TEXT NOT NULL,
  motif        TEXT NOT NULL,
  pdp_fictive  INTEGER NOT NULL,        -- 0 | 1
  PRIMARY KEY (identifiant, motif)
);
CREATE TABLE IF NOT EXISTS ppf_files (      -- historique des ingestions
  id            INTEGER PRIMARY KEY AUTOINCREMENT,
  file_name     TEXT NOT NULL,            -- nom seul (jamais le chemin)
  content_hash  TEXT NOT NULL,            -- SHA-256 hex du contenu brut
  lines         INTEGER NOT NULL,         -- lignes de données lues
  unique_addr   INTEGER NOT NULL,         -- identifiants distincts du fichier
  added_addr    INTEGER NOT NULL,         -- réellement ajoutés à la table
  is_duplicate  INTEGER NOT NULL,         -- 1 si content_hash déjà présent
  loaded_at     INTEGER NOT NULL          -- epoch s
);
```

Contrairement à `peppol_directory` (recréée à chaque chargement), ces tables
**persistent**. `Reset` = `DELETE FROM ppf_directory; DELETE FROM ppf_files;`
dans une transaction.

### Types sérialisés vers le frontend

```rust
#[derive(Debug, Clone, serde::Serialize)]
pub struct PpfFile {          // une entrée d'historique (et retour d'ingestion)
    pub file_name: String,
    pub lines: i64,
    pub unique_addr: i64,
    pub added_addr: i64,
    pub is_duplicate: bool,
    pub loaded_at: i64,
}
#[derive(Debug, Clone, serde::Serialize)]
pub struct PpfSummary { pub distinct_addr: i64, pub file_count: i64 }
```

### Signatures

```rust
// Ingestion cumulative d'un fichier déjà parsé, en UNE transaction.
pub fn ingest_ppf(
    &self,
    file_name: &str,
    content_hash: &str,
    rows: &[crate::ppf::PpfRow],
    lines: i64,
    loaded_at: i64,
) -> Result<PpfFile, String>;

pub fn ppf_files(&self) -> Result<Vec<PpfFile>, String>;   // ORDER BY id DESC
pub fn ppf_summary(&self) -> Result<PpfSummary, String>;   // distinct identifiant, COUNT(*) fichiers
pub fn reset_ppf(&self) -> Result<(), String>;             // DELETE des deux tables
```

### Algorithme de `ingest_ppf` (dans la transaction)

1. `before = SELECT COUNT(DISTINCT identifiant) FROM ppf_directory`.
2. `unique_addr = ` nombre d'identifiants distincts dans `rows` (HashSet Rust —
   source de vérité unique du compteur).
3. Upsert de chaque ligne :
   `INSERT INTO ppf_directory (identifiant, motif, pdp_fictive) VALUES (?,?,?)
    ON CONFLICT(identifiant, motif) DO UPDATE SET pdp_fictive = excluded.pdp_fictive`
   (prepared statement `prepare_cached`, motif de `upsert_batch`).
4. `after = SELECT COUNT(DISTINCT identifiant)`.
5. `added_addr = after - before`.
6. `is_duplicate = EXISTS(SELECT 1 FROM ppf_files WHERE content_hash = ?)`
   (évalué **avant** l'insert de cette entrée).
7. `INSERT INTO ppf_files (...)`.
8. `commit`. Un échec (parse en amont ou SQL) laisse la table intacte.

Renvoie le `PpfFile` inséré (affichage immédiat côté UI).

## 5. Module de parsing `ppf.rs` (nouveau, module étanche)

Pas de download (spécifique Peppol) : parsing seul.

```rust
#[derive(Debug, Clone)]
pub struct PpfRow { pub identifiant: String, pub motif: String, pub pdp_fictive: i64 }

pub struct PpfParse { pub rows: Vec<PpfRow>, pub lines: u64 }

/// Lit un CSV PPF (`;`, en-tête, BOM toléré) en flux. Colonnes lues par index :
/// 0 SIREN (ignoré), 1 IDENTIFIANT, 2 MOTIF_PRESENCE, 3 UTILISE_PDP_FICTIVE.
/// `on_progress(lignes_lues)` tous les 100 000 puis une fois en fin.
/// BLOQUANT : appeler depuis spawn_blocking.
pub fn stream_ppf<R: std::io::Read>(
    reader: R,
    on_progress: impl FnMut(u64),
) -> Result<PpfParse, String>;
```

Détails :
- `csv::ReaderBuilder::new().delimiter(b';').has_headers(true)` (mode strict,
  `flexible=false` : nombre de champs incohérent → `Err`, comme
  `stream_0225_values`).
- Le BOM n'affecte que l'en-tête (ligne ignorée) : lecture par **index**, donc
  sans effet sur les données.
- `identifiant`/`motif` = `record.get(i).trim()`.
- `pdp_fictive` : `"0"` → 0, `"1"` → 1, sinon `Err("… UTILISE_PDP_FICTIVE
  invalide '…' (attendu 0 ou 1) …")`.
- `identifiant` vide (après trim) → `Err` (une PK `NOT NULL` ne peut le stocker ;
  fail loud plutôt qu'un skip silencieux).
- Un champ manquant (`get` renvoie `None`) → `Err`.
- `lines` incrémenté à chaque enregistrement de données lu.

## 6. Commandes Tauri (`commands.rs`) + enregistrement (`lib.rs`)

```rust
#[derive(Clone, Serialize)]
pub struct PpfProgress { pub done: u64 }   // phase parse uniquement

// Async. Lit le fichier en mémoire (les exports PPF sont de taille modérée,
// ~quelques Mo à quelques dizaines de Mo — pas 214 Mo comme l'annuaire Peppol),
// hashe le contenu, parse, ingère. Tout le travail bloquant dans spawn_blocking.
pub async fn load_ppf_file(app: AppHandle, state: State<'_, AppState>, path: String)
    -> Result<crate::store::PpfFile, String>;
// 1. bytes = std::fs::read(&path)
// 2. content_hash = sha256_hex(&bytes)  (sha2 0.10 déjà en dép ; hex formaté à la main)
// 3. parse = ppf::stream_ppf(Cursor::new(&bytes), |n| app.emit("ppf://progress", PpfProgress{done:n}))
// 4. file_name = Path::new(&path).file_name()  (nom seul, jamais le chemin)
// 5. loaded_at = chrono::Utc::now().timestamp()
// 6. store.lock().ingest_ppf(&file_name, &content_hash, &parse.rows, parse.lines as i64, loaded_at)

pub fn ppf_files(state: State<'_, AppState>)   -> Result<Vec<crate::store::PpfFile>, String>;
pub fn ppf_summary(state: State<'_, AppState>) -> Result<crate::store::PpfSummary, String>;
pub fn reset_ppf(state: State<'_, AppState>)   -> Result<(), String>;
```

- `sha256_hex` : `Sha256::new()` + `update` + `finalize`, hex via
  `iter().map(|b| format!("{:02x}", b)).collect()` (pas de crate `hex`).
- Enregistrer les 4 commandes dans `invoke_handler` de `lib.rs`.
- Déclarer `mod ppf;` dans `lib.rs`.

Événement de progression : **`ppf://progress`**, payload `PpfProgress { done }`
(phase parse uniquement — pas de download).

## 7. Frontend

### `index.html`
Nouveau bloc `#ppf-card` (même langage visuel que `#dir-card`), placé **après**
`#dir-card` dans `<section id="step-file">` :
- en-tête « Annuaire PPF » + `.chip-ref` + sous-titre explicatif ;
- `#ppf-dropzone` + `#ppf-browse` (Parcourir…) + **`#ppf-reset` (`.btn-danger`)**
  (remplace le bouton Télécharger de l'annuaire Peppol) ;
- `#ppf-prog` (barre, phase parse) ;
- `#ppf-summary` (résumé : N adressages en table · M fichiers) ;
- `table#ppf-files` (colonnes : Fichier, Lignes, Adressages uniques, Ajoutés,
  Chargé le).

### `app.js` (section « Annuaire PPF », dupliquant le pattern annuaire)
- `renderPpf()` : `invoke("ppf_summary")` + `invoke("ppf_files")`, construit le
  résumé et la table **via `h()`** (jamais innerHTML) ; nom + « (doublon) » si
  `is_duplicate` ; « ajoutés » en vert si > 0, muted si 0 ; état vide « Aucun
  fichier chargé. ».
- `setPpfBusy(busy)`, verrou `ppfBusy`.
- `loadPpf(path)` : `invoke("load_ppf_file", { path })` → `renderPpf()` ; erreur
  → `banner("error", "Annuaire PPF : …")`.
- `listen("ppf://progress", …)` : barre indéterminée, « N lignes lues ».
- Handler `#ppf-browse` : `open({ filters:[csv,txt] })` → `loadPpf`.
- Handler `#ppf-reset` : ouvre la **modale maison** `modal(...nodes)` (titre,
  texte avec compteurs courants, boutons Annuler / Réinitialiser `.btn-danger`) ;
  Réinitialiser → `invoke("reset_ppf")` → `closeModal()` + `renderPpf()`.
- Routeur `tauri://drag-drop` : **3ᵉ cible** — si le point du drop tombe dans
  `#ppf-dropzone`, valider l'extension puis `loadPpf(paths[0])`.
- Au démarrage : `renderPpf()`.

### `styles.css`
- `#ppf-card` = `#dir-card`.
- `#ppf-summary`, `table.ppf-files` (+ `th/td.num` tabular-nums alignés à droite,
  `td.name` ellipsis, `.ppf-dup` muted italique, `td.added.pos b` vert /
  `td.added.zero` muted, `td.when` muted). (Valeurs figées dans la maquette
  validée `maquette-onglet-fichiers-ppf.html`.)

## 8. Hors-scope (explicite)

- Aucun croisement PPF ↔ résolutions ; aucune fonction `ppf_present`.
- Aucune colonne « présent PPF » dans `columns.js` / `output.rs` / le rapport.
- Aucune modification de l'onglet run ni du rapport HTML.
- Aucun bouton « Télécharger » PPF (pas de source réseau connue).

## 9. Tests (TDD — test d'abord, convention projet)

### `store.rs`
- `ppf_upsert_cumulatif_conserve_les_motifs` : `(id1,C),(id1,V),(id2,C)` → 3 lignes, `distinct_addr = 2`.
- `ppf_added_compte_les_nouveaux_identifiants` : fichier 1 apporte `id1,id2` → `added=2` ; fichier 2 apporte `id2,id3` → `added=1` (seul `id3` est nouveau, `id2` déjà en table).
- `ppf_upsert_ecrase_pdp_du_meme_couple` : `(id,C,0)` puis `(id,C,1)` → `pdp=1`, `added=0` au 2ᵉ.
- `ppf_is_duplicate_sur_hash_identique` : même `content_hash` → `is_duplicate=true` au 2ᵉ chargement (nom différent inclus, pour prouver que c'est le hash et pas le nom).
- `ppf_reset_vide_table_et_historique` : après `reset_ppf`, `distinct_addr=0`, `file_count=0`, `ppf_files` vide.
- `ppf_files_ordonne_recent_en_tete`.
- `ppf_summary_distinct_et_count`.
- `ouverture_cree_les_tables_ppf` : base préexistante sans ces tables → créées, ingestion possible (migration idempotente).

### `ppf.rs`
- `parse_ppf_point_virgule_et_bom` : BOM + `;` + en-tête + plusieurs lignes par identifiant → `rows` attendues, `lines` correct.
- `parse_ppf_pdp_invalide_est_une_erreur`.
- `parse_ppf_champ_manquant_ou_malforme_remonte_une_erreur`.
- `parse_ppf_identifiant_vide_est_une_erreur`.
- `parse_ppf_progress_appele_au_moins_une_fois`.

### Frontend
Pas de test unitaire (vanilla JS, l'UI n'a aucune logique métier). Vérification
par relecture + lancement de l'app (dépôt d'un fichier PPF, doublon, Reset).

## 10. Fichiers touchés

- `client/src-tauri/src/ppf.rs` — **nouveau** (parsing).
- `client/src-tauri/src/store.rs` — schéma + `PpfFile`/`PpfSummary` + `ingest_ppf`/`ppf_files`/`ppf_summary`/`reset_ppf`.
- `client/src-tauri/src/commands.rs` — `PpfProgress`, `load_ppf_file`, `ppf_files`, `ppf_summary`, `reset_ppf`, `sha256_hex`.
- `client/src-tauri/src/lib.rs` — `mod ppf;` + enregistrement des 4 commandes.
- `client/src/index.html` — bloc `#ppf-card`.
- `client/src/app.js` — section Annuaire PPF + 3ᵉ cible drag-drop.
- `client/src/styles.css` — styles `#ppf-card` / `table.ppf-files`.
