# Refonte profils & onglets Fichiers/Format — plan d'implémentation

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Profils v1 sans chemin de fichier (hash de colonnes + format de sortie), onglet 1 réduit à un hub de dépôt, onglet 2 unifiant désignation d'adressage, mapping, encodage/séparateur et boutons profil.

**Architecture:** Rust d'abord en TDD (hash `csv_io`, `Profile` v1 dans `config.rs`, purge des chemins relatifs dans `commands.rs`) — tout vérifiable par `cargo test` sans UI. Puis frontend vanilla (HTML/CSS/JS, pas de bundler) : la maquette a été validée écran par écran (spec `docs/superpowers/specs/2026-07-18-refonte-profils-onglets-design.md`).

**Tech Stack:** Rust (Tauri, serde_yaml, csv, encoding_rs), frontend vanilla + SortableJS vendorisé. Tests : `cargo test` dans `client/src-tauri/`.

**Conventions à respecter (CLAUDE.md projet) :** messages d'erreur et texte UI en français ; JAMAIS d'innerHTML avec données dynamiques (helper `h()` / `textContent`) ; commits `feat(superpopaul): …` ; l'UI n'a aucune logique métier.

---

### Task 1 : Hash de signature des colonnes (`csv_io::columns_hash`)

**Files:**
- Modify: `client/src-tauri/src/csv_io.rs` (fonction + tests dans le module `tests` en bas de fichier)

Le hash est une **valeur persistée** dans les profils : FNV-1a 64 bits écrit maison (stable entre versions de Rust, contrairement à `DefaultHasher` ; pas de dépendance crypto pour un contrôle de compatibilité). Octets UTF-8, chaque en-tête préfixé par sa longueur, ordre et casse significatifs, pas de trim, hex minuscule 16 caractères.

- [ ] **Step 1 : Écrire le test qui échoue**

Dans le module `tests` de `client/src-tauri/src/csv_io.rs` :

```rust
#[test]
fn columns_hash_stable_ordre_casse_et_non_ambigu() {
    let h = |names: &[&str]| {
        columns_hash(&names.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    };
    // Valeur en dur : le hash est PERSISTÉ dans les profils — si cette
    // assertion casse, l'algorithme a changé et tous les profils existants
    // deviennent incompatibles. Ne jamais « corriger » la valeur attendue.
    assert_eq!(h(&["SIREN", "RAISON_SOCIALE", "VILLE"]), "ec46ac4b9e99375d");
    // L'ordre des colonnes est significatif.
    assert_ne!(
        h(&["SIREN", "RAISON_SOCIALE", "VILLE"]),
        h(&["VILLE", "RAISON_SOCIALE", "SIREN"])
    );
    // La casse est significative (la résolution des colonnes l'est aussi).
    assert_ne!(
        h(&["SIREN", "RAISON_SOCIALE", "VILLE"]),
        h(&["siren", "raison_sociale", "ville"])
    );
    // Préfixage par longueur : pas d'ambiguïté de concaténation.
    assert_ne!(h(&["ab", "c"]), h(&["a", "bc"]));
}
```

- [ ] **Step 2 : Vérifier l'échec**

Run : `cd client/src-tauri && cargo test columns_hash`
Attendu : erreur de compilation `cannot find function columns_hash`.

- [ ] **Step 3 : Implémentation minimale**

Dans `csv_io.rs`, après `suggest_pid_column` :

```rust
fn fnv1a(mut h: u64, bytes: &[u8]) -> u64 {
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Signature des en-têtes d'entrée : FNV-1a 64 bits sur les octets UTF-8,
/// chaque en-tête préfixé par sa longueur (8 octets little-endian). Ordre et
/// casse significatifs, aucune normalisation. Valeur PERSISTÉE dans les
/// profils : l'algorithme ne doit jamais changer (test avec valeur en dur).
pub fn columns_hash(headers: &[String]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for name in headers {
        h = fnv1a(h, &(name.len() as u64).to_le_bytes());
        h = fnv1a(h, name.as_bytes());
    }
    format!("{h:016x}")
}
```

- [ ] **Step 4 : Vérifier le succès**

Run : `cargo test columns_hash`
Attendu : `test csv_io::tests::columns_hash_stable_ordre_casse_et_non_ambigu ... ok`

- [ ] **Step 5 : Commit**

```bash
git add client/src-tauri/src/csv_io.rs
git commit -m "feat(superpopaul): hash de signature des colonnes d'entrée (FNV-1a 64, csv_io)"
```

---

### Task 2 : `Preview` expose le hash et la taille du fichier

**Files:**
- Modify: `client/src-tauri/src/csv_io.rs` (struct `Preview` l.13-19, fn `preview` l.96-116, tests)

Le payload de `preview_csv` (`commands.rs::PreviewPayload`, qui `#[serde(flatten)]` le `Preview`) transporte ainsi le hash et la taille jusqu'à l'UI sans autre changement. La taille sert à la ligne méta du hub (« 1 248 Ko · séparateur … »).

- [ ] **Step 1 : Écrire le test qui échoue**

Dans le module `tests` de `csv_io.rs` :

```rust
#[test]
fn preview_expose_hash_des_colonnes_et_taille() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("x.csv");
    std::fs::write(&p, "a;b\n1;2\n").unwrap();
    let prev = preview(&p, 5).unwrap();
    assert_eq!(prev.columns_hash, columns_hash(&prev.headers));
    assert_eq!(prev.columns_hash, "41c80da72d0aec94"); // ["a", "b"], valeur en dur
    assert_eq!(prev.size_bytes, 8); // "a;b\n1;2\n"
}
```

Si `tempfile` n'est pas encore importé dans ce module de tests, l'utiliser en chemin complet (`tempfile::tempdir()`) — c'est déjà une dev-dependency (utilisée par les tests de `config.rs`).

- [ ] **Step 2 : Vérifier l'échec**

Run : `cargo test preview_expose`
Attendu : erreur de compilation `no field columns_hash on type Preview`.

- [ ] **Step 3 : Implémentation**

Struct `Preview` (l.13-19) :

```rust
#[derive(Debug, Clone, Serialize)]
pub struct Preview {
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
    pub delimiter: String,
    pub encoding: String,
    /// Signature des en-têtes (columns_hash) — comparée à celle des profils.
    pub columns_hash: String,
    pub size_bytes: u64,
}
```

Dans `preview()` (l.96-116), avant le `Ok(Preview { … })` :

```rust
    let size_bytes = std::fs::metadata(path)
        .map_err(|e| format!("métadonnées {path:?} : {e}"))?
        .len();
    let hash = columns_hash(&headers);
    Ok(Preview {
        headers,
        rows,
        delimiter: (meta.delimiter as char).to_string(),
        encoding: meta.encoding.to_string(),
        columns_hash: hash,
        size_bytes,
    })
```

(`headers` est déjà un `Vec<String>` ; calculer `hash` avant le move dans la struct.)

- [ ] **Step 4 : Vérifier le succès**

Run : `cargo test` (complet — d'autres tests construisent des `Preview` littéraux, p. ex. `suggest_pid_column` l.247-266 : leur ajouter `columns_hash: String::new(), size_bytes: 0`).
Attendu : tout vert.

- [ ] **Step 5 : Commit**

```bash
git add client/src-tauri/src/csv_io.rs
git commit -m "feat(superpopaul): preview expose la signature des colonnes et la taille du fichier"
```

---

### Task 3 : `Profile` v1 — sans chemin, avec hash et format de sortie

**Files:**
- Modify: `client/src-tauri/src/config.rs` (section « Profils » l.302-369, fixture `profile_exemple` l.676-685, tests l.687-742)

Rupture assumée, pas de migration : le fallback ancien-format et le booléen `legacy` disparaissent. `deny_unknown_fields` rejette naturellement les anciens profils (qui portent `input.path`).

- [ ] **Step 1 : Réécrire la fixture et les tests (qui échouent)**

Remplacer `profile_exemple()` (l.676-685) par :

```rust
    fn profile_exemple() -> Profile {
        Profile {
            version: 1,
            input: ProfileInput {
                pid_column: "siren".into(),
                columns_hash: "ec46ac4b9e99375d".into(),
            },
            output: ProfileOutput {
                encoding: OutputEncoding::Utf8Bom,
                separator: OutputSeparator::Auto,
            },
            columns: config_exemple().output.columns,
        }
    }
```

(`config_exemple().output.columns` contient `Input { name: "siren" }` + deux champs Peppol — l'invariant « pid en sortie » est satisfait.)

Remplacer les tests `profil_aller_retour_et_champ_inconnu_rejete` (l.687-704), `profil_depuis_yaml_ancien_format_complet` (l.706-716) et `profil_rejette_colonnes_vides_et_pid_manquant` (l.734-742) par :

```rust
    #[test]
    fn profil_aller_retour_et_champ_inconnu_rejete() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("clients.profil.yaml");
        save_profile_file(&p, &profile_exemple()).unwrap();
        let back = load_profile_file(&p).unwrap();
        assert_eq!(back.input.pid_column, "siren");
        assert_eq!(back.input.columns_hash, "ec46ac4b9e99375d");
        assert_eq!(back.output.encoding, OutputEncoding::Utf8Bom);
        assert_eq!(back.columns, profile_exemple().columns);
        // Ni clé API, ni réglages, ni chemin de fichier dans le YAML.
        let yaml = std::fs::read_to_string(&p).unwrap();
        assert!(!yaml.contains("key") && !yaml.contains("api"));
        assert!(!yaml.contains("path"), "le profil ne porte plus de chemin");
        // Typo : rejet net (deny_unknown_fields, plus de fallback à aspirer).
        let bad = yaml.replace("pid_column:", "pid_colum:");
        assert!(profile_from_yaml(&bad).is_err());
    }

    #[test]
    fn profil_anciens_formats_rejetes_sans_migration() {
        // Ancienne config complète et ancien profil (avec input.path) :
        // rejet net, l'utilisateur recrée ses fichiers — pas de migration.
        assert!(profile_from_yaml(yaml_ancien()).is_err());
        let ancien_profil = "version: 1\n\
                             input:\n  path: ./a.csv\n  pid_column: siren\n\
                             columns:\n  - source: input\n    name: siren\n";
        assert!(profile_from_yaml(ancien_profil).is_err());
    }

    #[test]
    fn profil_exige_pid_hash_et_pid_en_sortie() {
        let mut p = profile_exemple();
        p.input.pid_column.clear();
        assert!(p.validate().is_err());

        let mut p = profile_exemple();
        p.input.columns_hash.clear();
        assert!(p.validate().is_err());

        // La colonne d'adressage est obligatoire en sortie (une sortie sans
        // la clé est injoignable) — subsume « au moins une colonne ».
        let mut p = profile_exemple();
        p.columns
            .retain(|c| !matches!(c, ColumnSpec::Input { name } if name == "siren"));
        assert!(p.validate().is_err());
        let mut p = profile_exemple();
        p.columns.clear();
        assert!(p.validate().is_err());
    }
```

Adapter `champs_peppol_anciens_noms_lus_via_alias` (l.718-732) : `let (back, _) = profile_from_yaml(&ancien).unwrap();` devient `let back = profile_from_yaml(&ancien).unwrap();`.

- [ ] **Step 2 : Vérifier l'échec**

Run : `cargo test profil`
Attendu : erreurs de compilation (`ProfileInput` n'a pas de champ `columns_hash`, `ProfileOutput` inconnu, arité de retour de `profile_from_yaml`).

- [ ] **Step 3 : Implémentation**

Remplacer la section « Profils de chargement » (l.302-369) par :

```rust
// --- Profils de chargement (sauvegarde/chargement explicites) -----------------
// Ce qui décrit COMMENT parser l'entrée et générer la sortie : colonne des
// adressages, signature des colonnes d'entrée, forme de sortie, mapping.
// Jamais le fichier lui-même (le profil s'applique à tout fichier de même
// signature), ni la clé API, ni les réglages.

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub version: u32,
    pub input: ProfileInput,
    pub output: ProfileOutput,
    pub columns: Vec<ColumnSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileInput {
    pub pid_column: String,
    /// Signature des en-têtes du fichier d'entrée (csv_io::columns_hash) —
    /// un profil ne s'applique qu'à un fichier de même signature.
    pub columns_hash: String,
}

/// La forme de la sortie portée par le profil (encodage, séparateur) — le
/// reste de la forme (dossier, suffixe, horodatage) vit dans les réglages ⚙.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileOutput {
    #[serde(default)]
    pub encoding: OutputEncoding,
    #[serde(default)]
    pub separator: OutputSeparator,
}

impl Profile {
    pub fn validate(&self) -> Result<(), String> {
        if self.input.pid_column.is_empty() {
            return Err("le profil doit indiquer la colonne des adressages".into());
        }
        if self.input.columns_hash.is_empty() {
            return Err("le profil doit porter la signature des colonnes d'entrée".into());
        }
        // La colonne d'adressage est obligatoire en sortie : une sortie sans
        // la clé est injoignable. Subsume « au moins une colonne ».
        let pid_en_sortie = self.columns.iter().any(
            |c| matches!(c, ColumnSpec::Input { name } if name == &self.input.pid_column),
        );
        if !pid_en_sortie {
            return Err("le profil doit inclure la colonne des adressages en sortie".into());
        }
        Ok(())
    }
}

/// Lit un profil v1. Les anciens formats (profil avec chemin, config
/// complète) sont rejetés : pas de migration, l'utilisateur recrée.
pub fn profile_from_yaml(s: &str) -> Result<Profile, String> {
    let p: Profile = serde_yaml::from_str(s).map_err(|e| format!("profil : {e}"))?;
    p.validate()?;
    Ok(p)
}

pub fn save_profile_file(path: &Path, p: &Profile) -> Result<(), String> {
    p.validate()?;
    atomic_write(path, &serde_yaml::to_string(p).map_err(|e| e.to_string())?)
}

pub fn load_profile_file(path: &Path) -> Result<Profile, String> {
    let s = std::fs::read_to_string(path).map_err(|e| format!("lecture {path:?} : {e}"))?;
    profile_from_yaml(&s).map_err(|e| format!("{path:?} : {e}"))
}
```

**Attention :** `commands.rs` ne compile plus (tuple `(profile, legacy)`) — correction complète en Task 5 ; pour garder ce commit vert, appliquer dès maintenant le correctif minimal dans `commands.rs` :
- `load_profile` : `let (profile, legacy) = …` → `let profile = …` et `Ok(ProfileLoad { profile, legacy: false })` (le champ `legacy` disparaît en Task 5).

- [ ] **Step 4 : Vérifier le succès**

Run : `cargo test`
Attendu : tout vert (dont les 3 nouveaux tests profil).

- [ ] **Step 5 : Commit**

```bash
git add client/src-tauri/src/config.rs client/src-tauri/src/commands.rs
git commit -m "feat(superpopaul): profil v1 sans chemin — signature de colonnes, sortie, invariant adressage"
```

---

### Task 4 : Réglages sans encodage/séparateur

**Files:**
- Modify: `client/src-tauri/src/config.rs` (struct `OutputSettings` l.261-274, fixture `settings_exemple` l.609-622, tests)

- [ ] **Step 1 : Écrire le test qui échoue**

Dans le module `tests` de `config.rs`, après `settings_fichier_aller_retour_absent_et_corrompu` :

```rust
    #[test]
    fn reglages_anciens_avec_encodage_rejetes() {
        // encodage/séparateur ont déménagé dans les profils : un
        // superpopaul.yaml d'avant la refonte est rejeté avec une erreur
        // claire au démarrage (montrée, pas avalée), l'utilisateur recrée.
        let mut yaml = serde_yaml::to_string(&settings_exemple()).unwrap();
        // `output` est le dernier bloc du YAML : l'ajout indenté y atterrit.
        yaml.push_str("  encoding: utf-8-bom\n");
        assert!(serde_yaml::from_str::<Settings>(&yaml).is_err());
    }
```

- [ ] **Step 2 : Vérifier l'échec**

Run : `cargo test reglages_anciens`
Attendu : FAIL — `OutputSettings` accepte encore `encoding`, le parse réussit donc l'assertion `is_err()` échoue.

- [ ] **Step 3 : Implémentation**

Dans `OutputSettings` (l.261-274) : supprimer les deux champs `encoding` et `separator` (et leurs `#[serde(default)]`). Dans `settings_exemple()` (l.609-622) : supprimer les deux lignes `encoding:` / `separator:`.

- [ ] **Step 4 : Vérifier le succès**

Run : `cargo test`
Attendu : tout vert.

- [ ] **Step 5 : Commit**

```bash
git add client/src-tauri/src/config.rs
git commit -m "feat(superpopaul): réglages sans encodage/séparateur de sortie (déménagés dans les profils)"
```

---

### Task 5 : Purge des chemins relatifs et de `ProfileLoad`

**Files:**
- Modify: `client/src-tauri/src/commands.rs` (l.17-35, 56-72, 144-177, 179-184)
- Modify: `client/src-tauri/src/lib.rs` (enregistrement `resolved_input_path`)
- Modify: `client/src-tauri/src/config.rs` (`resolve_relative` l.426-438 env., test `chemins_resolus_relativement_au_yaml` l.849-858)

Tout ce mécanisme n'existait que parce que le profil portait un chemin. Le fichier d'entrée est désormais toujours celui choisi à l'onglet Fichiers (`cfg.input.path`, absolu).

- [ ] **Step 1 : Simplifier `AppState` et les helpers**

Dans `commands.rs` :
- Supprimer le champ `base` de `AppState` (l.21-24) et son initialisation dans `new()` (l.47).
- `current_config` (l.56-64) ne retourne plus que la config :

```rust
    fn current_config(&self) -> Result<Config, String> {
        self.config
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| String::from("Aucune configuration active."))
    }

    fn input_path(&self) -> Result<PathBuf, String> {
        Ok(PathBuf::from(&self.current_config()?.input.path))
    }
```

- Dans `client()` (l.74-90) : `let (_, cfg) = self.current_config()?;` → `let cfg = self.current_config()?;`. Corriger de même tout autre appelant qui déstructure le tuple.
- Supprimer la struct `ProfileLoad` (l.144-150) et la commande `resolved_input_path` (l.179-184).
- `load_profile` / `save_profile` (l.159-177) deviennent :

```rust
#[tauri::command]
pub fn load_profile(path: String) -> Result<config::Profile, String> {
    config::load_profile_file(Path::new(&path))
}

#[tauri::command]
pub fn save_profile(path: String, profile: config::Profile) -> Result<(), String> {
    config::save_profile_file(Path::new(&path), &profile)
}
```

- [ ] **Step 2 : Purger `lib.rs` et `config.rs`**

- `lib.rs` : retirer la ligne `commands::resolved_input_path,` du `generate_handler!`.
- `config.rs` : supprimer `resolve_relative` (l.426-438 env.) et le test `chemins_resolus_relativement_au_yaml`.

- [ ] **Step 3 : Vérifier**

Run : `cargo test` puis `grep -rn "resolve_relative\|resolved_input_path\|ProfileLoad" client/src-tauri/src client/src`
Attendu : tests verts ; le grep ne trouve plus que l'appel `resolved_input_path` d'`app.js` (retiré en Task 7 — le front n'est pas encore migré, l'app desktop ne doit pas être lancée entre les Tasks 5 et 7).

- [ ] **Step 4 : Commit**

```bash
git add client/src-tauri/src/commands.rs client/src-tauri/src/lib.rs client/src-tauri/src/config.rs
git commit -m "feat(superpopaul): purge des chemins relatifs de profil (base, resolved_input_path, ProfileLoad)"
```

---

### Task 6 : HTML & CSS — onglets Fichiers/Format, accents de typologie

**Files:**
- Modify: `client/src/index.html` (header l.11-22, step-file l.27-37, step-columns l.39-47, panneau ⚙ l.142-155)
- Modify: `client/src/styles.css` (tokens `:root` l.5-10, règles `#preview-table th.pid-col` l.110, nouvelles règles)

Pas de test front (convention projet) : la vérification est visuelle en Task 9. Après cette task, l'app est transitoirement incohérente avec `app.js` — c'est attendu, Tasks 6-8 forment un tout committé séparément pour la lisibilité des diffs.

- [ ] **Step 1 : Restructurer `index.html`**

Header (l.11-21) — le stepper est renommé, les boutons profil quittent le header :

```html
    <nav id="stepper">
      <button data-step="file"    class="step active">1. Fichiers</button>
      <button data-step="columns" class="step" disabled>2. Format</button>
      <button data-step="run"     class="step" disabled>3. Run</button>
    </nav>
    <span class="cfg-btns">
      <button id="btn-settings" class="btn-ghost" title="Réglages — appliqués et enregistrés à la fermeture.">⚙</button>
    </span>
```

Étape 1 (l.27-37) — hub : le sélecteur d'adressage part à l'étape 2, l'aperçu reste (lecture seule) :

```html
    <!-- Étape 1 : hub de dépôt (fichier principal ; companions plus tard) -->
    <section id="step-file" class="panel">
      <h2>Fichiers</h2>
      <div id="dropzone">Dépose le fichier principal (CSV) ici, ou <button id="btn-browse">Parcourir…</button></div>
      <div id="file-info" class="hidden">
        <p id="file-meta" class="muted"></p>
        <table id="preview-table"></table>
      </div>
    </section>
```

Étape 2 (l.39-47) — format complet : titre + boutons profil, désignation, tableau, zone, ligne sortie :

```html
    <!-- Étape 2 : format d'entrée/sortie du fichier principal -->
    <section id="step-columns" class="panel hidden">
      <div id="format-head">
        <h2>Format du fichier principal</h2>
        <span>
          <button id="btn-load-cfg" title="Charger un profil YAML — appliqué si ses colonnes correspondent au fichier ouvert.">Charger profil…</button>
          <button id="btn-save-cfg" title="Sauvegarder le format courant (colonne des adressages, colonnes, encodage, séparateur) en YAML.">Sauvegarder…</button>
        </span>
      </div>
      <p><label for="pid-column">Colonne des adressages :</label>
         <select id="pid-column" title="Colonne contenant les adressages Peppol à résoudre (ex. SIREN/SIRET) — 🔑 dans le tableau."></select>
         <span id="pid-hint" class="muted"></span></p>
      <p class="muted">Glisse les en-têtes : réordonne-les dans le tableau, écarte-les vers la
         zone du bas, réintègre-les où tu veux. Double-clic : écarte une colonne, ou la
         réintègre en dernière position. L'aperçu montre le résultat final.</p>
      <table id="out-preview"></table>
      <div id="col-zone"></div>
      <p id="out-format">Fichier de sortie :
        <label for="out-encoding">Encodage</label>
        <select id="out-encoding" title="UTF-8 avec BOM garantit les accents dans Excel FR.">
          <option value="utf-8-bom">UTF-8 avec BOM (Excel FR)</option>
          <option value="utf-8">UTF-8 sans BOM</option>
          <option value="windows-1252">Windows-1252</option>
        </select>
        <label for="out-sep">Séparateur</label>
        <select id="out-sep" title="« Identique à l'entrée » reprend le séparateur détecté au dépôt.">
          <option value="auto">Identique à l'entrée</option>
          <option value=";">Point-virgule ;</option>
          <option value=",">Virgule ,</option>
          <option value="|">Pipe |</option>
          <option value="&#9;">Tabulation</option>
        </select></p>
    </section>
```

Panneau ⚙ (l.142-155) : supprimer le `<p>` entier contenant les selects `out-encoding`/`out-sep` (ils viennent d'être déplacés ci-dessus — un id ne doit exister qu'une fois).

- [ ] **Step 2 : Styles**

Dans `styles.css` :
- Token (l.8, à la suite des couleurs existantes) : ajouter `--pid: #e8e4d8;` (écru lumineux — accent de la colonne d'adressage, distinction par luminosité + 🔑, validé en maquette).
- Remplacer la règle `#preview-table th.pid-col { color: var(--gold); border-color: var(--gold); }` (l.110) par `#preview-table th.pid-col { color: var(--pid); border-color: var(--pid); }` (continuité visuelle hub ↔ format ; l'or reste réservé à Peppol).
- Ajouter :

```css
#format-head { display: flex; align-items: center; justify-content: space-between; }
#out-preview th.pid, #out-preview td.pid { color: var(--pid); border-color: var(--pid); }
.key-btn { background: none; border: none; padding: 0 2px; cursor: pointer; opacity: 0; }
.key-btn:hover:not(:disabled) { border: none; }
#out-preview th.input:hover .key-btn { opacity: .45; }
#out-preview th.pid .key-btn { opacity: 1; cursor: default; }
```

- [ ] **Step 3 : Commit**

```bash
git add client/src/index.html client/src/styles.css
git commit -m "feat(superpopaul): onglets Fichiers/Format — hub de dépôt, écran format unifié, accent écru 🔑"
```

---

### Task 7 : `app.js` — hub, gating, désignation, profils

**Files:**
- Modify: `client/src/app.js` (validateStep l.70-82, showStep l.43-59, pickInput/renderFilePanel l.107-195, réglages l.206-320, profils l.647-726)

- [ ] **Step 1 : Gating et navigation**

`validateStep()` (l.70-82) — la désignation se vérifie désormais à l'étape Format :

```js
function validateStep() {
  const s = STEPS[current];
  if (s === "file" && !state.inputPath) return "Choisis d'abord un fichier CSV.";
  if (s === "columns" && !state.config.input.pid_column)
    return "Désigne la colonne des adressages (🔑).";
  // La clé API (mode api) est vérifiée au lancement du run (cockpit.js),
  // les réglages n'étant plus une étape du wizard.
  return null;
}
```

(La garde « au moins une colonne » disparaît : l'invariant « adressage obligatoire en sortie » la subsume.)

Dans `showStep()` (l.57), l'entrée dans Format rafraîchit ses trois zones :

```js
  if (STEPS[i] === "columns") { renderPidSelect(); fillOutFormat(); renderOutPreview(); }
```

- [ ] **Step 2 : Étape 1 — `renderFilePanel` réduit au hub**

Remplacer `renderFilePanel()` (l.148-171) — le sélecteur pid part dans `renderPidSelect()`, la méta gagne la taille :

```js
function renderFilePanel() {
  const p = state.preview;
  syncNextBtn(); // un fichier vient d'être chargé : « Suivant » devient utile
  $("file-info").classList.remove("hidden");
  const meta = $("file-meta");
  meta.replaceChildren(
    h("b", {}, state.inputPath.split(/[\\/]/).pop() ?? ""),
    ` — ${Math.round(p.size_bytes / 1024)} Ko · séparateur « ${p.delimiter} », encodage ${p.encoding}`);
  meta.title = state.inputPath;
  $("preview-table").replaceChildren(
    h("tr", {}, ...p.headers.map((hd) => h("th", {}, hd))),
    ...p.rows.map((r) => h("tr", {}, ...r.map((c) => h("td", {}, c)))),
  );
  highlightPidColumn();
}
```

Ajouter à la suite (nouvelle fonction — la liste et la clé 🔑 de columns.js passent toutes deux par `designatePid`, un seul état) :

```js
/** Liste de désignation de l'étape Format — miroir de state…pid_column.
 *  Sans désignation (aucune suggestion) : placeholder « — choisir — ». */
function renderPidSelect() {
  const headers = state.preview ? state.preview.headers : [];
  const opts = headers.map((hd) => {
    const o = h("option", {}, hd);
    o.selected = hd === state.config.input.pid_column;
    return o;
  });
  if (!state.config.input.pid_column) {
    const ph = h("option", { value: "" }, "— choisir —");
    ph.selected = true;
    ph.disabled = true;
    opts.unshift(ph);
  }
  $("pid-column").replaceChildren(...opts);
  $("pid-hint").textContent =
    state.preview && state.preview.suggested_pid_column != null
      ? "(suggestion automatique)" : "";
  // Un profil sans désignation serait invalide : sauvegarde grisée.
  $("btn-save-cfg").disabled = !state.config.input.pid_column;
}

/** Désignation — LE point d'entrée unique (liste ou clé 🔑 du tableau).
 *  La colonne désignée est obligatoire en sortie : si elle était écartée,
 *  elle est réintégrée d'office ; l'ancienne redevient écartable. */
function designatePid(name) {
  state.config.input.pid_column = name;
  const cols = state.config.output.columns;
  if (!cols.some((c) => c.source === "input" && c.name === name))
    cols.push({ source: "input", name });
  renderPidSelect();
  renderOutPreview();
  highlightPidColumn();
}
```

Le listener du select (l.192-195) devient :

```js
$("pid-column").addEventListener("change", (e) => designatePid(e.target.value));
```

`pickInput` (l.107-146) : inchangé sauf le libellé du filtre déjà en place — la suggestion continue d'alimenter `pid_column` (pré-désignation d'office). `highlightPidColumn` (l.175-180) : inchangé.

- [ ] **Step 3 : Ligne sortie de l'étape Format**

Après la section réglages (vers l.305), ajouter les liaisons des selects déplacés :

```js
// --- Étape Format : forme de sortie (encodage, séparateur) --------------------
function fillOutFormat() {
  $("out-encoding").value = state.config.output.encoding;
  $("out-sep").value = state.config.output.separator;
}
$("out-encoding").addEventListener("change", (e) => { state.config.output.encoding = e.target.value; });
$("out-sep").addEventListener("change", (e) => { state.config.output.separator = e.target.value; });
```

Et purger les réglages ⚙ de ces champs :
- `syncSettingsForm()` (l.212-213) : supprimer les deux lignes `c.output.encoding = …` / `c.output.separator = …`.
- `fillSettingsForm()` (l.244-245) : supprimer les deux lignes `$("out-encoding").value = …` / `$("out-sep").value = …`.
- `currentSettings()` (l.309-314) : la tranche sortie devient `const { dir, suffix, timestamp_suffix } = c.output;` et `output: { dir, suffix, timestamp_suffix }`.

(`applySettings` reste un `Object.assign` : les réglages chargés ne portent plus encodage/séparateur, les valeurs par défaut de `state.config.output` survivent.)

- [ ] **Step 4 : Profils — sauvegarde/chargement avec refus sec**

Remplacer intégralement les deux handlers (l.658-726) — plus de `resolved_input_path`, plus de saut vers Run, plus de branche « fichier introuvable » :

```js
$("btn-save-cfg").addEventListener("click", async () => {
  const f = await save({ filters: [{ name: "YAML", extensions: ["yaml", "yml"] }],
                         ...(await profileDialogDefault()) });
  if (!f) return;
  try {
    await invoke("save_profile", { path: f, profile: {
      version: 1,
      input: { pid_column: state.config.input.pid_column,
               columns_hash: state.preview.columns_hash },
      output: { encoding: state.config.output.encoding,
                separator: state.config.output.separator },
      columns: state.config.output.columns,
    } });
    hideBanner();
  } catch (e) {
    banner("error", `${e}`);
  }
});

$("btn-load-cfg").addEventListener("click", async () => {
  const f = await open({ multiple: false, filters: [{ name: "YAML", extensions: ["yaml", "yml"] }],
                         ...(await profileDialogDefault()) });
  if (!f) return;
  let p;
  try {
    p = await invoke("load_profile", { path: f });
  } catch (e) {
    banner("error", `Chargement impossible : ${e}`);
    return;
  }
  // Refus sec : un profil forcé sur d'autres colonnes produirait une sortie
  // silencieusement fausse. Aucun état modifié.
  if (p.input.columns_hash !== state.preview.columns_hash) {
    banner("error", "Profil incompatible avec le fichier ouvert — colonnes différentes.");
    return;
  }
  state.config.input.pid_column = p.input.pid_column;
  state.config.output.columns = p.columns;
  state.config.output.encoding = p.output.encoding;
  state.config.output.separator = p.output.separator;
  hideBanner();
  renderPidSelect();
  fillOutFormat();
  renderOutPreview();
  highlightPidColumn();
});
```

(`profileDialogDefault` l.653-656 est conservé tel quel — le mode portable garde son defaultPath.)

- [ ] **Step 5 : Vérification statique et commit**

Run : `grep -n "resolved_input_path\|out-encoding\|out-sep" client/src/app.js`
Attendu : plus aucune référence à `resolved_input_path` ; `out-encoding`/`out-sep` seulement dans le bloc « Étape Format ».

```bash
git add client/src/app.js
git commit -m "feat(superpopaul): app.js — hub, désignation unifiée, profils par signature avec refus sec"
```

---

### Task 8 : `columns.js` — clé 🔑, accent dérivé, gardes d'écartement

**Files:**
- Modify: `client/src/columns.js` (makeHeader l.43-48, makeCell l.52-60, Sortable l.127-145, dblclick l.152-161)

La colonne désignée reste une colonne `input` ordinaire : l'accent et les gardes sont **dérivés** en comparant le nom à `state.config.input.pid_column` (aucun nouveau type de colonne, pas d'état invalide possible).

- [ ] **Step 1 : En-têtes et cellules**

Remplacer `makeHeader` (l.43-48) et `makeCell` (l.52-60) :

```js
const isPidSpec = (c) =>
  c.source === "input" && c.name === state.config.input.pid_column;

function makeHeader(c) {
  const pid = isPidSpec(c);
  const attrs = { class: pid ? "input pid" : c.source, "data-key": colKey(c) };
  if (c.source === "peppol")
    attrs.title = "Champ calculé par l'API Peppol — les valeurs affichées sont un exemple.";
  if (pid)
    attrs.title = "Colonne des adressages — obligatoire en sortie, non écartable.";
  const th = h("th", attrs, "⠿ ");
  if (c.source === "input") {
    const k = h("button", {
      class: "key-btn",
      title: pid ? "Colonne des adressages"
                 : "Désigner comme colonne des adressages",
    }, "🔑");
    if (!pid) k.addEventListener("click", () => designatePid(c.name)); // app.js
    else k.disabled = true;
    th.append(k, " ");
  }
  th.append(colLabel(c));
  return th;
}

// Cellule du corps pour la colonne c et la ligne r du preview. `temp` marque
// une colonne matérialisée pendant un drag entrant (fond bleuté).
function makeCell(c, r, temp) {
  const key = colKey(c);
  if (c.source === "peppol")
    return h("td", { class: temp ? "muted temp" : "muted", "data-key": key },
      PEPPOL_SAMPLE[c.field]);
  const idx = state.preview.headers.indexOf(c.name);
  const cls = [temp ? "temp" : "", isPidSpec(c) ? "pid" : ""].join(" ").trim();
  return h("td", { class: cls, "data-key": key },
    idx >= 0 ? (r[idx] ?? "") : "");
}
```

- [ ] **Step 2 : Gardes de drag et de double-clic**

Dans `renderOutPreview()` (l.137-144), la garde « minimum 1 colonne » devient « la 🔑 ne part pas » (l'invariant garantit ≥ 1 colonne), et le clic sur la clé ne doit pas démarrer un drag :

```js
  sortHead = new Sortable(head, {
    ...common,
    filter: ".key-btn",       // le clic 🔑 désigne, il ne drague pas
    preventOnFilter: false,   // laisser le click natif partir
    // Garde : la colonne des adressages est obligatoire en sortie — son
    // en-tête refuse de partir vers la zone d'écartement.
    group: { name: "columns",
             pull: (_to, _from, dragEl) => !dragEl.classList.contains("pid"),
             put: true },
  });
```

Le raccourci double-clic (l.152-161) troque sa garde « dernière colonne » contre la garde 🔑 :

```js
$("out-preview").addEventListener("dblclick", (e) => {
  const th = e.target.closest("th[data-key]");
  if (!th) return;
  const cols = state.config.output.columns;
  const i = cols.findIndex((c) => colKey(c) === th.dataset.key);
  if (i < 0 || isPidSpec(cols[i])) return; // la 🔑 ne s'écarte pas
  cols.splice(i, 1);
  renderOutPreview();
});
```

(`renderColZone` est inchangé : la colonne désignée étant toujours dans `columns`, elle n'apparaît jamais en chip.)

- [ ] **Step 3 : Commit**

```bash
git add client/src/columns.js
git commit -m "feat(superpopaul): columns.js — désignation 🔑 au survol, accent dérivé, gardes d'écartement"
```

---

### Task 9 : Vérification de bout en bout

**Files:** aucun (vérification) — corrections éventuelles au fil de l'eau.

- [ ] **Step 1 : Tests Rust complets**

Run : `cd client/src-tauri && cargo test`
Attendu : tout vert, zéro test ignoré.

- [ ] **Step 2 : Symboles morts**

Run : `grep -rn "resolve_relative\|resolved_input_path\|ProfileLoad\|legacy" client/src-tauri/src client/src --include='*.rs' --include='*.js'`
Attendu : aucune occurrence (hors éventuels commentaires historiques à nettoyer s'ils décrivent du code disparu).

- [ ] **Step 3 : Vérification manuelle de l'app**

Lancer l'app (`cargo tauri dev` depuis `client/src-tauri/`, ou la commande de dev habituelle du projet) et dérouler :

1. **Hub** : déposer un CSV → méta « nom — N Ko · séparateur, encodage » + aperçu lecture seule ; onglet « 2. Format » s'active ; redéposer un autre fichier remplace.
2. **Format** : la colonne suggérée arrive pré-désignée (accent écru + 🔑, liste synchronisée) ; au survol d'un autre en-tête d'entrée la clé fantôme apparaît ; clic → la désignation bascule (liste suivie) ; drag de la colonne 🔑 vers la zone → refusé ; double-clic sur la 🔑 → refusé ; écarter une colonne ordinaire puis la désigner via la liste → elle se réintègre.
3. **Sortie** : changer encodage/séparateur sur la ligne du bas ; ouvrir ⚙ → les champs n'y sont plus.
4. **Profils** : « Sauvegarder… » grisé si aucune désignation (tester via un CSV sans colonne suggérable, p. ex. en-têtes `a;b`) ; sauvegarder un profil, inspecter le YAML (pas de `path`, hash présent, encodage/séparateur présents) ; recharger le profil sur le même fichier → appliqué ; le charger sur un CSV à colonnes différentes → bannière « Profil incompatible… », état intact.
5. **Run** : onglet 3 verrouillé sans désignation ; avec désignation, lancer un petit run de bout en bout (mode direct ou API) → la sortie respecte l'encodage/séparateur choisis à l'étape Format.

- [ ] **Step 4 : Commit final (corrections éventuelles)**

```bash
git add -A && git commit -m "fix(superpopaul): ajustements de la vérification de bout en bout"
```

(Seulement s'il y a eu des corrections ; sinon rien à committer.)
