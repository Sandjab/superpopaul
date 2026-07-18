# Annuaire Peppol — ingestion des participants 0225 — plan d'implémentation

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Charger `export-all-participants.csv` (drop / Parcourir / Télécharger) dans une table `peppol_directory` recréée à chaque chargement, avec horodatage persisté et affiché, filtrée sur l'adressage 0225.

**Architecture:** Module client-only `directory.rs` (fonction pure `parse_0225_value`, stream CSV, download proxy vers fichier temporaire supprimé) → `store.rs` persiste dans `superpopaul.db` (table `peppol_directory` + meta 1-ligne, une transaction) → 3 commandes Tauri async (`spawn_blocking`) → carte UI dans l'onglet Fichiers avec progression via événement `directory://progress`. Ingestion seule : aucun croisement avec la résolution.

**Tech Stack:** Rust (rusqlite bundled, crate `csv`, `reqwest`, `tempfile`, `chrono`), Tauri v2, JS vanilla.

**Spec :** `docs/superpowers/specs/2026-07-18-annuaire-peppol-directory-design.md`

---

## Structure des fichiers

- **Créé** : `client/src-tauri/src/directory.rs` — parsing 0225 (pur), stream CSV, download. Aucune dépendance Tauri (testable).
- **Modifié** : `client/src-tauri/src/store.rs` — table meta au schéma + `replace_peppol_directory` + `peppol_directory_status` + `DirStatus`.
- **Modifié** : `client/src-tauri/src/commands.rs` — `DirProgress`, `DirLoadResult`, 3 commandes.
- **Modifié** : `client/src-tauri/src/lib.rs` — `pub mod directory;` + enregistrement des commandes.
- **Modifié** : `client/src-tauri/Cargo.toml` — `tempfile` de dev-dep → dépendance.
- **Modifié** : `client/src/index.html` — carte Annuaire sous `#dropzone`.
- **Modifié** : `client/src/styles.css` — styles de la carte.
- **Modifié** : `client/src/app.js` — statut au démarrage, Parcourir, Télécharger, progression, routage du drop.

Commande de test Rust (depuis `client/src-tauri/`) : `cargo test <filtre> -- --nocolor`.

---

## Task 1 : `directory.rs` — `parse_0225_value` (fonction pure)

**Files:**
- Create: `client/src-tauri/src/directory.rs`
- Modify: `client/src-tauri/src/lib.rs` (ajouter `pub mod directory;`)

- [ ] **Step 1 : Créer le module avec la constante et la fonction, plus les tests**

Créer `client/src-tauri/src/directory.rs` :

```rust
//! Ingestion de l'annuaire Peppol (fichier export-all-participants.csv) —
//! fonctionnalité CLIENT-ONLY : aucune parité avec cli/popaul.py.
//! On ne charge que l'adressage 0225 (SIRENE français), stocké sans son
//! préfixe de scheme/ICD.

use std::io::Read;

/// Préfixe des Participant ID d'adressage 0225. Le scheme est l'invariant de
/// `pid::DEFAULT_SCHEME` ; le « 0225 » est l'exigence explicite du chantier
/// (test `prefixe_coherent_avec_pid` en garde-fou contre la dérive).
const PREFIX_0225: &str = "iso6523-actorid-upis::0225:";

/// URL d'export de l'annuaire Peppol (Télécharger).
pub const DIRECTORY_URL: &str = "https://directory.peppol.eu/export/participants-csv";

/// Renvoie la valeur (partie après `iso6523-actorid-upis::0225:`) si le
/// Participant ID est en 0225, sinon `None`. Verbatim : les suffixes
/// (`_replyto`, `_cdv_…`, `_SIRET`) sont conservés. Préfixe seul sans valeur → `None`.
pub fn parse_0225_value(participant_id: &str) -> Option<String> {
    match participant_id.trim().strip_prefix(PREFIX_0225) {
        Some(rest) if !rest.is_empty() => Some(rest.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefixe_coherent_avec_pid() {
        // Garde-fou : le préfixe 0225 doit rester aligné sur le scheme par
        // défaut de la canonicalisation.
        assert_eq!(PREFIX_0225, format!("{}::0225:", crate::pid::DEFAULT_SCHEME));
    }

    #[test]
    fn extrait_la_valeur_0225_nue() {
        assert_eq!(
            parse_0225_value("iso6523-actorid-upis::0225:000122308"),
            Some("000122308".to_string())
        );
    }

    #[test]
    fn conserve_les_suffixes_techniques_verbatim() {
        // Les entrées à suffixe (_replyto, _cdv_…) sont de vrais inscrits :
        // on les garde tels quels, on ne normalise pas.
        assert_eq!(
            parse_0225_value("iso6523-actorid-upis::0225:000009777_0054_replyto"),
            Some("000009777_0054_replyto".to_string())
        );
        assert_eq!(
            parse_0225_value("iso6523-actorid-upis::0225:005580436_cdv_d6a4bbca"),
            Some("005580436_cdv_d6a4bbca".to_string())
        );
    }

    #[test]
    fn ignore_les_autres_schemes() {
        assert_eq!(parse_0225_value("iso6523-actorid-upis::0002:000126010"), None);
        assert_eq!(parse_0225_value("iso6523-actorid-upis::0009:552100554"), None);
    }

    #[test]
    fn ignore_le_prefixe_seul_et_l_entete() {
        assert_eq!(parse_0225_value("iso6523-actorid-upis::0225:"), None);
        assert_eq!(parse_0225_value("Participant ID"), None);
        assert_eq!(parse_0225_value(""), None);
    }

    #[test]
    fn trimme_l_entree() {
        assert_eq!(
            parse_0225_value("  iso6523-actorid-upis::0225:000122308  "),
            Some("000122308".to_string())
        );
    }
}
```

- [ ] **Step 2 : Déclarer le module**

Dans `client/src-tauri/src/lib.rs`, ajouter la ligne `pub mod directory;` dans le bloc `pub mod …` (par ordre alphabétique, après `pub mod direct;` ligne 6) :

```rust
pub mod direct;
pub mod directory;
pub mod modes;
```

- [ ] **Step 3 : Lancer les tests — doivent échouer puis passer**

Run (depuis `client/src-tauri/`) : `cargo test directory::tests -- --nocolor`
Expected : les 6 tests `directory::tests::*` PASSENT (le module compile et la logique est correcte). Si un `use super::Read` inutilisé fait un warning, l'ignorer à ce stade (utilisé en Task 2).

- [ ] **Step 4 : Commit**

```bash
git add client/src-tauri/src/directory.rs client/src-tauri/src/lib.rs
git commit -m "feat(superpopaul): parse_0225_value — extraction de l'adressage 0225 de l'annuaire"
```

---

## Task 2 : `directory.rs` — `stream_0225_values` (parsing CSV en flux)

**Files:**
- Modify: `client/src-tauri/src/directory.rs`

- [ ] **Step 1 : Écrire le test d'abord**

Ajouter dans `mod tests` de `directory.rs` :

```rust
    #[test]
    fn stream_ne_garde_que_le_0225_dans_l_ordre() {
        // En-tête + mélange de schemes ; seules les valeurs 0225 ressortent,
        // dans l'ordre de lecture, en-tête ignoré.
        let csv = "\"Participant ID\"\n\
                   \"iso6523-actorid-upis::0002:000126010\"\n\
                   \"iso6523-actorid-upis::0225:000122308\"\n\
                   \"iso6523-actorid-upis::0009:552100554\"\n\
                   \"iso6523-actorid-upis::0225:000009777_0054_replyto\"\n";
        let mut progress_calls = 0u32;
        let vals = stream_0225_values(std::io::Cursor::new(csv), |_| progress_calls += 1).unwrap();
        assert_eq!(vals, vec!["000122308".to_string(), "000009777_0054_replyto".to_string()]);
        assert!(progress_calls >= 1, "on_progress doit être appelé au moins une fois");
    }

    #[test]
    fn stream_csv_vide_ou_entete_seule() {
        let vals = stream_0225_values(std::io::Cursor::new("\"Participant ID\"\n"), |_| {}).unwrap();
        assert!(vals.is_empty());
    }
```

- [ ] **Step 2 : Lancer le test — doit échouer à la compilation**

Run : `cargo test directory::tests::stream_ -- --nocolor`
Expected : FAIL — `cannot find function stream_0225_values`.

- [ ] **Step 3 : Implémenter la fonction**

Ajouter dans `directory.rs` (après `parse_0225_value`) :

```rust
/// Lit un CSV mono-colonne (`Participant ID`) en flux et renvoie les valeurs
/// 0225 dans l'ordre. `on_progress(lignes_lues)` est appelé tous les 100 000
/// enregistrements puis une fois en fin de lecture. BLOQUANT (5,2 M lignes
/// possibles) : appeler depuis `spawn_blocking`.
pub fn stream_0225_values<R: Read>(
    reader: R,
    mut on_progress: impl FnMut(u64),
) -> Result<Vec<String>, String> {
    let mut rdr = csv::ReaderBuilder::new().has_headers(true).from_reader(reader);
    let mut record = csv::StringRecord::new();
    let mut out = Vec::new();
    let mut lines: u64 = 0;
    loop {
        match rdr.read_record(&mut record) {
            Ok(true) => {
                lines += 1;
                if let Some(field) = record.get(0) {
                    if let Some(v) = parse_0225_value(field) {
                        out.push(v);
                    }
                }
                if lines % 100_000 == 0 {
                    on_progress(lines);
                }
            }
            Ok(false) => break,
            Err(e) => return Err(format!("lecture CSV de l'annuaire : {e}")),
        }
    }
    on_progress(lines);
    Ok(out)
}
```

- [ ] **Step 4 : Lancer les tests — doivent passer**

Run : `cargo test directory::tests -- --nocolor`
Expected : PASS (8 tests).

- [ ] **Step 5 : Commit**

```bash
git add client/src-tauri/src/directory.rs
git commit -m "feat(superpopaul): stream_0225_values — parsing CSV annuaire en flux"
```

---

## Task 3 : `store.rs` — persistance de `peppol_directory` + meta

**Files:**
- Modify: `client/src-tauri/src/store.rs`

- [ ] **Step 1 : Écrire les tests d'abord**

Ajouter dans `mod tests` de `store.rs` :

```rust
    #[test]
    fn directory_charge_dedup_et_compte() {
        let s = Store::open_in_memory().unwrap();
        let vals = vec!["000122308".to_string(), "0559".to_string(), "000122308".to_string()];
        let n = s.replace_peppol_directory(&vals, "file", 1000).unwrap();
        assert_eq!(n, 2, "la PK déduplique le doublon");
        let st = s.peppol_directory_status().unwrap().unwrap();
        assert_eq!(st.count, 2);
        assert_eq!(st.loaded_at, 1000);
        assert_eq!(st.source, "file");
    }

    #[test]
    fn directory_est_recreee_a_chaque_chargement() {
        let s = Store::open_in_memory().unwrap();
        s.replace_peppol_directory(&["a".into(), "b".into(), "c".into()], "file", 1).unwrap();
        // Deuxième chargement : contenu entièrement remplacé, pas cumulé.
        let n = s.replace_peppol_directory(&["x".into()], "download", 2).unwrap();
        assert_eq!(n, 1);
        let st = s.peppol_directory_status().unwrap().unwrap();
        assert_eq!(st.count, 1);
        assert_eq!(st.source, "download");
        assert_eq!(st.loaded_at, 2);
    }

    #[test]
    fn directory_status_none_avant_tout_chargement() {
        let s = Store::open_in_memory().unwrap();
        assert!(s.peppol_directory_status().unwrap().is_none());
    }

    #[test]
    fn ouverture_cree_la_table_meta_annuaire() {
        // Une base préexistante sans peppol_directory_meta doit rester
        // ouvrable et gagner la table (migration idempotente).
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sans_meta.db");
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
        assert!(s.peppol_directory_status().unwrap().is_none());
        s.replace_peppol_directory(&["z".into()], "file", 7).unwrap();
        assert_eq!(s.peppol_directory_status().unwrap().unwrap().count, 1);
    }
```

- [ ] **Step 2 : Lancer les tests — doivent échouer**

Run : `cargo test store::tests::directory -- --nocolor`
Expected : FAIL — `no method named replace_peppol_directory` / `peppol_directory_status`.

- [ ] **Step 3 : Ajouter la table meta au schéma**

Dans `store.rs`, remplacer la constante `SCHEMA` (lignes 29-43) par (le bloc `resolutions` est inchangé, on ajoute la table meta) :

```rust
const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS resolutions (
  participant       TEXT PRIMARY KEY,
  exists_in_peppol  INTEGER,
  pa_code           TEXT,
  pa_name           TEXT,
  pa_country        TEXT,
  extended_ctc_fr   INTEGER,
  api_status        TEXT NOT NULL,
  resolved_at       INTEGER NOT NULL,
  note              TEXT,
  ctc_activation    TEXT,
  ctc_expiration    TEXT
);
CREATE TABLE IF NOT EXISTS peppol_directory_meta (
  id         INTEGER PRIMARY KEY CHECK (id = 1),
  loaded_at  INTEGER NOT NULL,
  count      INTEGER NOT NULL,
  source     TEXT NOT NULL
);
";
```

- [ ] **Step 4 : Ajouter le type `DirStatus`**

Dans `store.rs`, après la struct `Resolution` (après la ligne 23, `}`), ajouter :

```rust
/// État du dernier chargement de l'annuaire Peppol (table meta 1-ligne).
#[derive(Debug, Clone, serde::Serialize)]
pub struct DirStatus {
    pub loaded_at: i64,
    pub count: i64,
    pub source: String,
}
```

- [ ] **Step 5 : Ajouter les méthodes au `impl Store`**

Dans `store.rs`, à la fin du bloc `impl Store` (juste avant le `}` qui ferme `impl Store`, avant `fn row_to_resolution` ou après — au choix, dans l'impl), ajouter :

```rust
    /// Recrée entièrement `peppol_directory` (DROP+CREATE) et y insère les
    /// valeurs (INSERT OR IGNORE — la PK déduplique), puis met à jour la meta,
    /// le tout dans UNE transaction : un échec laisse l'ancien contenu intact
    /// et l'horodatage ne peut pas diverger du contenu. Renvoie le nombre de
    /// lignes distinctes réellement en table.
    pub fn replace_peppol_directory(
        &self,
        values: &[String],
        source: &str,
        loaded_at: i64,
    ) -> Result<usize, String> {
        let tx = self.conn.unchecked_transaction().map_err(|e| e.to_string())?;
        tx.execute_batch(
            "DROP TABLE IF EXISTS peppol_directory;
             CREATE TABLE peppol_directory (value TEXT PRIMARY KEY);",
        )
        .map_err(|e| e.to_string())?;
        {
            let mut stmt = tx
                .prepare_cached("INSERT OR IGNORE INTO peppol_directory (value) VALUES (?1)")
                .map_err(|e| e.to_string())?;
            for v in values {
                stmt.execute(params![v]).map_err(|e| e.to_string())?;
            }
        }
        let count: i64 = tx
            .query_row("SELECT COUNT(*) FROM peppol_directory", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        tx.execute(
            "INSERT INTO peppol_directory_meta (id, loaded_at, count, source)
             VALUES (1, ?1, ?2, ?3)
             ON CONFLICT(id) DO UPDATE SET
               loaded_at=excluded.loaded_at, count=excluded.count, source=excluded.source",
            params![loaded_at, count, source],
        )
        .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(count as usize)
    }

    /// État du dernier chargement de l'annuaire ; `None` si jamais chargé.
    pub fn peppol_directory_status(&self) -> Result<Option<DirStatus>, String> {
        self.conn
            .query_row(
                "SELECT loaded_at, count, source FROM peppol_directory_meta WHERE id = 1",
                [],
                |r| {
                    Ok(DirStatus {
                        loaded_at: r.get(0)?,
                        count: r.get(1)?,
                        source: r.get(2)?,
                    })
                },
            )
            .optional()
            .map_err(|e| e.to_string())
    }
```

- [ ] **Step 6 : Lancer les tests — doivent passer**

Run : `cargo test store::tests -- --nocolor`
Expected : PASS (les 4 nouveaux + tous les existants, y compris les migrations).

- [ ] **Step 7 : Commit**

```bash
git add client/src-tauri/src/store.rs
git commit -m "feat(superpopaul): store — table peppol_directory recréée + meta horodatée"
```

---

## Task 4 : `directory.rs` — téléchargement en flux vers fichier temporaire

**Files:**
- Modify: `client/src-tauri/Cargo.toml` (tempfile en dépendance)
- Modify: `client/src-tauri/src/directory.rs`

- [ ] **Step 1 : Promouvoir `tempfile` en dépendance**

Dans `client/src-tauri/Cargo.toml`, ajouter dans `[dependencies]` (après `encoding_rs_io` par ex.) :

```toml
tempfile = "3"                # fichier temporaire du téléchargement annuaire (supprimé après parsing)
```

Puis dans `[dev-dependencies]`, **supprimer** la ligne `tempfile = "3"` devenue redondante (elle reste disponible aux tests via la dépendance normale).

- [ ] **Step 2 : Écrire le test d'abord (wiremock)**

Ajouter dans `mod tests` de `directory.rs` :

```rust
    #[tokio::test]
    async fn download_ecrit_le_corps_et_rapporte_la_progression() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        let body = "\"Participant ID\"\n\"iso6523-actorid-upis::0225:000122308\"\n";
        Mock::given(method("GET"))
            .and(path("/export/participants-csv"))
            .respond_with(ResponseTemplate::new(200).set_body_string(body))
            .mount(&server)
            .await;

        let mut last_done = 0u64;
        let tmp = download_to_temp(
            &format!("{}/export/participants-csv", server.uri()),
            None,
            None,
            |done, _total| last_done = done,
        )
        .await
        .unwrap();

        let content = std::fs::read_to_string(tmp.path()).unwrap();
        assert_eq!(content, body);
        assert_eq!(last_done, body.len() as u64);
    }
```

- [ ] **Step 3 : Lancer le test — doit échouer à la compilation**

Run : `cargo test directory::tests::download -- --nocolor`
Expected : FAIL — `cannot find function download_to_temp`.

- [ ] **Step 4 : Implémenter le téléchargement**

Ajouter dans `directory.rs` (le proxy est appliqué comme dans `api.rs`) :

```rust
/// Télécharge l'annuaire (streaming, chunk par chunk) dans un fichier
/// temporaire supprimé au Drop — le brut 214 Mo n'est jamais conservé.
/// `on_progress(octets_reçus, content_length)` alimente la barre.
/// Honore le proxy configuré (même construction que `api.rs`).
pub async fn download_to_temp(
    url: &str,
    proxy_url: Option<&str>,
    creds: Option<&crate::api::ProxyCreds>,
    mut on_progress: impl FnMut(u64, Option<u64>),
) -> Result<tempfile::NamedTempFile, String> {
    use std::io::Write;
    let mut b = reqwest::Client::builder();
    if let Some(purl) = proxy_url {
        let mut p = reqwest::Proxy::all(purl).map_err(|e| format!("proxy : {e}"))?;
        if let Some(c) = creds {
            p = p.basic_auth(&c.username, &c.password);
        }
        b = b.proxy(p);
    }
    let client = b.build().map_err(|e| e.to_string())?;
    let mut resp = client.get(url).send().await.map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("téléchargement de l'annuaire : HTTP {}", resp.status().as_u16()));
    }
    let total = resp.content_length();
    let mut tmp = tempfile::NamedTempFile::new().map_err(|e| e.to_string())?;
    let mut done: u64 = 0;
    while let Some(chunk) = resp.chunk().await.map_err(|e| e.to_string())? {
        tmp.write_all(&chunk).map_err(|e| e.to_string())?;
        done += chunk.len() as u64;
        on_progress(done, total);
    }
    tmp.flush().map_err(|e| e.to_string())?;
    Ok(tmp)
}
```

- [ ] **Step 5 : Lancer les tests — doivent passer**

Run : `cargo test directory:: -- --nocolor`
Expected : PASS (9 tests, dont le download wiremock).

- [ ] **Step 6 : Commit**

```bash
git add client/src-tauri/Cargo.toml client/src-tauri/Cargo.lock client/src-tauri/src/directory.rs
git commit -m "feat(superpopaul): download_to_temp — téléchargement annuaire en flux, proxy honoré"
```

---

## Task 5 : `commands.rs` — 3 commandes Tauri + enregistrement

**Files:**
- Modify: `client/src-tauri/src/commands.rs`
- Modify: `client/src-tauri/src/lib.rs`

> Note : ces commandes sont de l'orchestration (glue Tauri/AppState) — la logique
> testable est déjà couverte (Task 1-4). Vérification = compilation (`cargo build`)
> puis test manuel end-to-end (Task 8). Pas de test unitaire (nécessiterait un
> harnais Tauri complet) — signalé explicitement, pas contourné.

- [ ] **Step 1 : Ajouter les types et les commandes**

Dans `commands.rs`, à la fin du fichier, ajouter :

```rust
/// Progression émise pendant le chargement de l'annuaire.
/// phase = "download" (done/total en octets) | "parse" (done = lignes, total = None).
#[derive(Clone, Serialize)]
pub struct DirProgress {
    pub phase: &'static str,
    pub done: u64,
    pub total: Option<u64>,
}

#[derive(Serialize)]
pub struct DirLoadResult {
    pub loaded_at: i64,
    pub count: usize,
}

#[tauri::command]
pub fn directory_status(state: State<'_, AppState>) -> Result<Option<crate::store::DirStatus>, String> {
    state.store.lock().unwrap().peppol_directory_status()
}

/// Charge un fichier annuaire local (drop / Parcourir). Parsing bloquant hors
/// executor ; progression phase "parse" émise sur `directory://progress`.
#[tauri::command]
pub async fn load_directory_file(
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
) -> Result<DirLoadResult, String> {
    let store = state.store.clone();
    tokio::task::spawn_blocking(move || {
        let reader = std::io::BufReader::new(std::fs::File::open(&path).map_err(|e| e.to_string())?);
        let values = crate::directory::stream_0225_values(reader, |lines| {
            let _ = app.emit(
                "directory://progress",
                DirProgress { phase: "parse", done: lines, total: None },
            );
        })?;
        let loaded_at = chrono::Utc::now().timestamp();
        let count = store
            .lock()
            .unwrap()
            .replace_peppol_directory(&values, "file", loaded_at)?;
        Ok(DirLoadResult { loaded_at, count })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Télécharge l'annuaire puis le charge. Progression phase "download" pendant
/// le transfert, puis "parse" pendant l'analyse. Le temporaire est supprimé.
#[tauri::command]
pub async fn download_directory(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<DirLoadResult, String> {
    // Proxy éventuel — la config peut être absente (aucun run configuré).
    let (proxy, creds) = {
        let cfg = state.config.lock().unwrap().clone();
        let proxy = cfg.as_ref().and_then(|c| c.api.proxy.as_ref()).map(|p| p.url.clone());
        let creds = state.proxy_creds.lock().unwrap().clone();
        (proxy, creds)
    };
    let app_dl = app.clone();
    let tmp = crate::directory::download_to_temp(
        crate::directory::DIRECTORY_URL,
        proxy.as_deref(),
        creds.as_ref(),
        move |done, total| {
            let _ = app_dl.emit(
                "directory://progress",
                DirProgress { phase: "download", done, total },
            );
        },
    )
    .await?;
    let path = tmp.path().to_path_buf();
    let store = state.store.clone();
    let result = tokio::task::spawn_blocking(move || {
        let reader = std::io::BufReader::new(std::fs::File::open(&path).map_err(|e| e.to_string())?);
        let values = crate::directory::stream_0225_values(reader, |lines| {
            let _ = app.emit(
                "directory://progress",
                DirProgress { phase: "parse", done: lines, total: None },
            );
        })?;
        let loaded_at = chrono::Utc::now().timestamp();
        let count = store
            .lock()
            .unwrap()
            .replace_peppol_directory(&values, "download", loaded_at)?;
        Ok::<_, String>(DirLoadResult { loaded_at, count })
    })
    .await
    .map_err(|e| e.to_string())?;
    drop(tmp); // suppression du temporaire (214 Mo) après parsing
    result
}
```

- [ ] **Step 2 : Enregistrer les commandes**

Dans `client/src-tauri/src/lib.rs`, dans `tauri::generate_handler![…]` (lignes 61-82), ajouter à la fin de la liste (après `commands::export_report`, avec une virgule avant) :

```rust
            commands::export_report,
            commands::directory_status,
            commands::load_directory_file,
            commands::download_directory
```

- [ ] **Step 3 : Compiler**

Run (depuis `client/src-tauri/`) : `cargo build`
Expected : compilation OK, aucun warning sur les nouveaux symboles.

- [ ] **Step 4 : Lancer toute la suite de tests Rust**

Run : `cargo test -- --nocolor`
Expected : PASS (aucune régression).

- [ ] **Step 5 : Commit**

```bash
git add client/src-tauri/src/commands.rs client/src-tauri/src/lib.rs
git commit -m "feat(superpopaul): commandes annuaire (statut/charger/télécharger) + progression"
```

---

## Task 6 : Frontend — carte Annuaire (HTML + CSS)

**Files:**
- Modify: `client/src/index.html`
- Modify: `client/src/styles.css`

- [ ] **Step 1 : Ajouter la carte dans l'onglet Fichiers**

Dans `client/src/index.html`, dans `<section id="step-file">`, après le bloc `<div id="file-info" …>…</div>` (ligne 32) et avant `</section>` (ligne 33), insérer :

```html
      <div id="dir-card">
        <div class="dir-head">
          <h3>Annuaire Peppol <span class="chip-ref">référence</span></h3>
          <span class="muted">Déclaratif : un participant listé ici n'est pas forcément provisionné dans le réseau — et inversement.</span>
        </div>
        <div id="dir-dropzone">
          <span>Dépose <code>export-all-participants.csv</code> ici</span>
          <span class="dir-spacer"></span>
          <button id="dir-browse">Parcourir…</button>
          <button id="dir-download" class="btn-primary">⤓ Télécharger</button>
        </div>
        <p id="dir-status" class="muted">Jamais chargé.</p>
        <div id="dir-prog" class="hidden">
          <div class="dir-prog-lbl"><span id="dir-prog-text"></span><span id="dir-prog-num"></span></div>
          <div id="dir-bar" class="dir-bar"><span></span></div>
        </div>
      </div>
```

- [ ] **Step 2 : Ajouter les styles**

Dans `client/src/styles.css`, à la fin du fichier, ajouter (variables et conventions reprises de l'existant) :

```css
/* --- Carte Annuaire Peppol (onglet Fichiers) --- */
#dir-card { margin-top: 18px; border: 1px solid var(--border); background: var(--card);
            border-radius: 12px; padding: 16px 18px; }
.dir-head { display: flex; flex-direction: column; gap: 3px; margin-bottom: 12px; }
.dir-head h3 { font-size: 15px; margin: 0; display: flex; align-items: center; gap: 8px; }
.chip-ref { font-size: 11px; font-weight: 600; letter-spacing: .3px; text-transform: uppercase;
            color: var(--gold); border: 1px solid var(--gold); border-radius: 20px; padding: 1px 9px; }
#dir-dropzone { border: 2px dashed var(--border); border-radius: 10px; padding: 16px 14px;
                display: flex; align-items: center; flex-wrap: wrap; gap: 10px; color: var(--muted); }
#dir-dropzone.over { border-color: var(--gold); color: var(--fg); }
.dir-spacer { flex: 1; min-width: 8px; }
#dir-status { margin: 12px 0 0; font-size: 13px; }
#dir-status b { color: var(--fg); }
#dir-status.empty b { color: var(--amber); }
#dir-status .dot { color: var(--green); }
#dir-prog { margin: 12px 0 0; }
.dir-prog-lbl { font-size: 13px; margin-bottom: 6px; display: flex; justify-content: space-between; color: var(--muted); }
.dir-bar { height: 8px; border-radius: 6px; background: var(--track); overflow: hidden; }
.dir-bar > span { display: block; height: 100%; width: 0; background: var(--gold); }
.dir-bar.indet > span { width: 40%; animation: dir-slide 1.2s ease-in-out infinite; }
@keyframes dir-slide { 0% { margin-left: -40%; } 100% { margin-left: 100%; } }
```

- [ ] **Step 3 : Vérification visuelle**

Lancer le client (`cargo tauri dev` depuis `client/src-tauri/`, ou la méthode projet).
Expected : sous la dropzone principale, la carte « Annuaire Peppol » apparaît avec la puce « référence », la zone de dépôt, les boutons Parcourir…/⤓ Télécharger, et « Jamais chargé. ». (Le câblage vient en Task 7 — les boutons ne font encore rien.)

- [ ] **Step 4 : Commit**

```bash
git add client/src/index.html client/src/styles.css
git commit -m "feat(superpopaul): carte Annuaire Peppol dans l'onglet Fichiers (HTML/CSS)"
```

---

## Task 7 : Frontend — câblage (statut, Parcourir, Télécharger, progression, routage du drop)

**Files:**
- Modify: `client/src/app.js`

- [ ] **Step 1 : Ajouter le bloc annuaire**

Dans `client/src/app.js`, après le bloc du drag-drop de l'étape 1 (après la ligne 252, le `listen("tauri://drag-drop", …)` existant), insérer :

```js
// --- Annuaire Peppol (référence déclarative, onglet Fichiers) ---------------
const ddz = $("dir-dropzone");

/** Rend la ligne d'état à partir d'un DirStatus (ou null = jamais chargé).
 *  Données via textContent uniquement (le compteur vient du backend, mais on
 *  ne fait jamais confiance à une entrée dérivée d'un CSV). */
function renderDirStatus(st) {
  const el = $("dir-status");
  el.textContent = "";
  if (!st) {
    el.className = "muted empty";
    el.append(
      h("b", {}, "Jamais chargé."),
      " Téléchargez l'annuaire ou déposez le CSV pour peupler la base."
    );
    return;
  }
  const when = new Date(st.loaded_at * 1000).toLocaleString("fr-FR", {
    day: "2-digit", month: "2-digit", year: "numeric", hour: "2-digit", minute: "2-digit",
  });
  const origine = st.source === "download" ? "téléchargé" : "depuis le fichier";
  el.className = "muted";
  el.append(
    h("span", { class: "dot" }, "●"),
    " Dernier chargement : ",
    h("b", {}, when),
    " — ",
    h("b", {}, st.count.toLocaleString("fr-FR")),
    ` adressages 0225 (${origine}).`
  );
}

/** Active/désactive les contrôles et affiche/masque la barre de progression. */
function setDirBusy(busy) {
  $("dir-browse").disabled = busy;
  $("dir-download").disabled = busy;
  $("dir-prog").classList.toggle("hidden", !busy);
  if (!busy) {
    $("dir-bar").classList.remove("indet");
    $("dir-bar").firstElementChild.style.width = "0";
  }
}

async function loadDirectory(kind, arg) {
  setDirBusy(true);
  $("dir-status").classList.add("hidden");
  try {
    const r = kind === "download"
      ? await invoke("download_directory")
      : await invoke("load_directory_file", { path: arg });
    renderDirStatus({ loaded_at: r.loaded_at, count: r.count, source: kind === "download" ? "download" : "file" });
  } catch (err) {
    banner("error", `Annuaire Peppol : ${err}`);
  } finally {
    setDirBusy(false);
    $("dir-status").classList.remove("hidden");
  }
}

// Progression : phase "download" (octets, barre en %) puis "parse" (lignes, indéterminée).
listen("directory://progress", (e) => {
  const { phase, done, total } = e.payload;
  const bar = $("dir-bar");
  if (phase === "download") {
    bar.classList.remove("indet");
    const mo = (n) => (n / 1048576).toFixed(0);
    if (total) {
      const pct = Math.round((done / total) * 100);
      bar.firstElementChild.style.width = pct + "%";
      $("dir-prog-text").textContent = "Téléchargement de l'annuaire…";
      $("dir-prog-num").textContent = `${mo(done)} Mo / ${mo(total)} Mo · ${pct} %`;
    } else {
      bar.classList.add("indet");
      $("dir-prog-text").textContent = "Téléchargement de l'annuaire…";
      $("dir-prog-num").textContent = `${mo(done)} Mo`;
    }
  } else {
    bar.classList.add("indet");
    $("dir-prog-text").textContent = "Analyse et chargement en base…";
    $("dir-prog-num").textContent = `${done.toLocaleString("fr-FR")} lignes lues`;
  }
});

$("dir-browse").addEventListener("click", async () => {
  const f = await open({ multiple: false, filters: [{ name: "CSV", extensions: ["csv", "txt"] }] });
  if (f) await loadDirectory("file", f);
});
$("dir-download").addEventListener("click", () => loadDirectory("download"));

ddz.addEventListener("dragover", (e) => { e.preventDefault(); ddz.classList.add("over"); });
ddz.addEventListener("dragleave", () => ddz.classList.remove("over"));

// Statut initial au démarrage.
invoke("directory_status").then(renderDirStatus).catch(() => {});
```

- [ ] **Step 2 : Router le drop vers la bonne zone**

Dans `client/src/app.js`, remplacer le listener `tauri://drag-drop` existant (lignes 248-252) par :

```js
// Le drop de fichier natif arrive par l'événement Tauri drag-drop. Deux cibles
// dans l'étape Fichiers : on route selon la position (px physiques → CSS).
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
    loadDirectory("file", paths[0]);
  } else {
    pickInput(paths[0]);
  }
});
```

- [ ] **Step 3 : Vérification manuelle du câblage (petit fichier)**

Lancer le client. Créer un petit CSV de test (quelques lignes 0225 + autres schemes) et :
- cliquer **Parcourir…** de la carte → sélectionner le fichier → la ligne d'état passe à « Dernier chargement : … — N adressages 0225 (depuis le fichier) » ;
- **glisser-déposer** le même fichier sur la zone annuaire → même résultat (et un dépôt sur la dropzone principale charge toujours le fichier principal) ;
- relancer l'app → la ligne d'état est restaurée depuis la base (persistance).

Expected : les trois comportements OK, aucune erreur console.

- [ ] **Step 4 : Commit**

```bash
git add client/src/app.js
git commit -m "feat(superpopaul): câblage annuaire — statut, Parcourir, Télécharger, progression, routage drop"
```

---

## Task 8 : Vérification end-to-end (fichier réel)

**Files:** aucun (vérification).

- [ ] **Step 1 : Charger le fichier réel par Parcourir**

Lancer le client. Via **Parcourir…** de la carte Annuaire, ouvrir
`../deaddrop/in/export-all-participants.csv` (214 Mo, 5,2 M lignes).
Expected : barre « Analyse et chargement en base… » avec compteur de lignes qui progresse, puis ligne d'état « Dernier chargement : `<date/heure>` — **1 542 342** adressages 0225 (depuis le fichier). » (à quelques unités près selon l'export du jour).

- [ ] **Step 2 : Vérifier la base**

Depuis un terminal, inspecter la base (`superpopaul.db`, dossier de données de l'app ; en dev macOS : `~/Library/Application Support/<bundle-id>/superpopaul.db`) :

Run : `sqlite3 "<chemin>/superpopaul.db" "SELECT count, source, loaded_at FROM peppol_directory_meta; SELECT COUNT(*) FROM peppol_directory; SELECT value FROM peppol_directory LIMIT 3;"`
Expected : `count` = `SELECT COUNT(*)` = ~1 542 342 ; `source` = `file` ; 3 valeurs de type `000122308` / `..._replyto`.

- [ ] **Step 3 : Vérifier la recréation**

Recharger le même fichier (ou un extrait plus court). Expected : `count` reflète le NOUVEAU chargement (pas de cumul), horodatage mis à jour.

- [ ] **Step 4 : (optionnel) Télécharger**

Cliquer **⤓ Télécharger** (réseau requis, ~214 Mo). Expected : barre de téléchargement en %, puis analyse, puis état « (téléchargé) ». Aucun fichier de 214 Mo laissé dans le dossier de données (temporaire supprimé).

- [ ] **Step 5 : Commit éventuel de finition**

Si des ajustements ont été nécessaires, commit dédié. Sinon, rien à committer.

---

## Auto-revue

- **Couverture du spec** : table `peppol_directory(value PK)` + meta une transaction (Task 3) ; `parse_0225_value` verbatim (Task 1) ; stream CSV (Task 2) ; download temp supprimé + proxy (Task 4) ; 3 commandes + progression (Task 5) ; carte UI + états + routage drop (Task 6-7) ; horodatage affiché/persisté (Task 3/7) ; vérif fichier réel (Task 8). Hors périmètre (croisement résolution, index SIREN, conservation brut) non implémenté — conforme.
- **Placeholders** : aucun — chaque step porte le code réel ou une commande exacte.
- **Cohérence des types** : `DirStatus{loaded_at:i64,count:i64,source:String}` (store) ↔ `directory_status` renvoie `Option<store::DirStatus>` ↔ `renderDirStatus(st)` lit `loaded_at/count/source`. `DirLoadResult{loaded_at:i64,count:usize}` ↔ `loadDirectory` lit `r.loaded_at/r.count`. `DirProgress{phase,done,total}` ↔ listener lit `phase/done/total`. `stream_0225_values(reader, on_progress)` et `replace_peppol_directory(values,source,loaded_at)` appelées avec la même signature en Task 5. `parse_0225_value` / `PREFIX_0225` cohérents Task 1-2. OK.
