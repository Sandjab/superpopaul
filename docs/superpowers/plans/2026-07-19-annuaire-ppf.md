# Annuaire PPF — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Charger des fichiers « Annuaire PPF » (export B2B du PPF) dans une table SQLite cumulative par upsert, avec historique persisté des fichiers ingérés, Reset, et un bloc dédié dans l'onglet Fichiers.

**Architecture:** Backend Rust (Tauri) — nouveau module de parsing `ppf.rs`, deux tables SQLite (`ppf_directory` PK `(identifiant, motif)` + `ppf_files` historique) et méthodes `Store`, commandes Tauri (`load_ppf_file`/`ppf_files`/`ppf_summary`/`reset_ppf`). Frontend vanilla JS — bloc `#ppf-card` dupliquant le pattern de l'annuaire Peppol, sans download, avec bouton Reset (modale de confirmation) et table d'historique. Aucun croisement avec les résolutions (hors-scope).

**Tech Stack:** Rust, rusqlite, crate `csv`, `sha2` (déjà en dépendance), Tauri 2 ; HTML/CSS/JS vanilla.

**Spec:** `docs/superpowers/specs/2026-07-19-ppf-directory-design.md`

**Conventions projet:** TDD (test d'abord pour toute logique Rust) ; commits `feat(superpopaul): …` ; textes UI en français ; jamais d'`innerHTML` avec données dynamiques (helper `h()`) ; modules Rust étanches.

---

## File Structure

- **Create** `client/src-tauri/src/ppf.rs` — parsing du CSV PPF en flux (aucune autre responsabilité ; pas de download).
- **Modify** `client/src-tauri/src/store.rs` — schéma des 2 tables, types `PpfFile`/`PpfSummary`, méthodes `ingest_ppf`/`ppf_files`/`ppf_summary`/`reset_ppf`.
- **Modify** `client/src-tauri/src/commands.rs` — `sha256_hex`, `PpfProgress`, commandes `load_ppf_file`/`ppf_files`/`ppf_summary`/`reset_ppf`.
- **Modify** `client/src-tauri/src/lib.rs` — `mod ppf;` + enregistrement des 4 commandes.
- **Modify** `client/src/index.html` — bloc `#ppf-card`.
- **Modify** `client/src/styles.css` — styles `#ppf-card` / `table.ppf-files`.
- **Modify** `client/src/app.js` — section « Annuaire PPF » + 3ᵉ cible du routeur drag-drop.

Toutes les commandes `cargo` s'exécutent depuis `client/src-tauri/`.

---

## Task 1 : Module de parsing `ppf.rs`

**Files:**
- Create: `client/src-tauri/src/ppf.rs`
- Modify: `client/src-tauri/src/lib.rs` (ajout `mod ppf;`)

- [ ] **Step 1 : Écrire le module avec ses tests (test d'abord dans le même fichier)**

Créer `client/src-tauri/src/ppf.rs` avec l'en-tête, les types, la fonction et le `mod tests` :

```rust
//! Ingestion de l'annuaire PPF (export B2B du Portail Public de Facturation) —
//! fonctionnalité CLIENT-ONLY : aucune parité avec cli/popaul.py.
//! Format : CSV `;`, en-tête (BOM toléré), colonnes
//! SIREN;IDENTIFIANT;MOTIF_PRESENCE;UTILISE_PDP_FICTIVE. On conserve tout
//! sauf le SIREN (colonne 0).

use std::io::Read;

/// Une ligne de données PPF retenue (le SIREN de tête est ignoré).
#[derive(Debug, Clone)]
pub struct PpfRow {
    pub identifiant: String,
    pub motif: String,
    pub pdp_fictive: i64, // 0 | 1
}

/// Résultat d'un parse : lignes retenues + nombre de lignes de données lues.
pub struct PpfParse {
    pub rows: Vec<PpfRow>,
    pub lines: u64,
}

/// Lit un CSV PPF (`;`, en-tête, BOM toléré) en flux. Colonnes par index :
/// 0 SIREN (ignoré), 1 IDENTIFIANT, 2 MOTIF_PRESENCE, 3 UTILISE_PDP_FICTIVE.
/// `on_progress(lignes_lues)` tous les 100 000 puis une fois en fin de lecture.
/// BLOQUANT (fichier volumineux) : appeler depuis `spawn_blocking`.
pub fn stream_ppf<R: Read>(
    reader: R,
    mut on_progress: impl FnMut(u64),
) -> Result<PpfParse, String> {
    let mut rdr = csv::ReaderBuilder::new()
        .delimiter(b';')
        .has_headers(true)
        .from_reader(reader);
    let mut record = csv::StringRecord::new();
    let mut rows = Vec::new();
    let mut lines: u64 = 0;
    loop {
        match rdr.read_record(&mut record) {
            Ok(true) => {
                lines += 1;
                let identifiant = record.get(1).unwrap_or("").trim();
                let motif = record.get(2).unwrap_or("").trim();
                let pdp_raw = record.get(3).unwrap_or("").trim();
                if identifiant.is_empty() {
                    return Err(format!("ligne {lines} : IDENTIFIANT vide"));
                }
                let pdp_fictive = match pdp_raw {
                    "0" => 0,
                    "1" => 1,
                    other => {
                        return Err(format!(
                            "ligne {lines} : UTILISE_PDP_FICTIVE invalide '{other}' (attendu 0 ou 1)"
                        ))
                    }
                };
                rows.push(PpfRow {
                    identifiant: identifiant.to_string(),
                    motif: motif.to_string(),
                    pdp_fictive,
                });
                if lines % 100_000 == 0 {
                    on_progress(lines);
                }
            }
            Ok(false) => break,
            Err(e) => return Err(format!("lecture CSV PPF : {e}")),
        }
    }
    on_progress(lines);
    Ok(PpfParse { rows, lines })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ppf_point_virgule_et_bom() {
        // BOM UTF-8 + en-tête + 3 lignes (id 005520242 sur deux motifs).
        let csv = "\u{feff}SIREN;IDENTIFIANT;MOTIF_PRESENCE;UTILISE_PDP_FICTIVE\n\
                   005520176;005520176;C;1\n\
                   005520242;005520242;C;1\n\
                   005520242;005520242;V;0\n";
        let mut calls = 0u32;
        let p = stream_ppf(std::io::Cursor::new(csv), |_| calls += 1).unwrap();
        assert_eq!(p.lines, 3);
        assert_eq!(p.rows.len(), 3);
        assert_eq!(p.rows[0].identifiant, "005520176");
        assert_eq!(p.rows[0].motif, "C");
        assert_eq!(p.rows[0].pdp_fictive, 1);
        assert_eq!(p.rows[2].motif, "V");
        assert_eq!(p.rows[2].pdp_fictive, 0);
        assert!(calls >= 1, "on_progress doit être appelé au moins une fois");
    }

    #[test]
    fn parse_ppf_pdp_invalide_est_une_erreur() {
        let csv = "SIREN;IDENTIFIANT;MOTIF_PRESENCE;UTILISE_PDP_FICTIVE\n\
                   005520176;005520176;C;X\n";
        let res = stream_ppf(std::io::Cursor::new(csv), |_| {});
        assert!(res.is_err());
        assert!(res.unwrap_err().contains("UTILISE_PDP_FICTIVE"));
    }

    #[test]
    fn parse_ppf_champ_manquant_remonte_une_erreur() {
        // 3 champs au lieu de 4 (mode strict : nombre de champs incohérent).
        let csv = "SIREN;IDENTIFIANT;MOTIF_PRESENCE;UTILISE_PDP_FICTIVE\n\
                   005520176;005520176;C\n";
        let res = stream_ppf(std::io::Cursor::new(csv), |_| {});
        assert!(res.is_err(), "un CSV malformé doit remonter une Err");
    }

    #[test]
    fn parse_ppf_identifiant_vide_est_une_erreur() {
        let csv = "SIREN;IDENTIFIANT;MOTIF_PRESENCE;UTILISE_PDP_FICTIVE\n\
                   005520176;;C;1\n";
        let res = stream_ppf(std::io::Cursor::new(csv), |_| {});
        assert!(res.is_err());
        assert!(res.unwrap_err().contains("IDENTIFIANT"));
    }

    #[test]
    fn parse_ppf_entete_seule_ne_produit_rien() {
        let p = stream_ppf(
            std::io::Cursor::new("SIREN;IDENTIFIANT;MOTIF_PRESENCE;UTILISE_PDP_FICTIVE\n"),
            |_| {},
        )
        .unwrap();
        assert_eq!(p.lines, 0);
        assert!(p.rows.is_empty());
    }
}
```

- [ ] **Step 2 : Déclarer le module**

Dans `client/src-tauri/src/lib.rs`, ajouter `mod ppf;` à côté des autres déclarations de modules (chercher `mod directory;` et ajouter la ligne juste après, en respectant l'ordre alphabétique/existant).

- [ ] **Step 3 : Lancer les tests — ils doivent échouer d'abord puis passer**

Run: `cargo test --quiet ppf::`
Expected: PASS (5 tests). Si un test échoue, corriger l'implémentation avant de continuer.
Note : la discipline « voir rouge » est ici obtenue en écrivant les tests dans le même fichier ; pour vérifier qu'ils testent réellement, casser temporairement un `assert` n'est pas requis — les 5 tests doivent passer.

- [ ] **Step 4 : Commit**

```bash
git add client/src-tauri/src/ppf.rs client/src-tauri/src/lib.rs
git commit -m "feat(superpopaul): module ppf — parsing CSV PPF (;, BOM, verdict pdp)"
```

---

## Task 2 : Tables et méthodes `Store` (`store.rs`)

**Files:**
- Modify: `client/src-tauri/src/store.rs` (schéma, types, méthodes, tests)

- [ ] **Step 1 : Écrire les tests (test d'abord)**

Dans le `mod tests` de `client/src-tauri/src/store.rs`, ajouter un helper et les tests. Placer après les tests annuaire existants (`directory_present_annuaire_vide`) :

```rust
    fn ppf_row(id: &str, motif: &str, pdp: i64) -> crate::ppf::PpfRow {
        crate::ppf::PpfRow { identifiant: id.into(), motif: motif.into(), pdp_fictive: pdp }
    }

    #[test]
    fn ppf_upsert_cumulatif_conserve_les_motifs() {
        let s = Store::open_in_memory().unwrap();
        let rows = vec![ppf_row("id1", "C", 1), ppf_row("id1", "V", 0), ppf_row("id2", "C", 1)];
        let f = s.ingest_ppf("a.csv", "hashA", &rows, 3, 1000).unwrap();
        assert_eq!(f.unique_addr, 2, "id1 et id2 : deux adressages distincts");
        assert_eq!(f.added_addr, 2);
        assert_eq!(f.lines, 3);
        assert!(!f.is_duplicate);
        let sum = s.ppf_summary().unwrap();
        assert_eq!(sum.distinct_addr, 2);
        assert_eq!(sum.file_count, 1);
    }

    #[test]
    fn ppf_added_compte_les_nouveaux_identifiants() {
        let s = Store::open_in_memory().unwrap();
        let f1 = s.ingest_ppf("a.csv", "hA", &[ppf_row("id1", "C", 1), ppf_row("id2", "C", 1)], 2, 1).unwrap();
        assert_eq!(f1.added_addr, 2);
        // Fichier 2 : id2 déjà là, seul id3 est nouveau.
        let f2 = s.ingest_ppf("b.csv", "hB", &[ppf_row("id2", "C", 1), ppf_row("id3", "C", 0)], 2, 2).unwrap();
        assert_eq!(f2.unique_addr, 2);
        assert_eq!(f2.added_addr, 1);
        assert_eq!(s.ppf_summary().unwrap().distinct_addr, 3);
    }

    #[test]
    fn ppf_upsert_ecrase_pdp_du_meme_couple() {
        let s = Store::open_in_memory().unwrap();
        s.ingest_ppf("a.csv", "hA", &[ppf_row("id1", "C", 0)], 1, 1).unwrap();
        let f2 = s.ingest_ppf("b.csv", "hB", &[ppf_row("id1", "C", 1)], 1, 2).unwrap();
        assert_eq!(f2.added_addr, 0, "(id1,C) déjà présent : aucun nouvel adressage");
        let pdp: i64 = s
            .conn
            .query_row(
                "SELECT pdp_fictive FROM ppf_directory WHERE identifiant='id1' AND motif='C'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(pdp, 1, "l'upsert écrase pdp_fictive");
    }

    #[test]
    fn ppf_is_duplicate_sur_hash_identique_pas_le_nom() {
        let s = Store::open_in_memory().unwrap();
        let f1 = s.ingest_ppf("a.csv", "HASH", &[ppf_row("id1", "C", 1)], 1, 1).unwrap();
        assert!(!f1.is_duplicate);
        // Nom DIFFÉRENT, même hash de contenu : c'est le hash qui décide.
        let f2 = s.ingest_ppf("autre-nom.csv", "HASH", &[ppf_row("id1", "C", 1)], 1, 2).unwrap();
        assert!(f2.is_duplicate);
        assert_eq!(f2.added_addr, 0);
    }

    #[test]
    fn ppf_reset_vide_table_et_historique() {
        let s = Store::open_in_memory().unwrap();
        s.ingest_ppf("a.csv", "hA", &[ppf_row("id1", "C", 1)], 1, 1).unwrap();
        s.reset_ppf().unwrap();
        let sum = s.ppf_summary().unwrap();
        assert_eq!(sum.distinct_addr, 0);
        assert_eq!(sum.file_count, 0);
        assert!(s.ppf_files().unwrap().is_empty());
    }

    #[test]
    fn ppf_files_ordonne_recent_en_tete() {
        let s = Store::open_in_memory().unwrap();
        s.ingest_ppf("a.csv", "hA", &[ppf_row("id1", "C", 1)], 1, 10).unwrap();
        s.ingest_ppf("b.csv", "hB", &[ppf_row("id2", "C", 1)], 1, 20).unwrap();
        let files = s.ppf_files().unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].file_name, "b.csv", "le plus récent en tête (id DESC)");
        assert_eq!(files[1].file_name, "a.csv");
    }

    #[test]
    fn ouverture_cree_les_tables_ppf() {
        // Base préexistante sans les tables PPF → créées à l'ouverture.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sans_ppf.db");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE resolutions (
                   participant TEXT PRIMARY KEY, exists_in_peppol INTEGER,
                   pa_code TEXT, pa_name TEXT, pa_country TEXT,
                   extended_ctc_fr INTEGER, api_status TEXT NOT NULL,
                   resolved_at INTEGER NOT NULL );",
            )
            .unwrap();
        }
        let s = Store::open(&path).unwrap();
        assert_eq!(s.ppf_summary().unwrap().file_count, 0);
        s.ingest_ppf("a.csv", "hA", &[ppf_row("id1", "C", 1)], 1, 1).unwrap();
        assert_eq!(s.ppf_summary().unwrap().distinct_addr, 1);
    }
```

- [ ] **Step 2 : Lancer les tests pour les voir échouer**

Run: `cargo test --quiet store::tests::ppf_ store::tests::ouverture_cree_les_tables_ppf`
Expected: FAIL (erreurs de compilation : `ingest_ppf`, `ppf_summary`, `ppf_files`, `reset_ppf`, `PpfFile`, `PpfSummary` inconnus).

- [ ] **Step 3 : Ajouter les 2 tables au `const SCHEMA`**

Dans `client/src-tauri/src/store.rs`, `const SCHEMA`, ajouter avant la fin de la chaîne (après le bloc `peppol_directory_meta`) :

```sql
CREATE TABLE IF NOT EXISTS ppf_directory (
  identifiant  TEXT NOT NULL,
  motif        TEXT NOT NULL,
  pdp_fictive  INTEGER NOT NULL,
  PRIMARY KEY (identifiant, motif)
);
CREATE TABLE IF NOT EXISTS ppf_files (
  id            INTEGER PRIMARY KEY AUTOINCREMENT,
  file_name     TEXT NOT NULL,
  content_hash  TEXT NOT NULL,
  lines         INTEGER NOT NULL,
  unique_addr   INTEGER NOT NULL,
  added_addr    INTEGER NOT NULL,
  is_duplicate  INTEGER NOT NULL,
  loaded_at     INTEGER NOT NULL
);
```

- [ ] **Step 4 : Ajouter les types sérialisés**

Après la struct `DirStatus` (vers la ligne 31), ajouter :

```rust
/// Une entrée de l'historique des ingestions PPF (et retour d'une ingestion).
#[derive(Debug, Clone, serde::Serialize)]
pub struct PpfFile {
    pub file_name: String,
    pub lines: i64,
    pub unique_addr: i64,
    pub added_addr: i64,
    pub is_duplicate: bool,
    pub loaded_at: i64,
}

/// Résumé de l'annuaire PPF : adressages distincts en table et nombre de fichiers.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PpfSummary {
    pub distinct_addr: i64,
    pub file_count: i64,
}
```

- [ ] **Step 5 : Ajouter les méthodes à `impl Store`**

Après `peppol_directory_status` (avant la fermeture de `impl Store`), ajouter :

```rust
    /// Ingestion cumulative d'un fichier PPF déjà parsé, en UNE transaction.
    /// `added_addr` = identifiants distincts nouveaux (COUNT(DISTINCT) après −
    /// avant) ; `is_duplicate` = ce `content_hash` a déjà été ingéré.
    pub fn ingest_ppf(
        &self,
        file_name: &str,
        content_hash: &str,
        rows: &[crate::ppf::PpfRow],
        lines: i64,
        loaded_at: i64,
    ) -> Result<PpfFile, String> {
        let unique_addr = rows
            .iter()
            .map(|r| r.identifiant.as_str())
            .collect::<HashSet<_>>()
            .len() as i64;
        let tx = self.conn.unchecked_transaction().map_err(|e| e.to_string())?;
        let before: i64 = tx
            .query_row("SELECT COUNT(DISTINCT identifiant) FROM ppf_directory", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        {
            let mut stmt = tx
                .prepare_cached(
                    "INSERT INTO ppf_directory (identifiant, motif, pdp_fictive)
                     VALUES (?1, ?2, ?3)
                     ON CONFLICT(identifiant, motif) DO UPDATE SET pdp_fictive = excluded.pdp_fictive",
                )
                .map_err(|e| e.to_string())?;
            for r in rows {
                stmt.execute(params![r.identifiant, r.motif, r.pdp_fictive])
                    .map_err(|e| e.to_string())?;
            }
        }
        let after: i64 = tx
            .query_row("SELECT COUNT(DISTINCT identifiant) FROM ppf_directory", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        let is_duplicate: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM ppf_files WHERE content_hash = ?1)",
                params![content_hash],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        let added_addr = after - before;
        tx.execute(
            "INSERT INTO ppf_files
               (file_name, content_hash, lines, unique_addr, added_addr, is_duplicate, loaded_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![file_name, content_hash, lines, unique_addr, added_addr, is_duplicate, loaded_at],
        )
        .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(PpfFile { file_name: file_name.to_string(), lines, unique_addr, added_addr, is_duplicate, loaded_at })
    }

    /// Historique des fichiers ingérés, le plus récent en tête.
    pub fn ppf_files(&self) -> Result<Vec<PpfFile>, String> {
        let mut stmt = self
            .conn
            .prepare_cached(
                "SELECT file_name, lines, unique_addr, added_addr, is_duplicate, loaded_at
                 FROM ppf_files ORDER BY id DESC",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| {
                Ok(PpfFile {
                    file_name: r.get(0)?,
                    lines: r.get(1)?,
                    unique_addr: r.get(2)?,
                    added_addr: r.get(3)?,
                    is_duplicate: r.get::<_, i64>(4)? != 0,
                    loaded_at: r.get(5)?,
                })
            })
            .map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| e.to_string())?);
        }
        Ok(out)
    }

    /// Adressages distincts en table et nombre de fichiers ingérés.
    pub fn ppf_summary(&self) -> Result<PpfSummary, String> {
        let distinct_addr: i64 = self
            .conn
            .query_row("SELECT COUNT(DISTINCT identifiant) FROM ppf_directory", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        let file_count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM ppf_files", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        Ok(PpfSummary { distinct_addr, file_count })
    }

    /// Reset : vide la table et l'historique (les fichiers sur disque intacts).
    pub fn reset_ppf(&self) -> Result<(), String> {
        self.conn
            .execute_batch("DELETE FROM ppf_directory; DELETE FROM ppf_files;")
            .map_err(|e| e.to_string())
    }
```

Note : `HashSet` est déjà importé en tête de `store.rs` (`use std::collections::{HashMap, HashSet};`). `params` aussi (`use rusqlite::{params, ...}`).

- [ ] **Step 6 : Lancer les tests — ils doivent passer**

Run: `cargo test --quiet store::`
Expected: PASS (tous les tests `store::`, dont les 7 nouveaux `ppf_*`/`ouverture_cree_les_tables_ppf`).

- [ ] **Step 7 : Commit**

```bash
git add client/src-tauri/src/store.rs
git commit -m "feat(superpopaul): store PPF — tables cumulatives, ingest upsert, historique, reset"
```

---

## Task 3 : Commandes Tauri (`commands.rs`) + enregistrement (`lib.rs`)

**Files:**
- Modify: `client/src-tauri/src/commands.rs`
- Modify: `client/src-tauri/src/lib.rs`

- [ ] **Step 1 : Écrire le test de `sha256_hex` (test d'abord)**

Dans `client/src-tauri/src/commands.rs`, si un `#[cfg(test)] mod tests { ... }` existe, y ajouter le test ; sinon créer le bloc en fin de fichier :

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_hex_valeurs_connues() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
```

- [ ] **Step 2 : Lancer le test pour le voir échouer**

Run: `cargo test --quiet commands::tests::sha256_hex_valeurs_connues`
Expected: FAIL (compilation : `sha256_hex` introuvable).

- [ ] **Step 3 : Implémenter `sha256_hex` + `PpfProgress` + les 4 commandes**

Dans `client/src-tauri/src/commands.rs`, ajouter (près des structs `DirProgress`/`DirLoadResult`, vers la ligne 571) :

```rust
/// Progression d'ingestion PPF (phase parse ; pas de download).
#[derive(Clone, Serialize)]
pub struct PpfProgress {
    pub done: u64,
}

/// SHA-256 hexadécimal minuscule du contenu brut d'un fichier.
fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for b in digest {
        out.push_str(&format!("{b:02x}"));
    }
    out
}
```

Puis, à la suite des commandes annuaire (`download_directory`, vers la ligne 669), ajouter :

```rust
/// Charge un fichier PPF : lit le contenu en mémoire (exports de taille
/// modérée — pas 214 Mo comme l'annuaire Peppol), hashe, parse, ingère par
/// upsert cumulatif. Renvoie l'entrée d'historique créée. BLOQUANT →
/// spawn_blocking.
#[tauri::command]
pub async fn load_ppf_file(
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
) -> Result<crate::store::PpfFile, String> {
    let store = state.store.clone();
    tokio::task::spawn_blocking(move || {
        let bytes = std::fs::read(&path).map_err(|e| format!("lecture du fichier PPF : {e}"))?;
        let content_hash = sha256_hex(&bytes);
        let file_name = Path::new(&path)
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.clone());
        let parse = crate::ppf::stream_ppf(std::io::Cursor::new(&bytes), |done| {
            let _ = app.emit("ppf://progress", PpfProgress { done });
        })?;
        store.lock().unwrap().ingest_ppf(
            &file_name,
            &content_hash,
            &parse.rows,
            parse.lines as i64,
            chrono::Utc::now().timestamp(),
        )
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Historique des fichiers PPF ingérés (le plus récent en tête).
#[tauri::command]
pub fn ppf_files(state: State<'_, AppState>) -> Result<Vec<crate::store::PpfFile>, String> {
    state.store.lock().unwrap().ppf_files()
}

/// Résumé de l'annuaire PPF (adressages distincts, nombre de fichiers).
#[tauri::command]
pub fn ppf_summary(state: State<'_, AppState>) -> Result<crate::store::PpfSummary, String> {
    state.store.lock().unwrap().ppf_summary()
}

/// Vide l'annuaire PPF et son historique.
#[tauri::command]
pub fn reset_ppf(state: State<'_, AppState>) -> Result<(), String> {
    state.store.lock().unwrap().reset_ppf()
}
```

Note : `AppHandle`, `State`, `AppState`, `Path`, `Serialize`, le trait d'`emit` (`Emitter`) et `chrono` sont déjà importés dans `commands.rs` (utilisés par les commandes annuaire). Vérifier en haut du fichier ; ne rien réimporter en double.

- [ ] **Step 4 : Enregistrer les commandes dans `lib.rs`**

Dans `client/src-tauri/src/lib.rs`, `invoke_handler![...]` (là où figurent `commands::directory_status, commands::load_directory_file, commands::download_directory`), ajouter :

```rust
            commands::load_ppf_file,
            commands::ppf_files,
            commands::ppf_summary,
            commands::reset_ppf,
```

- [ ] **Step 5 : Lancer les tests + compiler tout le backend**

Run: `cargo test --quiet`
Expected: PASS (toute la suite, dont `sha256_hex_valeurs_connues`). La compilation valide aussi `load_ppf_file`/`ppf_files`/`ppf_summary`/`reset_ppf`.

- [ ] **Step 6 : Commit**

```bash
git add client/src-tauri/src/commands.rs client/src-tauri/src/lib.rs
git commit -m "feat(superpopaul): commandes PPF — load/list/summary/reset + hash SHA-256"
```

---

## Task 4 : Frontend — bloc Annuaire PPF (`index.html`, `styles.css`, `app.js`)

**Files:**
- Modify: `client/src/index.html` (bloc `#ppf-card`)
- Modify: `client/src/styles.css` (styles)
- Modify: `client/src/app.js` (section PPF + 3ᵉ cible drag-drop)

Pas de test unitaire (vanilla JS, l'UI n'a aucune logique métier) ; vérification par lancement de l'app en fin de tâche.

- [ ] **Step 1 : Ajouter le markup `#ppf-card`**

Dans `client/src/index.html`, juste après la fermeture de `#dir-card` (ligne 53 `</div>`) et avant `</section>` (ligne 54) :

```html
      <div id="ppf-card">
        <div class="dir-head">
          <h3>Annuaire PPF <span class="chip-ref">référence</span></h3>
          <span class="muted">Annuaire B2B du PPF (identifiants destinataires). Cumulatif : chaque fichier déposé enrichit la table ; « Reset » la vide entièrement.</span>
        </div>
        <div id="ppf-dropzone">
          <span>Dépose un export PPF (<code>export_b2b_…csv</code>) ici</span>
          <span class="dir-spacer"></span>
          <button id="ppf-browse">Parcourir…</button>
          <button id="ppf-reset" class="btn-danger">Reset…</button>
        </div>
        <div id="ppf-prog" class="hidden">
          <div class="dir-prog-lbl"><span id="ppf-prog-text"></span><span id="ppf-prog-num"></span></div>
          <div id="ppf-bar" class="dir-bar"><span></span></div>
        </div>
        <p id="ppf-summary" class="muted">Aucun fichier chargé.</p>
        <table id="ppf-files" class="ppf-files hidden"></table>
      </div>
```

- [ ] **Step 2 : Ajouter les styles**

À la fin de `client/src/styles.css`, ajouter :

```css
/* --- Carte Annuaire PPF (onglet Fichiers) : cumulative, historique des fichiers.
   Même langage visuel que #dir-card ; bouton Reset (danger) au lieu de Télécharger. --- */
#ppf-card { margin-top: 16px; border: 1px solid var(--border); background: transparent;
            border-radius: 12px; padding: 14px 16px; }
#ppf-dropzone { border: 2px dashed var(--border); border-radius: 10px; padding: 16px 14px;
                display: flex; align-items: center; flex-wrap: wrap; gap: 10px; color: var(--muted); }
#ppf-dropzone.over { border-color: var(--gold); color: var(--fg); }
#ppf-prog { margin: 12px 0 0; }
#ppf-summary { margin: 14px 0 6px; font-size: 13px; }
#ppf-summary b { color: var(--fg); }
#ppf-summary .dot { color: var(--green); }
table.ppf-files { border-collapse: collapse; width: 100%; margin: 6px 0 0; font-size: 12.5px; }
.ppf-files th, .ppf-files td { border: 1px solid var(--border); padding: 4px 10px; text-align: left; }
.ppf-files th { background: var(--card); color: var(--muted); font-weight: 600; }
.ppf-files th.num, .ppf-files td.num { text-align: right; font-variant-numeric: tabular-nums; white-space: nowrap; }
.ppf-files td.name { max-width: 34ch; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.ppf-dup { color: var(--muted); font-style: italic; }
.ppf-files td.added.pos b { color: var(--green); }
.ppf-files td.added.zero { color: var(--muted); }
.ppf-files td.when { color: var(--muted); white-space: nowrap; }
```

- [ ] **Step 3 : Ajouter la section JS « Annuaire PPF »**

À la fin de `client/src/app.js`, après la ligne 379 (`invoke("directory_status")…`), ajouter :

```javascript
// --- Annuaire PPF (cumulatif, historique des fichiers, onglet Fichiers) -----

/** Recharge le résumé + la table d'historique (via h(), jamais innerHTML). */
function renderPpf() {
  Promise.all([invoke("ppf_summary"), invoke("ppf_files")])
    .then(([sum, files]) => {
      const summary = $("ppf-summary");
      const table = $("ppf-files");
      if (!files.length) {
        summary.className = "muted";
        summary.replaceChildren(document.createTextNode("Aucun fichier chargé."));
        table.classList.add("hidden");
        table.replaceChildren();
        return;
      }
      const plur = sum.file_count > 1 ? "s" : "";
      summary.className = "";
      summary.replaceChildren(
        h("span", { class: "dot" }, "●"),
        " ",
        h("b", {}, sum.distinct_addr.toLocaleString("fr-FR")),
        " adressages en table · ",
        h("b", {}, String(sum.file_count)),
        ` fichier${plur} ingéré${plur}`
      );
      const thead = h("thead", {}, h("tr", {},
        h("th", {}, "Fichier"),
        h("th", { class: "num" }, "Lignes"),
        h("th", { class: "num" }, "Adressages uniques"),
        h("th", { class: "num" }, "Ajoutés"),
        h("th", {}, "Chargé le")
      ));
      const tbody = h("tbody", {});
      for (const f of files) {
        const name = h("td", { class: "name", title: f.file_name }, f.file_name);
        if (f.is_duplicate) name.append(" ", h("span", { class: "ppf-dup" }, "(doublon)"));
        const added = h("td", { class: `num added ${f.added_addr > 0 ? "pos" : "zero"}` });
        if (f.added_addr > 0) added.append(h("b", {}, f.added_addr.toLocaleString("fr-FR")));
        else added.append("0");
        const when = new Date(f.loaded_at * 1000).toLocaleString("fr-FR", {
          day: "2-digit", month: "2-digit", year: "numeric", hour: "2-digit", minute: "2-digit",
        });
        tbody.append(h("tr", {},
          name,
          h("td", { class: "num" }, f.lines.toLocaleString("fr-FR")),
          h("td", { class: "num" }, f.unique_addr.toLocaleString("fr-FR")),
          added,
          h("td", { class: "when" }, when)
        ));
      }
      table.replaceChildren(thead, tbody);
      table.classList.remove("hidden");
    })
    .catch((err) => banner("error", `Annuaire PPF : ${err}`));
}

function setPpfBusy(busy) {
  $("ppf-browse").disabled = busy;
  $("ppf-reset").disabled = busy;
  $("ppf-prog").classList.toggle("hidden", !busy);
  if (!busy) {
    $("ppf-bar").classList.remove("indet");
    $("ppf-bar").firstElementChild.style.width = "0";
  }
}

let ppfBusy = false;

async function loadPpf(path) {
  if (ppfBusy) return;
  ppfBusy = true;
  setPpfBusy(true);
  try {
    await invoke("load_ppf_file", { path });
    renderPpf();
  } catch (err) {
    banner("error", `Annuaire PPF : ${err}`);
  } finally {
    ppfBusy = false;
    setPpfBusy(false);
  }
}

// Progression : phase parse uniquement (barre indéterminée, lignes lues).
listen("ppf://progress", (e) => {
  const bar = $("ppf-bar");
  bar.classList.add("indet");
  bar.firstElementChild.style.width = "";
  $("ppf-prog-text").textContent = "Analyse et chargement en base…";
  $("ppf-prog-num").textContent = `${e.payload.done.toLocaleString("fr-FR")} lignes lues`;
});

$("ppf-browse").addEventListener("click", async (e) => {
  const btn = e.currentTarget;
  btn.disabled = true; // garde de ré-entrance pendant le dialog
  try {
    const f = await open({ multiple: false, filters: [{ name: "CSV", extensions: ["csv", "txt"] }] });
    if (f) await loadPpf(f);
  } finally {
    btn.disabled = false;
  }
});

// Reset : modale de confirmation maison (nœuds DOM, jamais innerHTML).
$("ppf-reset").addEventListener("click", () => {
  invoke("ppf_summary").then((sum) => {
    modal(
      h("h3", {}, "Vider l'annuaire PPF ?"),
      h("p", { class: "muted" },
        "Cette action supprime les ",
        h("b", {}, sum.distinct_addr.toLocaleString("fr-FR")),
        " adressages de la table et l'historique des ",
        h("b", {}, String(sum.file_count)),
        " fichiers ingérés. Les fichiers sur votre disque ne sont pas touchés. Action irréversible."
      ),
      h("div", { class: "modal-btns" },
        h("button", { onclick: closeModal }, "Annuler"),
        h("button", {
          class: "btn-danger",
          onclick: async () => {
            try {
              await invoke("reset_ppf");
              closeModal();
              renderPpf();
            } catch (err) {
              closeModal();
              banner("error", `Annuaire PPF : ${err}`);
            }
          },
        }, "Réinitialiser")
      )
    );
  });
});

const pdz = $("ppf-dropzone");
pdz.addEventListener("dragover", (e) => { e.preventDefault(); pdz.classList.add("over"); });
pdz.addEventListener("dragleave", () => pdz.classList.remove("over"));

// État initial au démarrage.
renderPpf();
```

- [ ] **Step 4 : Ajouter la 3ᵉ cible dans le routeur `tauri://drag-drop`**

Dans `client/src/app.js`, le listener `listen("tauri://drag-drop", …)` (lignes 254-272). Il faut : (a) déclarer `pdz` **avant** le listener — mais `pdz` est défini au Step 3 en fin de fichier, donc pour l'ordre d'exécution, remplacer dans le listener l'usage direct par `$("ppf-dropzone")`. Modifier le corps du listener ainsi (ajouter la remise à zéro de la classe `over` et la branche PPF) :

Remplacer :
```javascript
listen("tauri://drag-drop", (e) => {
  dz.classList.remove("over");
  ddz.classList.remove("over");
  const paths = e.payload.paths || [];
  if (!paths.length || STEPS[current] !== "file") return;
  const pos = e.payload.position || { x: 0, y: 0 };
  const dpr = window.devicePixelRatio || 1;
  const x = pos.x / dpr, y = pos.y / dpr;
  const r = ddz.getBoundingClientRect();
  if (x >= r.left && x <= r.right && y >= r.top && y <= r.bottom) {
    if (!/\.(csv|txt)$/i.test(paths[0])) {
      banner("warn", `Ce fichier n'est pas un CSV (.csv ou .txt attendu) : ${paths[0]}`);
      return;
    }
    loadDirectory("file", paths[0]);
  } else {
    pickInput(paths[0]);
  }
});
```

Par :
```javascript
listen("tauri://drag-drop", (e) => {
  dz.classList.remove("over");
  ddz.classList.remove("over");
  $("ppf-dropzone").classList.remove("over");
  const paths = e.payload.paths || [];
  if (!paths.length || STEPS[current] !== "file") return;
  const pos = e.payload.position || { x: 0, y: 0 };
  const dpr = window.devicePixelRatio || 1;
  const x = pos.x / dpr, y = pos.y / dpr;
  const inside = (el) => {
    const r = el.getBoundingClientRect();
    return x >= r.left && x <= r.right && y >= r.top && y <= r.bottom;
  };
  const csvOk = () => {
    if (/\.(csv|txt)$/i.test(paths[0])) return true;
    banner("warn", `Ce fichier n'est pas un CSV (.csv ou .txt attendu) : ${paths[0]}`);
    return false;
  };
  if (inside(ddz)) {
    if (csvOk()) loadDirectory("file", paths[0]);
  } else if (inside($("ppf-dropzone"))) {
    if (csvOk()) loadPpf(paths[0]);
  } else {
    pickInput(paths[0]);
  }
});
```

- [ ] **Step 5 : Lancer l'app et vérifier de bout en bout**

Run: `npm run tauri dev` (depuis `client/`) — ou via le skill `run` du projet.
Vérifier manuellement, onglet Fichiers :
1. Déposer le fichier exemple `export_b2b_siren_cf_typologie_b2b_prod_17-07-2026.csv` sur la zone PPF → barre de progression, puis une ligne dans la table (lignes/uniques/ajoutés cohérents, ajoutés > 0 en vert).
2. Redéposer le **même** fichier → nouvelle ligne « (doublon) », ajoutés = 0.
3. « Parcourir… » charge aussi un fichier.
4. « Reset… » ouvre la modale ; « Réinitialiser » vide la table et l'historique (« Aucun fichier chargé. »).
5. Le bloc Annuaire Peppol reste inchangé et fonctionnel.

- [ ] **Step 6 : Commit**

```bash
git add client/src/index.html client/src/styles.css client/src/app.js
git commit -m "feat(superpopaul): onglet Fichiers — bloc Annuaire PPF (dépôt cumulatif, historique, reset)"
```

---

## Notes de vérification finale

- `cargo test --quiet` : toute la suite verte.
- `cargo clippy` : pas de nouveau warning (le projet compile sans warning).
- Aucune modification de l'onglet run, du rapport, de `columns.js`/`output.rs` (hors-scope confirmé).
