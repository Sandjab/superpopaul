# Super Popaul 🍿 — Plan d'implémentation

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Application desktop Tauri 2 (Windows/macOS, ≤20 Mo) qui résout en masse des adressages Peppol depuis un CSV via l'API REST existante, avec cache SQLite, config YAML et cockpit temps réel.

**Architecture:** Backend Rust (modules étanches : `pid`, `config`, `store`, `csv_io`, `api`, `telemetry`, `resolver`, `output`, `commands`) + UI vanilla HTML/CSS/JS dans la webview système. L'UI n'affiche que ce que Rust lui envoie (commandes `invoke` + événements). La base SQLite est la source de vérité ; le CSV de sortie est une projection.

**Tech Stack:** Tauri 2, tokio, reqwest (rustls), rusqlite (bundled), serde_yaml, csv, encoding_rs(_io), chardetng ; tests : wiremock, tempfile.

**Spec de référence :** `docs/superpowers/specs/2026-07-12-super-popaul-design.md` (décisions actées : wizard linéaire, cockpit sombre, mapping par aperçu manipulable, base globale, credentials proxy jamais persistés).

**Réponse API (vérifiée dans `peppol_api.py` / `popaul.py`) :**
- `POST /resolve/batch`, body `{"participants": [...], "test": false}`, header `X-API-Key` → `{"results": [{"participant_id", "exists", "pa": {"code","name","country"}, "supports_extended_ctc_fr", "note"} | {"participant", "error"}]}`
- `GET /resolve/<pid>` → un item au même format (mode unitaire = `batch_size: 1` utilise quand même POST batch avec 1 élément : un seul chemin de code).
- `GET /health` → public (test de connectivité).
- Erreurs : 401 clé invalide, 429 + `Retry-After` (secondes), 503 serveur saturé.

---

## Prérequis (une fois)

```bash
rustc --version            # >= 1.77 ; sinon: rustup update
cargo tauri --version      # >= 2 ; sinon: cargo install tauri-cli --locked
```

macOS : Xcode CLT (`xcode-select --install` si besoin).

## Structure des fichiers

```
superpopaul/
├── README.md               # présentation, build, distribution
├── CLAUDE.md               # conventions du sous-projet
├── .gitignore              # target/, *.db
├── src/                    # frontend statique (frontendDist, pas de bundler)
│   ├── index.html          # coquille : splash inline, wizard 4 étapes, cockpit
│   ├── styles.css          # thème sombre cockpit (palette maquettes validées)
│   ├── app.js              # état global, navigation wizard, config load/save
│   ├── columns.js          # étape 2 : aperçu manipulable des colonnes
│   └── cockpit.js          # étape Run : ring, tuiles, graphes, bannières
└── src-tauri/
    ├── Cargo.toml
    ├── tauri.conf.json
    ├── capabilities/default.json
    ├── icons/              # générés (placeholder puis définitif)
    └── src/
        ├── main.rs         # appelle superpopaul_lib::run()
        ├── lib.rs          # Builder Tauri, état, enregistrement commandes
        ├── pid.rs          # canonicalisation + dédoublonnage (parité popaul.py)
        ├── config.rs       # YAML (jamais de credentials proxy sérialisés)
        ├── store.rs        # SQLite globale (résolutions)
        ├── modes.rs        # compute_todo : full / reprise / refresh
        ├── csv_io.rs       # sniff (séparateur+encodage), preview, lecture colonne
        ├── api.rs          # client HTTP (proxy, erreurs typées)
        ├── telemetry.rs    # compteurs, latences, débits, snapshot
        ├── resolver.rs     # moteur : workers, pause/stop, AIMD, breaker, calibrage
        ├── output.rs       # génération CSV de sortie (jointure entrée × base)
        └── commands.rs     # commandes Tauri + événements
```

Chaque module Rust est ajouté à `lib.rs` (`pub mod x;`) par la tâche qui le crée.

---

## Phase 1 — Noyau Rust (testable sans UI)

### Task 1 : Scaffold du sous-projet

**Files:**
- Create: `superpopaul/README.md`, `superpopaul/CLAUDE.md`, `superpopaul/.gitignore`
- Create: `superpopaul/src/index.html` (placeholder), `superpopaul/src-tauri/*` (via `cargo tauri init`)

- [ ] **Step 1 : Créer le répertoire et les fichiers de doc**

`superpopaul/README.md` :

```markdown
# Super Popaul 🍿

Application graphique standalone (Windows + macOS) de résolution Peppol en masse :
un CSV d'adressages en entrée, un CSV enrichi en sortie (existe dans Peppol,
code PA, pays PA, support EXTENDED-CTC-FR), via l'API REST `peppol_api`
(la même que le client CLI `popaul.py` à la racine du repo).

## Points clés
- **Wizard 4 étapes** : fichier d'entrée → colonnes de sortie → sortie & API → run.
- **Cache SQLite global** (dossier données utilisateur) : chaque adressage unique
  est résolu une fois ; modes **full / reprise / refresh** (seuil d'ancienneté).
- **Config YAML** sauvegardable/rechargeable (chemins relatifs au YAML).
  La clé API y est stockée (avertissement UI) ; les identifiants proxy **jamais**.
- **Cockpit temps réel** : ring de progression + ETA, % Peppol, % CTC-FR,
  débits (req/s et adressages/s), codes HTTP, latences p50/p90/p99.
- **Pause/reprise** à chaud et entre sessions (détection de run incomplet).
- Erreurs intelligentes : 401 → suspension + ressaisie de clé ; 429 → backoff
  adaptatif (AIMD) ; 5xx en rafale → circuit breaker avec re-test automatique.

## Développement

```bash
cd superpopaul/src-tauri
cargo test          # logique métier (aucune UI requise)
cargo tauri dev     # app en mode dev
cargo tauri build   # binaire de distribution
```

## Distribution
Binaires **non signés** : voir `NOTICE-OUVERTURE.md` (Gatekeeper : clic droit
> Ouvrir ; SmartScreen : Informations complémentaires > Exécuter quand même).
macOS : build local. Windows : GitHub Actions (`.github/workflows/superpopaul-windows.yml`).

## Spec & plan
- Spec : `../docs/superpowers/specs/2026-07-12-super-popaul-design.md`
- Plan : `../docs/superpowers/plans/2026-07-12-super-popaul.md`
```

`superpopaul/CLAUDE.md` :

```markdown
# Super Popaul — conventions du sous-projet

- Sous-projet **autonome** : ne dépend d'aucun module Python du repo parent.
  La parité de comportement avec `popaul.py` (canonicalisation, format API)
  est vérifiée par tests, pas par import.
- Rust : modules étanches (`pid`, `config`, `store`, `modes`, `csv_io`, `api`,
  `telemetry`, `resolver`, `output`, `commands`). Toute logique métier est
  testable sans UI (`cargo test` dans `src-tauri/`).
- Frontend : vanilla HTML/CSS/JS, **pas de bundler ni de framework**.
  L'UI n'a aucune logique métier : elle invoque des commandes et affiche
  des événements.
- Sécurité UI : **jamais d'innerHTML avec des données dynamiques** (contenu
  CSV, messages d'erreur backend) — construire le DOM via le helper `h()`
  de `app.js` ou `textContent`. Un CSV est une entrée non fiable.
- Sécurité : les identifiants proxy ne sont JAMAIS écrits sur disque
  (test `config::proxy_creds_never_serialized` — ne pas le contourner).
- Texte UI et messages d'erreur en **français**.
- TDD : test d'abord pour toute logique Rust. Commits fréquents,
  format `feat(superpopaul): …` / `fix(superpopaul): …`.
```

`superpopaul/.gitignore` :

```
src-tauri/target/
*.db
```

- [ ] **Step 2 : Frontend placeholder puis scaffold Tauri**

```bash
mkdir -p superpopaul/src
cat > superpopaul/src/index.html <<'EOF'
<!doctype html>
<html lang="fr"><head><meta charset="utf-8"><title>Super Popaul</title></head>
<body><h1>Super Popaul 🍿</h1></body></html>
EOF
cd superpopaul && cargo tauri init --ci
```

`--ci` prend les défauts sans questions ; on réécrit ensuite la conf.

- [ ] **Step 3 : Remplacer `superpopaul/src-tauri/tauri.conf.json`**

```json
{
  "$schema": "https://schema.tauri.app/config/2",
  "productName": "Super Popaul",
  "version": "0.1.0",
  "identifier": "cloud.gavini.superpopaul",
  "build": {
    "frontendDist": "../src"
  },
  "app": {
    "withGlobalTauri": true,
    "windows": [
      {
        "title": "Super Popaul",
        "width": 1000,
        "height": 700,
        "minWidth": 860,
        "minHeight": 600,
        "center": true
      }
    ],
    "security": { "csp": null }
  },
  "bundle": {
    "active": true,
    "targets": "all",
    "icon": ["icons/32x32.png", "icons/128x128.png", "icons/128x128@2x.png", "icons/icon.icns", "icons/icon.ico"]
  }
}
```

(Le splash est un overlay dans `index.html` — pas de seconde fenêtre : plus simple et le démarrage Tauri est quasi instantané ; l'overlay couvre l'ouverture de la base et le chargement de config.)

- [ ] **Step 4 : Remplacer `superpopaul/src-tauri/Cargo.toml`**

```toml
[package]
name = "superpopaul"
version = "0.1.0"
edition = "2021"

[lib]
name = "superpopaul_lib"
crate-type = ["staticlib", "cdylib", "rlib"]

[build-dependencies]
tauri-build = { version = "2", features = [] }

[dependencies]
tauri = { version = "2", features = [] }
tauri-plugin-dialog = "2"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
serde_yaml = "0.9"
csv = "1.3"
rusqlite = { version = "0.32", features = ["bundled"] }
tokio = { version = "1", features = ["rt-multi-thread", "macros", "sync", "time"] }
reqwest = { version = "0.12", default-features = false, features = ["json", "rustls-tls"] }
encoding_rs = "0.8"
encoding_rs_io = "0.1"
chardetng = "0.1"
thiserror = "1"

[dev-dependencies]
wiremock = "0.6"
tempfile = "3"

[profile.release]
strip = true
lto = true
codegen-units = 1
panic = "abort"
```

Et `superpopaul/src-tauri/src/main.rs` :

```rust
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    superpopaul_lib::run();
}
```

Et `superpopaul/src-tauri/src/lib.rs` (version initiale, enrichie tâche par tâche) :

```rust
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .run(tauri::generate_context!())
        .expect("erreur au lancement de Super Popaul");
}
```

- [ ] **Step 5 : Vérifier la compilation**

```bash
cd superpopaul/src-tauri && cargo check
```

Attendu : `Finished` sans erreur (premier build long : compilation de Tauri).

- [ ] **Step 6 : Commit**

```bash
git add superpopaul
git commit -m "feat(superpopaul): scaffold Tauri 2 + README + CLAUDE.md"
```

---

### Task 2 : `pid.rs` — canonicalisation et dédoublonnage

**Files:**
- Create: `superpopaul/src-tauri/src/pid.rs`
- Modify: `superpopaul/src-tauri/src/lib.rs` (ajout `pub mod pid;`)

Parité exacte avec `popaul.py::canonical` : trim, et préfixe `iso6523-actorid-upis::` si le PID ne contient pas `::`.

- [ ] **Step 1 : Écrire les tests (dans `pid.rs`, module `#[cfg(test)]`)**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_ajoute_le_scheme_par_defaut() {
        assert_eq!(canonical("0009:552100554"), "iso6523-actorid-upis::0009:552100554");
    }

    #[test]
    fn canonical_conserve_un_pid_deja_complet() {
        assert_eq!(
            canonical("iso6523-actorid-upis::0009:552100554"),
            "iso6523-actorid-upis::0009:552100554"
        );
    }

    #[test]
    fn canonical_trimme() {
        assert_eq!(canonical("  0009:1  "), "iso6523-actorid-upis::0009:1");
    }

    #[test]
    fn unique_canonical_deduplique_en_gardant_l_ordre() {
        let vals = ["0009:1", "iso6523-actorid-upis::0009:1", "", "0009:2", "0009:1"]
            .map(String::from);
        assert_eq!(
            unique_canonical(vals),
            vec![
                "iso6523-actorid-upis::0009:1".to_string(),
                "iso6523-actorid-upis::0009:2".to_string()
            ]
        );
    }
}
```

- [ ] **Step 2 : Vérifier que ça échoue**

Run: `cd superpopaul/src-tauri && cargo test pid` — Attendu : erreur de compilation (`canonical` non défini).

- [ ] **Step 3 : Implémenter**

```rust
use std::collections::HashSet;

pub const DEFAULT_SCHEME: &str = "iso6523-actorid-upis";

/// Forme canonique du participant_id, identique à popaul.py : ajoute le
/// scheme par défaut si le PID est donné en forme courte "0009:x".
pub fn canonical(pid: &str) -> String {
    let pid = pid.trim();
    if pid.contains("::") {
        pid.to_string()
    } else {
        format!("{DEFAULT_SCHEME}::{pid}")
    }
}

/// Canonicalise, ignore les valeurs vides, déduplique en conservant l'ordre
/// de première apparition.
pub fn unique_canonical<I: IntoIterator<Item = String>>(values: I) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for v in values {
        let v = v.trim();
        if v.is_empty() {
            continue;
        }
        let c = canonical(v);
        if seen.insert(c.clone()) {
            out.push(c);
        }
    }
    out
}
```

Ajouter dans `lib.rs`, au-dessus de `pub fn run()` : `pub mod pid;`

- [ ] **Step 4 : Vérifier que ça passe**

Run: `cargo test pid` — Attendu : `4 passed`.

- [ ] **Step 5 : Commit**

```bash
git add superpopaul/src-tauri/src
git commit -m "feat(superpopaul): pid — canonicalisation + dédoublonnage (parité popaul.py)"
```

---

### Task 3 : `config.rs` — YAML sans secrets proxy

**Files:**
- Create: `superpopaul/src-tauri/src/config.rs`
- Modify: `superpopaul/src-tauri/src/lib.rs` (ajout `pub mod config;`)

- [ ] **Step 1 : Écrire les tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn config_exemple() -> Config {
        Config {
            version: 1,
            api: ApiConfig {
                url: "https://peppol.example.org".into(),
                key: "MA_CLE".into(),
                batch_size: 50,
                concurrency: 8,
                proxy: Some(ProxyConfig {
                    url: "http://proxy:3128".into(),
                    username: Some("jp".into()),
                    password: Some("SECRET".into()),
                }),
                refresh_days: 30,
            },
            input: InputConfig {
                path: "./clients.csv".into(),
                delimiter: ";".into(),
                encoding: "utf-8".into(),
                pid_column: "siren".into(),
            },
            output: OutputConfig {
                path: "./clients_enrichis.csv".into(),
                timestamp_suffix: true,
                columns: vec![
                    ColumnSpec::Input { name: "siren".into() },
                    ColumnSpec::Peppol { field: PeppolField::Exists },
                    ColumnSpec::Peppol { field: PeppolField::PaCode },
                ],
            },
        }
    }

    #[test]
    fn proxy_creds_never_serialized() {
        // Encode l'intention de sécurité de la spec : le YAML ne doit JAMAIS
        // contenir les identifiants proxy, même s'ils sont en mémoire.
        let yaml = to_yaml(&config_exemple()).unwrap();
        assert!(!yaml.contains("SECRET"));
        assert!(!yaml.contains("username"));
        assert!(!yaml.contains("password"));
        assert!(yaml.contains("http://proxy:3128")); // l'URL, elle, est persistée
    }

    #[test]
    fn round_trip_yaml() {
        let cfg = config_exemple();
        let back = from_yaml(&to_yaml(&cfg).unwrap()).unwrap();
        assert_eq!(back.api.key, "MA_CLE");
        assert_eq!(back.api.batch_size, 50);
        assert_eq!(back.output.columns.len(), 3);
        // Les credentials n'ont pas survécu au round-trip : c'est voulu.
        assert_eq!(back.api.proxy.as_ref().unwrap().username, None);
    }

    #[test]
    fn validate_rejette_batch_size_hors_bornes() {
        let mut cfg = config_exemple();
        cfg.api.batch_size = 51;
        assert!(cfg.validate().is_err());
        cfg.api.batch_size = 0;
        assert!(cfg.validate().is_err());
        cfg.api.batch_size = 1;
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn chemins_resolus_relativement_au_yaml() {
        let p = resolve_relative(std::path::Path::new("/tmp/projet/conf.yaml"), "./clients.csv");
        assert_eq!(p, std::path::PathBuf::from("/tmp/projet/./clients.csv"));
        let abs = resolve_relative(std::path::Path::new("/tmp/projet/conf.yaml"), "/data/x.csv");
        assert_eq!(abs, std::path::PathBuf::from("/data/x.csv"));
    }
}
```

- [ ] **Step 2 : Vérifier l'échec** — Run: `cargo test config` — Attendu : erreur de compilation.

- [ ] **Step 3 : Implémenter**

```rust
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub version: u32,
    pub api: ApiConfig,
    pub input: InputConfig,
    pub output: OutputConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiConfig {
    pub url: String,
    pub key: String,
    pub batch_size: u32,
    pub concurrency: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy: Option<ProxyConfig>,
    pub refresh_days: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyConfig {
    pub url: String,
    /// Identifiants proxy : mémoire uniquement, JAMAIS sérialisés (spec).
    #[serde(skip)]
    pub username: Option<String>,
    #[serde(skip)]
    pub password: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputConfig {
    pub path: String,
    pub delimiter: String,
    pub encoding: String,
    pub pid_column: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputConfig {
    pub path: String,
    pub timestamp_suffix: bool,
    pub columns: Vec<ColumnSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "source", rename_all = "lowercase")]
pub enum ColumnSpec {
    Input { name: String },
    Peppol { field: PeppolField },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PeppolField {
    Exists,
    PaCode,
    PaName,
    PaCountry,
    ExtendedCtcFr,
}

impl Config {
    pub fn validate(&self) -> Result<(), String> {
        if !(1..=50).contains(&self.api.batch_size) {
            return Err("batch_size doit être entre 1 et 50".into());
        }
        if self.api.concurrency < 1 {
            return Err("concurrency doit être ≥ 1".into());
        }
        if self.input.delimiter.len() != 1 {
            return Err("delimiter doit être un caractère unique".into());
        }
        Ok(())
    }
}

pub fn to_yaml(cfg: &Config) -> Result<String, String> {
    serde_yaml::to_string(cfg).map_err(|e| e.to_string())
}

pub fn from_yaml(s: &str) -> Result<Config, String> {
    serde_yaml::from_str(s).map_err(|e| e.to_string())
}

pub fn load(path: &Path) -> Result<Config, String> {
    let s = std::fs::read_to_string(path).map_err(|e| format!("lecture {path:?} : {e}"))?;
    let cfg = from_yaml(&s)?;
    cfg.validate()?;
    Ok(cfg)
}

pub fn save(path: &Path, cfg: &Config) -> Result<(), String> {
    cfg.validate()?;
    std::fs::write(path, to_yaml(cfg)?).map_err(|e| format!("écriture {path:?} : {e}"))
}

/// Résout un chemin de la config relativement au répertoire du fichier YAML.
pub fn resolve_relative(yaml_path: &Path, p: &str) -> PathBuf {
    let pb = PathBuf::from(p);
    if pb.is_absolute() {
        pb
    } else {
        yaml_path.parent().unwrap_or(Path::new(".")).join(pb)
    }
}
```

Ajouter `pub mod config;` dans `lib.rs`.

- [ ] **Step 4 : Vérifier** — Run: `cargo test config` — Attendu : `4 passed`.

- [ ] **Step 5 : Commit**

```bash
git add superpopaul/src-tauri/src
git commit -m "feat(superpopaul): config YAML — credentials proxy jamais sérialisés"
```

---

### Task 4 : `store.rs` — base SQLite globale

**Files:**
- Create: `superpopaul/src-tauri/src/store.rs`
- Modify: `superpopaul/src-tauri/src/lib.rs` (ajout `pub mod store;`)

- [ ] **Step 1 : Écrire les tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn res(pid: &str, ok: bool, at: i64) -> Resolution {
        Resolution {
            participant: pid.into(),
            exists_in_peppol: if ok { Some(true) } else { None },
            pa_code: if ok { Some("PA0042".into()) } else { None },
            pa_name: if ok { Some("ACME PA".into()) } else { None },
            pa_country: if ok { Some("FR".into()) } else { None },
            extended_ctc_fr: if ok { Some(true) } else { None },
            api_status: if ok { "ok".into() } else { "error:503".into() },
            resolved_at: at,
        }
    }

    #[test]
    fn upsert_puis_get() {
        let s = Store::open_in_memory().unwrap();
        s.upsert(&res("iso6523-actorid-upis::0009:1", true, 1000)).unwrap();
        let r = s.get("iso6523-actorid-upis::0009:1").unwrap().unwrap();
        assert_eq!(r.pa_code.as_deref(), Some("PA0042"));
        assert_eq!(r.api_status, "ok");
        // upsert écrase (re-résolution)
        s.upsert(&res("iso6523-actorid-upis::0009:1", true, 2000)).unwrap();
        assert_eq!(s.get("iso6523-actorid-upis::0009:1").unwrap().unwrap().resolved_at, 2000);
    }

    #[test]
    fn load_map_charge_uniquement_les_pids_demandes() {
        let s = Store::open_in_memory().unwrap();
        s.upsert(&res("a::1", true, 1)).unwrap();
        s.upsert(&res("a::2", false, 2)).unwrap();
        s.upsert(&res("a::3", true, 3)).unwrap();
        let m = s.load_map(&["a::1".into(), "a::2".into(), "a::inconnu".into()]).unwrap();
        assert_eq!(m.len(), 2);
        assert!(m.contains_key("a::1"));
        assert!(!m.contains_key("a::3"));
    }

    #[test]
    fn get_absent_renvoie_none() {
        let s = Store::open_in_memory().unwrap();
        assert!(s.get("a::zzz").unwrap().is_none());
    }
}
```

- [ ] **Step 2 : Vérifier l'échec** — Run: `cargo test store` — Attendu : erreur de compilation.

- [ ] **Step 3 : Implémenter**

```rust
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone, serde::Serialize)]
pub struct Resolution {
    pub participant: String,
    pub exists_in_peppol: Option<bool>,
    pub pa_code: Option<String>,
    pub pa_name: Option<String>,
    pub pa_country: Option<String>,
    pub extended_ctc_fr: Option<bool>,
    pub api_status: String,
    pub resolved_at: i64,
}

pub struct Store {
    conn: Connection,
}

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS resolutions (
  participant       TEXT PRIMARY KEY,
  exists_in_peppol  INTEGER,
  pa_code           TEXT,
  pa_name           TEXT,
  pa_country        TEXT,
  extended_ctc_fr   INTEGER,
  api_status        TEXT NOT NULL,
  resolved_at       INTEGER NOT NULL
);
";

impl Store {
    pub fn open(path: &Path) -> Result<Self, String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let conn = Connection::open(path).map_err(|e| e.to_string())?;
        conn.pragma_update(None, "journal_mode", "WAL").map_err(|e| e.to_string())?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> Result<Self, String> {
        Self::init(Connection::open_in_memory().map_err(|e| e.to_string())?)
    }

    fn init(conn: Connection) -> Result<Self, String> {
        conn.execute_batch(SCHEMA).map_err(|e| e.to_string())?;
        Ok(Store { conn })
    }

    pub fn upsert(&self, r: &Resolution) -> Result<(), String> {
        self.conn
            .execute(
                "INSERT INTO resolutions
                 (participant, exists_in_peppol, pa_code, pa_name, pa_country,
                  extended_ctc_fr, api_status, resolved_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8)
                 ON CONFLICT(participant) DO UPDATE SET
                   exists_in_peppol=excluded.exists_in_peppol,
                   pa_code=excluded.pa_code, pa_name=excluded.pa_name,
                   pa_country=excluded.pa_country,
                   extended_ctc_fr=excluded.extended_ctc_fr,
                   api_status=excluded.api_status, resolved_at=excluded.resolved_at",
                params![
                    r.participant,
                    r.exists_in_peppol,
                    r.pa_code,
                    r.pa_name,
                    r.pa_country,
                    r.extended_ctc_fr,
                    r.api_status,
                    r.resolved_at
                ],
            )
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    pub fn get(&self, pid: &str) -> Result<Option<Resolution>, String> {
        self.conn
            .query_row(
                "SELECT participant, exists_in_peppol, pa_code, pa_name, pa_country,
                        extended_ctc_fr, api_status, resolved_at
                 FROM resolutions WHERE participant = ?1",
                params![pid],
                Self::row_to_resolution,
            )
            .optional()
            .map_err(|e| e.to_string())
    }

    /// Charge en mémoire les résolutions des PIDs demandés (calcul des modes,
    /// jointure de sortie). Par lots de 500 pour rester sous la limite de
    /// variables SQLite.
    pub fn load_map(&self, pids: &[String]) -> Result<HashMap<String, Resolution>, String> {
        let mut out = HashMap::with_capacity(pids.len());
        for chunk in pids.chunks(500) {
            let placeholders = vec!["?"; chunk.len()].join(",");
            let sql = format!(
                "SELECT participant, exists_in_peppol, pa_code, pa_name, pa_country,
                        extended_ctc_fr, api_status, resolved_at
                 FROM resolutions WHERE participant IN ({placeholders})"
            );
            let mut stmt = self.conn.prepare(&sql).map_err(|e| e.to_string())?;
            let rows = stmt
                .query_map(rusqlite::params_from_iter(chunk), Self::row_to_resolution)
                .map_err(|e| e.to_string())?;
            for r in rows {
                let r = r.map_err(|e| e.to_string())?;
                out.insert(r.participant.clone(), r);
            }
        }
        Ok(out)
    }

    fn row_to_resolution(row: &rusqlite::Row<'_>) -> rusqlite::Result<Resolution> {
        Ok(Resolution {
            participant: row.get(0)?,
            exists_in_peppol: row.get(1)?,
            pa_code: row.get(2)?,
            pa_name: row.get(3)?,
            pa_country: row.get(4)?,
            extended_ctc_fr: row.get(5)?,
            api_status: row.get(6)?,
            resolved_at: row.get(7)?,
        })
    }
}
```

Ajouter `pub mod store;` dans `lib.rs`.

- [ ] **Step 4 : Vérifier** — Run: `cargo test store` — Attendu : `3 passed`.

- [ ] **Step 5 : Commit**

```bash
git add superpopaul/src-tauri/src
git commit -m "feat(superpopaul): store SQLite (WAL, upsert, load_map par lots)"
```

---

### Task 5 : `modes.rs` — full / reprise / refresh

**Files:**
- Create: `superpopaul/src-tauri/src/modes.rs`
- Modify: `superpopaul/src-tauri/src/lib.rs` (ajout `pub mod modes;`)

- [ ] **Step 1 : Écrire les tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{Resolution, Store};

    /// Base : a::1 résolu ok récent, a::2 résolu ok VIEUX, a::3 en échec,
    /// a::4 absent. now = 100 jours (en secondes).
    fn base() -> (Store, Vec<String>, i64) {
        let s = Store::open_in_memory().unwrap();
        let now = 100 * 86400_i64;
        let mk = |pid: &str, status: &str, at: i64| Resolution {
            participant: pid.into(),
            exists_in_peppol: Some(status == "ok"),
            pa_code: None, pa_name: None, pa_country: None,
            extended_ctc_fr: None,
            api_status: status.into(),
            resolved_at: at,
        };
        s.upsert(&mk("a::1", "ok", now - 86400)).unwrap();        // 1 jour
        s.upsert(&mk("a::2", "ok", now - 50 * 86400)).unwrap();   // 50 jours
        s.upsert(&mk("a::3", "error:503", now - 86400)).unwrap(); // échec
        let pids: Vec<String> = ["a::1", "a::2", "a::3", "a::4"].map(String::from).to_vec();
        (s, pids, now)
    }

    #[test]
    fn full_prend_tout() {
        let (s, pids, now) = base();
        assert_eq!(compute_todo(&RunMode::Full, &pids, &s, now).unwrap(), pids);
    }

    #[test]
    fn reprise_prend_les_absents_seulement() {
        let (s, pids, now) = base();
        let mode = RunMode::Reprise { retry_failures: false };
        assert_eq!(compute_todo(&mode, &pids, &s, now).unwrap(), vec!["a::4".to_string()]);
    }

    #[test]
    fn reprise_avec_retry_reprend_aussi_les_echecs() {
        let (s, pids, now) = base();
        let mode = RunMode::Reprise { retry_failures: true };
        assert_eq!(
            compute_todo(&mode, &pids, &s, now).unwrap(),
            vec!["a::3".to_string(), "a::4".to_string()]
        );
    }

    #[test]
    fn refresh_prend_absents_echecs_et_perimes() {
        let (s, pids, now) = base();
        let mode = RunMode::Refresh { max_age_days: 30 };
        // a::2 (50 jours) est périmé ; a::3 (échec) repris ; a::4 absent ;
        // a::1 (1 jour) est frais → exclu.
        assert_eq!(
            compute_todo(&mode, &pids, &s, now).unwrap(),
            vec!["a::2".to_string(), "a::3".to_string(), "a::4".to_string()]
        );
    }
}
```

- [ ] **Step 2 : Vérifier l'échec** — Run: `cargo test modes` — Attendu : erreur de compilation.

- [ ] **Step 3 : Implémenter**

```rust
use crate::store::Store;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "mode", rename_all = "lowercase")]
pub enum RunMode {
    Full,
    Reprise { retry_failures: bool },
    Refresh { max_age_days: u32 },
}

/// Liste des adressages à résoudre parmi les PIDs uniques du fichier
/// d'entrée, selon le mode. L'ordre d'entrée est conservé.
pub fn compute_todo(
    mode: &RunMode,
    unique_pids: &[String],
    store: &Store,
    now: i64,
) -> Result<Vec<String>, String> {
    if matches!(mode, RunMode::Full) {
        return Ok(unique_pids.to_vec());
    }
    let known = store.load_map(unique_pids)?;
    let keep = |pid: &&String| -> bool {
        match known.get(*pid) {
            None => true, // jamais tenté
            Some(r) => match mode {
                RunMode::Full => true,
                RunMode::Reprise { retry_failures } => *retry_failures && r.api_status != "ok",
                RunMode::Refresh { max_age_days } => {
                    r.api_status != "ok"
                        || r.resolved_at < now - (*max_age_days as i64) * 86400
                }
            },
        }
    };
    Ok(unique_pids.iter().filter(keep).cloned().collect())
}
```

Ajouter `pub mod modes;` dans `lib.rs`.

- [ ] **Step 4 : Vérifier** — Run: `cargo test modes` — Attendu : `4 passed`.

- [ ] **Step 5 : Commit**

```bash
git add superpopaul/src-tauri/src
git commit -m "feat(superpopaul): modes full/reprise/refresh"
```

---

### Task 6 : `csv_io.rs` — sniff, aperçu, lecture de colonne, suggestion

**Files:**
- Create: `superpopaul/src-tauri/src/csv_io.rs`
- Modify: `superpopaul/src-tauri/src/lib.rs` (ajout `pub mod csv_io;`)

- [ ] **Step 1 : Écrire les tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn tmp_csv(content: &[u8]) -> tempfile::NamedTempFile {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(content).unwrap();
        f.flush().unwrap();
        f
    }

    #[test]
    fn sniff_detecte_point_virgule_et_utf8() {
        let f = tmp_csv("siren;raison_sociale\n0009:1;ACME\n".as_bytes());
        let m = sniff(f.path()).unwrap();
        assert_eq!(m.delimiter, b';');
        assert_eq!(m.encoding, "utf-8");
    }

    #[test]
    fn sniff_detecte_virgule_et_windows1252() {
        // "société" avec é encodé windows-1252 (0xE9)
        let mut bytes = b"siren,soci".to_vec();
        bytes.push(0xE9);
        bytes.extend_from_slice(b"t\n1,ACME\n");
        let f = tmp_csv(&bytes);
        let m = sniff(f.path()).unwrap();
        assert_eq!(m.delimiter, b',');
        assert_eq!(m.encoding, "windows-1252");
    }

    #[test]
    fn preview_renvoie_entetes_et_lignes() {
        let f = tmp_csv("a;b\n1;x\n2;y\n3;z\n".as_bytes());
        let p = preview(f.path(), 2).unwrap();
        assert_eq!(p.headers, vec!["a", "b"]);
        assert_eq!(p.rows, vec![vec!["1", "x"], vec!["2", "y"]]);
    }

    #[test]
    fn read_column_renvoie_toutes_les_valeurs_dans_l_ordre() {
        let f = tmp_csv("id;siren\nl1;0009:1\nl2;0009:2\nl3;0009:1\n".as_bytes());
        let m = sniff(f.path()).unwrap();
        let vals = read_column(f.path(), &m, "siren").unwrap();
        assert_eq!(vals, vec!["0009:1", "0009:2", "0009:1"]);
    }

    #[test]
    fn read_column_colonne_inconnue_erreur_claire() {
        let f = tmp_csv("a;b\n1;2\n".as_bytes());
        let m = sniff(f.path()).unwrap();
        let err = read_column(f.path(), &m, "zz").unwrap_err();
        assert!(err.contains("zz"), "message: {err}");
    }

    #[test]
    fn suggest_trouve_la_colonne_pid() {
        let p = Preview {
            headers: vec!["id".into(), "siren".into(), "nom".into()],
            rows: vec![
                vec!["l1".into(), "0009:552100554".into(), "ACME".into()],
                vec!["l2".into(), "552100554".into(), "GLOBEX".into()],
            ],
            delimiter: ";".into(),
            encoding: "utf-8".into(),
        };
        assert_eq!(suggest_pid_column(&p), Some(1));
    }

    #[test]
    fn suggest_none_si_rien_ne_ressemble() {
        let p = Preview {
            headers: vec!["nom".into()],
            rows: vec![vec!["ACME".into()], vec!["GLOBEX".into()]],
            delimiter: ";".into(),
            encoding: "utf-8".into(),
        };
        assert_eq!(suggest_pid_column(&p), None);
    }
}
```

- [ ] **Step 2 : Vérifier l'échec** — Run: `cargo test csv_io` — Attendu : erreur de compilation.

- [ ] **Step 3 : Implémenter**

```rust
use chardetng::EncodingDetector;
use encoding_rs_io::DecodeReaderBytesBuilder;
use serde::Serialize;
use std::fs::File;
use std::io::Read;
use std::path::Path;

#[derive(Debug, Clone, Serialize)]
pub struct CsvMeta {
    pub delimiter: u8,
    pub encoding: &'static str, // "utf-8" | "windows-1252"
}

#[derive(Debug, Clone, Serialize)]
pub struct Preview {
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
    pub delimiter: String,
    pub encoding: String,
}

/// Détecte séparateur et encodage sur les premiers 64 Ko.
pub fn sniff(path: &Path) -> Result<CsvMeta, String> {
    let mut buf = vec![0u8; 64 * 1024];
    let n = File::open(path)
        .and_then(|mut f| f.read(&mut buf))
        .map_err(|e| format!("lecture {path:?} : {e}"))?;
    let sample = &buf[..n];

    let mut det = EncodingDetector::new();
    det.feed(sample, n < buf.len());
    let enc = det.guess(None, true);
    let encoding = if enc == encoding_rs::UTF_8 { "utf-8" } else { "windows-1252" };

    let first_line = sample.split(|&b| b == b'\n').next().unwrap_or(sample);
    let delimiter = [b';', b',', b'\t', b'|']
        .into_iter()
        .max_by_key(|d| first_line.iter().filter(|&&b| b == *d).count())
        .unwrap();
    Ok(CsvMeta { delimiter, encoding })
}

fn reader(path: &Path, meta: &CsvMeta) -> Result<csv::Reader<Box<dyn Read>>, String> {
    let f = File::open(path).map_err(|e| format!("ouverture {path:?} : {e}"))?;
    let enc = if meta.encoding == "utf-8" { encoding_rs::UTF_8 } else { encoding_rs::WINDOWS_1252 };
    let decoded: Box<dyn Read> = Box::new(
        DecodeReaderBytesBuilder::new().encoding(Some(enc)).bom_sniffing(true).build(f),
    );
    Ok(csv::ReaderBuilder::new()
        .delimiter(meta.delimiter)
        .flexible(true)
        .from_reader(decoded))
}

/// Entêtes + n premières lignes, pour l'aperçu du wizard.
pub fn preview(path: &Path, n: usize) -> Result<Preview, String> {
    let meta = sniff(path)?;
    let mut rdr = reader(path, &meta)?;
    let headers = rdr
        .headers()
        .map_err(|e| e.to_string())?
        .iter()
        .map(String::from)
        .collect();
    let mut rows = Vec::with_capacity(n);
    for rec in rdr.records().take(n) {
        let rec = rec.map_err(|e| e.to_string())?;
        rows.push(rec.iter().map(String::from).collect());
    }
    Ok(Preview {
        headers,
        rows,
        delimiter: (meta.delimiter as char).to_string(),
        encoding: meta.encoding.to_string(),
    })
}

/// Toutes les valeurs (brutes, non dédupliquées) d'une colonne, dans l'ordre
/// du fichier. Streaming : la mémoire ne contient que les valeurs.
pub fn read_column(path: &Path, meta: &CsvMeta, column: &str) -> Result<Vec<String>, String> {
    let mut rdr = reader(path, meta)?;
    let headers = rdr.headers().map_err(|e| e.to_string())?.clone();
    let idx = headers
        .iter()
        .position(|h| h == column)
        .ok_or_else(|| format!("Colonne '{column}' absente de l'entête : {headers:?}"))?;
    let mut out = Vec::new();
    for rec in rdr.records() {
        let rec = rec.map_err(|e| e.to_string())?;
        out.push(rec.get(idx).unwrap_or("").to_string());
    }
    Ok(out)
}

/// Ressemble à un adressage Peppol : forme longue "scheme::valeur",
/// "xxxx:yyyy" (préfixe numérique à 4 chiffres), ou SIREN (9 chiffres).
fn looks_like_pid(v: &str) -> bool {
    let v = v.trim();
    if v.contains("::") {
        return true;
    }
    if let Some((prefix, rest)) = v.split_once(':') {
        return prefix.len() == 4 && prefix.chars().all(|c| c.is_ascii_digit()) && !rest.is_empty();
    }
    v.len() == 9 && v.chars().all(|c| c.is_ascii_digit())
}

/// Suggère l'index de la colonne d'adressage : celle dont ≥ 60 % des valeurs
/// d'exemple non vides ressemblent à un PID (meilleur score si plusieurs).
pub fn suggest_pid_column(p: &Preview) -> Option<usize> {
    let mut best: Option<(usize, f64)> = None;
    for col in 0..p.headers.len() {
        let vals: Vec<&str> = p
            .rows
            .iter()
            .filter_map(|r| r.get(col).map(String::as_str))
            .filter(|v| !v.trim().is_empty())
            .collect();
        if vals.is_empty() {
            continue;
        }
        let score = vals.iter().filter(|v| looks_like_pid(v)).count() as f64 / vals.len() as f64;
        if score >= 0.6 && best.map_or(true, |(_, s)| score > s) {
            best = Some((col, score));
        }
    }
    best.map(|(i, _)| i)
}
```

Ajouter `pub mod csv_io;` dans `lib.rs`.

- [ ] **Step 4 : Vérifier** — Run: `cargo test csv_io` — Attendu : `7 passed`.

- [ ] **Step 5 : Commit**

```bash
git add superpopaul/src-tauri/src
git commit -m "feat(superpopaul): csv_io — sniff, aperçu, lecture colonne, suggestion PID"
```

---

### Task 7 : `api.rs` — client HTTP (proxy, erreurs typées)

**Files:**
- Create: `superpopaul/src-tauri/src/api.rs`
- Modify: `superpopaul/src-tauri/src/lib.rs` (ajout `pub mod api;`)

Le client fait **un** appel par méthode, sans retry interne : les retries sont pilotés par le moteur (Task 9), pour que 429/5xx alimentent l'AIMD et le circuit breaker.

- [ ] **Step 1 : Écrire les tests (wiremock)**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn pids(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[tokio::test]
    async fn resolve_batch_parse_la_reponse() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/resolve/batch"))
            .and(header("X-API-Key", "BONNE_CLE"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "results": [
                    {"participant_id": "iso6523-actorid-upis::0009:1", "exists": true,
                     "pa": {"code": "PA0042", "name": "ACME PA", "country": "FR"},
                     "supports_extended_ctc_fr": true, "note": null},
                    {"participant": "0009:zz", "error": "Identifiant invalide."}
                ]
            })))
            .mount(&server)
            .await;

        let c = ApiClient::new(&server.uri(), "BONNE_CLE", None, None).unwrap();
        let (items, stats) = c.resolve_batch(&pids(&["0009:1", "0009:zz"])).await.unwrap();
        assert_eq!(stats.http_status, 200);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].exists, Some(true));
        assert_eq!(items[0].pa.as_ref().unwrap().code.as_deref(), Some("PA0042"));
        assert_eq!(items[1].error.as_deref(), Some("Identifiant invalide."));
    }

    #[tokio::test]
    async fn erreur_401_typee_auth() {
        let server = MockServer::start().await;
        Mock::given(method("POST")).and(path("/resolve/batch"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server).await;
        let c = ApiClient::new(&server.uri(), "MAUVAISE", None, None).unwrap();
        assert!(matches!(
            c.resolve_batch(&pids(&["0009:1"])).await,
            Err(ApiError::Auth(401))
        ));
    }

    #[tokio::test]
    async fn erreur_429_lit_retry_after() {
        let server = MockServer::start().await;
        Mock::given(method("POST")).and(path("/resolve/batch"))
            .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "3"))
            .mount(&server).await;
        let c = ApiClient::new(&server.uri(), "K", None, None).unwrap();
        match c.resolve_batch(&pids(&["0009:1"])).await {
            Err(ApiError::RateLimited { retry_after_s }) => assert_eq!(retry_after_s, 3.0),
            other => panic!("attendu RateLimited, obtenu {other:?}"),
        }
    }

    #[tokio::test]
    async fn erreur_5xx_typee_server() {
        let server = MockServer::start().await;
        Mock::given(method("POST")).and(path("/resolve/batch"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server).await;
        let c = ApiClient::new(&server.uri(), "K", None, None).unwrap();
        assert!(matches!(
            c.resolve_batch(&pids(&["0009:1"])).await,
            Err(ApiError::Server(503))
        ));
    }

    #[tokio::test]
    async fn test_key_ok_sur_resolve_unitaire() {
        let server = MockServer::start().await;
        Mock::given(method("GET")).and(path("/resolve/0009:552100554"))
            .and(header("X-API-Key", "BONNE_CLE"))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                serde_json::json!({"participant_id": "iso6523-actorid-upis::0009:552100554",
                                   "exists": true})))
            .mount(&server).await;
        let c = ApiClient::new(&server.uri(), "BONNE_CLE", None, None).unwrap();
        assert!(c.test_key().await.is_ok());
    }
}
```

- [ ] **Step 2 : Vérifier l'échec** — Run: `cargo test api` — Attendu : erreur de compilation.

- [ ] **Step 3 : Implémenter**

```rust
use serde::Deserialize;
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct ProxyCreds {
    pub username: String,
    pub password: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("Clé API invalide ou révoquée (HTTP {0}).")]
    Auth(u16),
    #[error("Le proxy demande une authentification (HTTP 407).")]
    ProxyAuth,
    #[error("Rate limit atteint (HTTP 429), Retry-After {retry_after_s}s.")]
    RateLimited { retry_after_s: f64 },
    #[error("Erreur serveur (HTTP {0}).")]
    Server(u16),
    #[error("Erreur réseau : {0}")]
    Network(String),
}

impl ApiError {
    /// Code HTTP associé, pour la répartition des codes au dashboard
    /// (0 = erreur réseau sans réponse).
    pub fn http_status(&self) -> u16 {
        match self {
            ApiError::Auth(s) | ApiError::Server(s) => *s,
            ApiError::ProxyAuth => 407,
            ApiError::RateLimited { .. } => 429,
            ApiError::Network(_) => 0,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct PaInfo {
    pub code: Option<String>,
    pub name: Option<String>,
    pub country: Option<String>,
}

/// Un item de réponse de l'API (format vérifié dans peppol_api.py) :
/// succès = {participant_id, exists, pa{...}, supports_extended_ctc_fr, note} ;
/// échec  = {participant, error}.
#[derive(Debug, Clone, Deserialize)]
pub struct ApiItem {
    #[serde(default)]
    pub participant_id: Option<String>,
    #[serde(default)]
    pub participant: Option<String>,
    #[serde(default)]
    pub exists: Option<bool>,
    #[serde(default)]
    pub pa: Option<PaInfo>,
    #[serde(default)]
    pub supports_extended_ctc_fr: Option<bool>,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct CallStats {
    pub http_status: u16,
    pub latency_ms: u64,
}

#[derive(Clone)]
pub struct ApiClient {
    http: reqwest::Client,
    base: String,
    key: String,
}

impl ApiClient {
    pub fn new(
        base_url: &str,
        key: &str,
        proxy_url: Option<&str>,
        creds: Option<&ProxyCreds>,
    ) -> Result<Self, String> {
        let mut b = reqwest::Client::builder().timeout(Duration::from_secs(75));
        if let Some(purl) = proxy_url {
            let mut p = reqwest::Proxy::all(purl).map_err(|e| format!("proxy : {e}"))?;
            if let Some(c) = creds {
                p = p.basic_auth(&c.username, &c.password);
            }
            b = b.proxy(p);
        }
        Ok(ApiClient {
            http: b.build().map_err(|e| e.to_string())?,
            base: base_url.trim_end_matches('/').to_string(),
            key: key.to_string(),
        })
    }

    /// Même client (même pool/proxy), nouvelle clé — pour la reprise après 401.
    pub fn with_key(&self, key: &str) -> Self {
        ApiClient { key: key.to_string(), ..self.clone() }
    }

    pub async fn resolve_batch(
        &self,
        pids: &[String],
    ) -> Result<(Vec<ApiItem>, CallStats), ApiError> {
        let t0 = Instant::now();
        let resp = self
            .http
            .post(format!("{}/resolve/batch", self.base))
            .header("X-API-Key", &self.key)
            .json(&serde_json::json!({ "participants": pids, "test": false }))
            .send()
            .await
            .map_err(|e| self.map_send_err(e))?;
        let latency_ms = t0.elapsed().as_millis() as u64;
        let status = resp.status().as_u16();
        match status {
            200 => {
                #[derive(Deserialize)]
                struct R { results: Vec<ApiItem> }
                let r: R = resp.json().await.map_err(|e| ApiError::Network(e.to_string()))?;
                Ok((r.results, CallStats { http_status: 200, latency_ms }))
            }
            s => Err(Self::status_to_error(s, resp.headers())),
        }
    }

    /// Test unitaire de la clé : une vraie résolution GET /resolve/<pid>.
    pub async fn test_key(&self) -> Result<CallStats, ApiError> {
        let t0 = Instant::now();
        let resp = self
            .http
            .get(format!("{}/resolve/0009:552100554", self.base))
            .header("X-API-Key", &self.key)
            .send()
            .await
            .map_err(|e| self.map_send_err(e))?;
        let latency_ms = t0.elapsed().as_millis() as u64;
        let status = resp.status().as_u16();
        if status == 200 {
            Ok(CallStats { http_status: 200, latency_ms })
        } else {
            Err(Self::status_to_error(status, resp.headers()))
        }
    }

    /// Connectivité seule (endpoint public /health, sans clé).
    pub async fn health(&self) -> Result<CallStats, ApiError> {
        let t0 = Instant::now();
        let resp = self
            .http
            .get(format!("{}/health", self.base))
            .send()
            .await
            .map_err(|e| self.map_send_err(e))?;
        let latency_ms = t0.elapsed().as_millis() as u64;
        let status = resp.status().as_u16();
        if status == 200 {
            Ok(CallStats { http_status: 200, latency_ms })
        } else {
            Err(Self::status_to_error(status, resp.headers()))
        }
    }

    fn status_to_error(status: u16, headers: &reqwest::header::HeaderMap) -> ApiError {
        match status {
            401 | 403 => ApiError::Auth(status),
            407 => ApiError::ProxyAuth,
            429 => {
                let retry_after_s = headers
                    .get("Retry-After")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.parse::<f64>().ok())
                    .unwrap_or(2.0);
                ApiError::RateLimited { retry_after_s }
            }
            s => ApiError::Server(s),
        }
    }

    fn map_send_err(&self, e: reqwest::Error) -> ApiError {
        // reqwest signale l'échec d'auth proxy comme une erreur de connexion ;
        // on repère "407" dans le message pour donner un diagnostic actionnable.
        let msg = e.to_string();
        if msg.contains("407") {
            ApiError::ProxyAuth
        } else {
            ApiError::Network(msg)
        }
    }
}
```

Ajouter `pub mod api;` dans `lib.rs`.

- [ ] **Step 4 : Vérifier** — Run: `cargo test api` — Attendu : `5 passed`.

- [ ] **Step 5 : Commit**

```bash
git add superpopaul/src-tauri/src
git commit -m "feat(superpopaul): client API — batch/test/health, erreurs typées, proxy"
```

---

### Task 8 : `telemetry.rs` — compteurs, latences, débits, ETA

**Files:**
- Create: `superpopaul/src-tauri/src/telemetry.rs`
- Modify: `superpopaul/src-tauri/src/lib.rs` (ajout `pub mod telemetry;`)

- [ ] **Step 1 : Écrire les tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compteurs_et_pourcentages() {
        let t = Telemetry::new(1000);
        // 2 appels : 50 adressages ok (30 existent, 20 ctc), puis 25 ok + 5 échecs
        t.record_call(200, 120, 50, 30, 20, 0);
        t.record_call(200, 250, 30, 10, 5, 5);
        let s = t.snapshot();
        assert_eq!(s.total, 1000);
        assert_eq!(s.done, 80);
        assert_eq!(s.exists, 40);
        assert_eq!(s.ctc, 25);
        assert_eq!(s.failed, 5);
        assert_eq!(s.http.get(&200), Some(&2));
    }

    #[test]
    fn erreurs_comptees_sans_progression() {
        let t = Telemetry::new(100);
        t.record_error(429);
        t.record_error(0); // réseau
        let s = t.snapshot();
        assert_eq!(s.done, 0);
        assert_eq!(s.http.get(&429), Some(&1));
        assert_eq!(s.http.get(&0), Some(&1));
    }

    #[test]
    fn percentiles_latence() {
        let t = Telemetry::new(100);
        for (i, ms) in (1..=100u64).enumerate() {
            t.record_call(200, ms, 1, 0, 0, 0);
            let _ = i;
        }
        let s = t.snapshot();
        let l = s.latency.unwrap();
        assert_eq!(l.min, 1);
        assert_eq!(l.max, 100);
        assert_eq!(l.p50, 50);
        assert_eq!(l.p90, 90);
        assert_eq!(l.p99, 99);
    }

    #[test]
    fn eta_present_des_qu_il_y_a_du_debit() {
        let t = Telemetry::new(1000);
        assert!(t.snapshot().eta_s.is_none()); // rien traité
        t.record_call(200, 100, 100, 0, 0, 0);
        let s = t.snapshot();
        assert!(s.addr_per_s > 0.0);
        assert!(s.eta_s.is_some());
    }
}
```

- [ ] **Step 2 : Vérifier l'échec** — Run: `cargo test telemetry` — Attendu : erreur de compilation.

- [ ] **Step 3 : Implémenter**

```rust
use serde::Serialize;
use std::collections::{BTreeMap, VecDeque};
use std::sync::Mutex;
use std::time::Instant;

/// Fenêtre glissante pour les débits instantanés.
const WINDOW_S: f64 = 10.0;

pub struct Telemetry {
    total: u64,
    inner: Mutex<Inner>,
}

struct Inner {
    done: u64,
    exists: u64,
    ctc: u64,
    failed: u64,
    http: BTreeMap<u16, u64>,
    latencies_ms: Vec<u32>,
    calls: VecDeque<(Instant, u32)>, // (instant, adressages traités par l'appel)
}

#[derive(Debug, Clone, Serialize)]
pub struct LatStats {
    pub min: u32,
    pub mean: u32,
    pub p50: u32,
    pub p90: u32,
    pub p99: u32,
    pub max: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct Snapshot {
    pub done: u64,
    pub total: u64,
    pub exists: u64,
    pub ctc: u64,
    pub failed: u64,
    pub http: BTreeMap<u16, u64>,
    pub latency: Option<LatStats>,
    pub req_per_s: f64,
    pub addr_per_s: f64,
    pub eta_s: Option<u64>,
}

impl Telemetry {
    pub fn new(total: u64) -> Self {
        Telemetry {
            total,
            inner: Mutex::new(Inner {
                done: 0,
                exists: 0,
                ctc: 0,
                failed: 0,
                http: BTreeMap::new(),
                latencies_ms: Vec::new(),
                calls: VecDeque::new(),
            }),
        }
    }

    /// Un appel HTTP abouti (200) : addr adressages traités, dont `exists`
    /// présents Peppol, `ctc` supportant CTC-FR, `failed` en erreur item.
    pub fn record_call(&self, http_status: u16, latency_ms: u64, addr: u32, exists: u32, ctc: u32, failed: u32) {
        let mut i = self.inner.lock().unwrap();
        i.done += addr as u64;
        i.exists += exists as u64;
        i.ctc += ctc as u64;
        i.failed += failed as u64;
        *i.http.entry(http_status).or_insert(0) += 1;
        i.latencies_ms.push(latency_ms.min(u32::MAX as u64) as u32);
        i.calls.push_back((Instant::now(), addr));
    }

    /// Un appel HTTP en erreur (0 = réseau) : compté, aucune progression.
    pub fn record_error(&self, http_status: u16) {
        let mut i = self.inner.lock().unwrap();
        *i.http.entry(http_status).or_insert(0) += 1;
        i.calls.push_back((Instant::now(), 0));
    }

    pub fn snapshot(&self) -> Snapshot {
        let mut i = self.inner.lock().unwrap();
        let now = Instant::now();
        while let Some((t, _)) = i.calls.front() {
            if now.duration_since(*t).as_secs_f64() > WINDOW_S {
                i.calls.pop_front();
            } else {
                break;
            }
        }
        // Fenêtre effective : depuis le plus vieil appel conservé (évite de
        // diviser par 10 s quand le run vient de démarrer).
        let span = i
            .calls
            .front()
            .map(|(t, _)| now.duration_since(*t).as_secs_f64().max(0.25))
            .unwrap_or(1.0);
        let req_per_s = i.calls.len() as f64 / span;
        let addr_in_window: u64 = i.calls.iter().map(|(_, a)| *a as u64).sum();
        let addr_per_s = addr_in_window as f64 / span;
        let remaining = self.total.saturating_sub(i.done);
        let eta_s = if addr_per_s > 0.0 && remaining > 0 {
            Some((remaining as f64 / addr_per_s).round() as u64)
        } else {
            None
        };
        Snapshot {
            done: i.done,
            total: self.total,
            exists: i.exists,
            ctc: i.ctc,
            failed: i.failed,
            http: i.http.clone(),
            latency: lat_stats(&i.latencies_ms),
            req_per_s,
            addr_per_s,
            eta_s,
        }
    }
}

fn lat_stats(lat: &[u32]) -> Option<LatStats> {
    if lat.is_empty() {
        return None;
    }
    let mut v = lat.to_vec();
    v.sort_unstable();
    let pct = |p: f64| v[((v.len() as f64 - 1.0) * p / 100.0).round() as usize];
    Some(LatStats {
        min: v[0],
        mean: (v.iter().map(|&x| x as u64).sum::<u64>() / v.len() as u64) as u32,
        p50: pct(50.0),
        p90: pct(90.0),
        p99: pct(99.0),
        max: *v.last().unwrap(),
    })
}
```

Ajouter `pub mod telemetry;` dans `lib.rs`.

- [ ] **Step 4 : Vérifier** — Run: `cargo test telemetry` — Attendu : `4 passed`.

- [ ] **Step 5 : Commit**

```bash
git add superpopaul/src-tauri/src
git commit -m "feat(superpopaul): telemetry — compteurs, latences pXX, débits req/s+adr/s, ETA"
```

---

### Task 9 : `resolver.rs` — moteur (workers, pause/stop, AIMD, breaker, calibrage)

**Files:**
- Create: `superpopaul/src-tauri/src/resolver.rs`
- Modify: `superpopaul/src-tauri/src/lib.rs` (ajout `pub mod resolver;`)

Découpé en trois sous-étapes TDD : (a) AIMD + breaker (unitaires purs), (b) moteur intégré (wiremock), (c) calibrage.

- [ ] **Step 1 : Tests unitaires AIMD et circuit breaker**

```rust
#[cfg(test)]
mod tests_ctrl {
    use super::*;

    #[test]
    fn aimd_divise_par_deux_sur_429_et_remonte_de_un() {
        let a = Aimd::new(16);
        assert_eq!(a.allowed(), 16);
        a.on_rate_limited();
        assert_eq!(a.allowed(), 8);
        a.on_rate_limited();
        assert_eq!(a.allowed(), 4);
        // 50 succès consécutifs → +1, plafonné au max initial
        for _ in 0..50 { a.on_success(); }
        assert_eq!(a.allowed(), 5);
        for _ in 0..(50 * 20) { a.on_success(); }
        assert_eq!(a.allowed(), 16); // jamais au-dessus du max configuré
    }

    #[test]
    fn aimd_ne_descend_jamais_sous_un() {
        let a = Aimd::new(2);
        a.on_rate_limited();
        a.on_rate_limited();
        a.on_rate_limited();
        assert_eq!(a.allowed(), 1);
    }

    #[test]
    fn breaker_ouvre_apres_seuil_et_backoff_croissant() {
        let mut b = Breaker::new(3);
        assert_eq!(b.on_failure(), None);
        assert_eq!(b.on_failure(), None);
        let d1 = b.on_failure().expect("ouvre au 3e échec");
        assert_eq!(d1.as_secs(), 30);
        // ré-ouvre : backoff double, plafonné à 300 s
        b.on_failure(); b.on_failure();
        let d2 = b.on_failure().unwrap();
        assert_eq!(d2.as_secs(), 60);
        b.on_success(); // succès → tout est réarmé
        b.on_failure(); b.on_failure();
        assert_eq!(b.on_failure().unwrap().as_secs(), 30);
    }
}
```

- [ ] **Step 2 : Vérifier l'échec** — Run: `cargo test tests_ctrl` — Attendu : erreur de compilation.

- [ ] **Step 3 : Implémenter `Aimd` et `Breaker`**

```rust
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

/// Concurrence adaptative : ÷2 sur rate-limit, +1 après 50 succès consécutifs
/// (AIMD), bornée à [1, max].
pub struct Aimd {
    allowed: AtomicU32,
    max: u32,
    ok_streak: AtomicU32,
}

const AIMD_STREAK: u32 = 50;

impl Aimd {
    pub fn new(max: u32) -> Self {
        Aimd { allowed: AtomicU32::new(max.max(1)), max: max.max(1), ok_streak: AtomicU32::new(0) }
    }

    pub fn allowed(&self) -> u32 {
        self.allowed.load(Ordering::Relaxed)
    }

    pub fn on_rate_limited(&self) {
        self.ok_streak.store(0, Ordering::Relaxed);
        let cur = self.allowed.load(Ordering::Relaxed);
        self.allowed.store((cur / 2).max(1), Ordering::Relaxed);
    }

    pub fn on_success(&self) {
        let streak = self.ok_streak.fetch_add(1, Ordering::Relaxed) + 1;
        if streak >= AIMD_STREAK {
            self.ok_streak.store(0, Ordering::Relaxed);
            let cur = self.allowed.load(Ordering::Relaxed);
            self.allowed.store((cur + 1).min(self.max), Ordering::Relaxed);
        }
    }
}

/// Circuit breaker : ouvre après `threshold` échecs consécutifs (5xx/réseau),
/// avec un backoff 30 s doublé à chaque ouverture (plafond 300 s).
pub struct Breaker {
    threshold: u32,
    consecutive: u32,
    opens: u32,
}

impl Breaker {
    pub fn new(threshold: u32) -> Self {
        Breaker { threshold, consecutive: 0, opens: 0 }
    }

    pub fn on_failure(&mut self) -> Option<Duration> {
        self.consecutive += 1;
        if self.consecutive >= self.threshold {
            self.consecutive = 0;
            let secs = (30u64 << self.opens.min(3)).min(300);
            self.opens += 1;
            Some(Duration::from_secs(secs))
        } else {
            None
        }
    }

    pub fn on_success(&mut self) {
        self.consecutive = 0;
        self.opens = 0;
    }
}
```

- [ ] **Step 4 : Vérifier** — Run: `cargo test tests_ctrl` — Attendu : `3 passed`. Commit :

```bash
git add superpopaul/src-tauri/src
git commit -m "feat(superpopaul): AIMD + circuit breaker"
```

- [ ] **Step 5 : Tests d'intégration du moteur (wiremock)**

Un `Respond` custom fait écho aux participants reçus (réponse dynamique) :

```rust
#[cfg(test)]
mod tests_engine {
    use super::*;
    use crate::api::ApiClient;
    use crate::store::Store;
    use crate::telemetry::Telemetry;
    use std::sync::{Arc, Mutex};
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

    /// Répond 200 en faisant écho : chaque participant reçu existe dans Peppol.
    struct EchoResolver;
    impl Respond for EchoResolver {
        fn respond(&self, req: &Request) -> ResponseTemplate {
            let body: serde_json::Value = serde_json::from_slice(&req.body).unwrap();
            let results: Vec<serde_json::Value> = body["participants"]
                .as_array().unwrap().iter()
                .map(|p| serde_json::json!({
                    "participant_id": p, "exists": true,
                    "pa": {"code": "PA1", "name": "PA UN", "country": "FR"},
                    "supports_extended_ctc_fr": true
                }))
                .collect();
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"results": results}))
        }
    }

    fn pids(n: usize) -> Vec<String> {
        (0..n).map(|i| format!("iso6523-actorid-upis::0009:{i}")).collect()
    }

    async fn run_engine(
        server: &MockServer,
        key: &str,
        todo: Vec<String>,
    ) -> (RunHandle, tokio::sync::mpsc::Receiver<EngineEvent>, Arc<Mutex<Store>>) {
        let store = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
        let client = ApiClient::new(&server.uri(), key, None, None).unwrap();
        let (tx, rx) = tokio::sync::mpsc::channel(256);
        let handle = Engine::start(
            client,
            EngineParams { batch_size: 10, concurrency: 4 },
            todo,
            store.clone(),
            tx,
        );
        (handle, rx, store)
    }

    async fn wait_finished(rx: &mut tokio::sync::mpsc::Receiver<EngineEvent>) -> (u64, u64) {
        loop {
            match tokio::time::timeout(std::time::Duration::from_secs(10), rx.recv()).await {
                Ok(Some(EngineEvent::Finished { done, failed, .. })) => return (done, failed),
                Ok(Some(_)) => continue,
                other => panic!("Finished attendu, obtenu {other:?}"),
            }
        }
    }

    #[tokio::test]
    async fn chemin_nominal_tout_est_resolu_en_base() {
        let server = MockServer::start().await;
        Mock::given(method("POST")).and(path("/resolve/batch"))
            .respond_with(EchoResolver).mount(&server).await;
        let (handle, mut rx, store) = run_engine(&server, "K", pids(53)).await;
        let (done, failed) = wait_finished(&mut rx).await;
        assert_eq!((done, failed), (53, 0));
        let m = store.lock().unwrap().load_map(&pids(53)).unwrap();
        assert_eq!(m.len(), 53);
        assert!(m.values().all(|r| r.api_status == "ok" && r.pa_code.as_deref() == Some("PA1")));
        let _ = handle;
    }

    #[tokio::test]
    async fn un_429_ralentit_puis_le_run_aboutit() {
        let server = MockServer::start().await;
        // Le premier appel prend un 429, les suivants passent.
        Mock::given(method("POST")).and(path("/resolve/batch"))
            .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "0"))
            .up_to_n_times(1).mount(&server).await;
        Mock::given(method("POST")).and(path("/resolve/batch"))
            .respond_with(EchoResolver).mount(&server).await;
        let (handle, mut rx, _store) = run_engine(&server, "K", pids(30)).await;
        let (done, _) = wait_finished(&mut rx).await;
        assert_eq!(done, 30);
        assert_eq!(handle.telemetry.snapshot().http.get(&429), Some(&1));
    }

    #[tokio::test]
    async fn cle_invalide_suspend_puis_nouvelle_cle_reprend() {
        let server = MockServer::start().await;
        Mock::given(method("POST")).and(path("/resolve/batch"))
            .and(header("X-API-Key", "BONNE"))
            .respond_with(EchoResolver).mount(&server).await;
        Mock::given(method("POST")).and(path("/resolve/batch"))
            .respond_with(ResponseTemplate::new(401)).mount(&server).await;

        let (handle, mut rx, _store) = run_engine(&server, "MAUVAISE", pids(20)).await;
        // On doit recevoir Suspended{auth_api}
        loop {
            match tokio::time::timeout(std::time::Duration::from_secs(10), rx.recv()).await {
                Ok(Some(EngineEvent::Suspended { reason, .. })) => {
                    assert_eq!(reason, "auth_api");
                    break;
                }
                Ok(Some(_)) => continue,
                other => panic!("Suspended attendu, obtenu {other:?}"),
            }
        }
        handle.update_key("BONNE");
        handle.set_paused(false);
        let (done, _) = wait_finished(&mut rx).await;
        assert_eq!(done, 20);
    }

    #[tokio::test]
    async fn stop_arrete_sans_perdre_ce_qui_est_fait() {
        let server = MockServer::start().await;
        Mock::given(method("POST")).and(path("/resolve/batch"))
            .respond_with(EchoResolver).mount(&server).await;
        let (handle, mut rx, store) = run_engine(&server, "K", pids(50)).await;
        handle.request_stop();
        let (_done, _) = wait_finished(&mut rx).await;
        // Tout ce qui est marqué done est réellement en base.
        let snap = handle.telemetry.snapshot();
        let m = store.lock().unwrap().load_map(&pids(50)).unwrap();
        assert_eq!(m.len() as u64, snap.done);
    }
}
```

- [ ] **Step 6 : Vérifier l'échec** — Run: `cargo test tests_engine` — Attendu : erreur de compilation.

- [ ] **Step 7 : Implémenter le moteur**

```rust
use crate::api::{ApiClient, ApiError, ApiItem};
use crate::pid::canonical;
use crate::store::{Resolution, Store};
use crate::telemetry::{Snapshot, Telemetry};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{mpsc, watch};

#[derive(Debug)]
pub enum EngineEvent {
    Telemetry(Snapshot),
    /// reason ∈ {"auth_api", "auth_proxy", "server_down"}
    Suspended { reason: String, message: String, retry_in_s: Option<u64> },
    Resumed,
    Finished { done: u64, failed: u64, stopped: bool },
}

pub struct EngineParams {
    pub batch_size: usize,
    pub concurrency: u32,
}

pub struct RunHandle {
    paused: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    key_tx: watch::Sender<String>,
    pub telemetry: Arc<Telemetry>,
}

impl RunHandle {
    pub fn set_paused(&self, p: bool) {
        self.paused.store(p, Ordering::Relaxed);
    }
    pub fn request_stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }
    pub fn update_key(&self, key: &str) {
        let _ = self.key_tx.send(key.to_string());
    }
    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::Relaxed)
    }
}

fn now_epoch() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

/// Convertit un item API en résolution à persister. `sent` = PID envoyé
/// (repli si l'API ne renvoie pas participant_id).
fn to_resolution(item: &ApiItem, sent: &str, at: i64) -> Resolution {
    let participant = item
        .participant_id
        .clone()
        .or_else(|| item.participant.clone().map(|p| canonical(&p)))
        .unwrap_or_else(|| canonical(sent));
    match &item.error {
        Some(e) => Resolution {
            participant,
            exists_in_peppol: None,
            pa_code: None,
            pa_name: None,
            pa_country: None,
            extended_ctc_fr: None,
            api_status: format!("error:{e}"),
            resolved_at: at,
        },
        None => Resolution {
            participant,
            exists_in_peppol: item.exists,
            pa_code: item.pa.as_ref().and_then(|p| p.code.clone()),
            pa_name: item.pa.as_ref().and_then(|p| p.name.clone()),
            pa_country: item.pa.as_ref().and_then(|p| p.country.clone()),
            extended_ctc_fr: item.supports_extended_ctc_fr,
            api_status: "ok".into(),
            resolved_at: at,
        },
    }
}

pub struct Engine;

impl Engine {
    pub fn start(
        client: ApiClient,
        params: EngineParams,
        todo: Vec<String>,
        store: Arc<Mutex<Store>>,
        tx: mpsc::Sender<EngineEvent>,
    ) -> RunHandle {
        let total = todo.len() as u64;
        let telemetry = Arc::new(Telemetry::new(total));
        let paused = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let aimd = Arc::new(Aimd::new(params.concurrency));
        let breaker = Arc::new(Mutex::new(Breaker::new(5)));
        let suspended = Arc::new(AtomicBool::new(false));
        let (key_tx, key_rx) = watch::channel(String::new());
        // La clé initiale vit dans le client ; le canal ne sert qu'aux MAJ.

        let queue: Arc<Mutex<VecDeque<Vec<String>>>> = Arc::new(Mutex::new(
            todo.chunks(params.batch_size.max(1)).map(|c| c.to_vec()).collect(),
        ));
        let in_flight = Arc::new(AtomicU32::new(0));

        let mut workers = Vec::new();
        for idx in 0..params.concurrency {
            let (client, queue, store, telemetry, paused, stop, aimd, breaker, suspended, tx, mut key_rx) = (
                client.clone(), queue.clone(), store.clone(), telemetry.clone(),
                paused.clone(), stop.clone(), aimd.clone(), breaker.clone(),
                suspended.clone(), tx.clone(), key_rx.clone(),
            );
            let in_flight = in_flight.clone();
            workers.push(tokio::spawn(async move {
                let mut client = client;
                loop {
                    if stop.load(Ordering::Relaxed) {
                        break;
                    }
                    if paused.load(Ordering::Relaxed) || idx >= aimd.allowed() {
                        tokio::time::sleep(Duration::from_millis(150)).await;
                        continue;
                    }
                    // Nouvelle clé disponible ? (reprise après 401)
                    if key_rx.has_changed().unwrap_or(false) {
                        let k = key_rx.borrow_and_update().clone();
                        if !k.is_empty() {
                            client = client.with_key(&k);
                            suspended.store(false, Ordering::Relaxed);
                        }
                    }
                    let chunk = { queue.lock().unwrap().pop_front() };
                    let Some(chunk) = chunk else { break };
                    in_flight.fetch_add(1, Ordering::SeqCst);
                    match client.resolve_batch(&chunk).await {
                        Ok((items, stats)) => {
                            breaker.lock().unwrap().on_success();
                            aimd.on_success();
                            let at = now_epoch();
                            let (mut ex, mut ctc, mut failed) = (0u32, 0u32, 0u32);
                            {
                                let st = store.lock().unwrap();
                                for (i, item) in items.iter().enumerate() {
                                    let sent = chunk.get(i).map(String::as_str).unwrap_or("");
                                    let r = to_resolution(item, sent, at);
                                    if r.api_status == "ok" {
                                        if r.exists_in_peppol == Some(true) { ex += 1; }
                                        if r.extended_ctc_fr == Some(true) { ctc += 1; }
                                    } else {
                                        failed += 1;
                                    }
                                    let _ = st.upsert(&r);
                                }
                            }
                            telemetry.record_call(stats.http_status, stats.latency_ms,
                                                  items.len() as u32, ex, ctc, failed);
                        }
                        Err(e) => {
                            telemetry.record_error(e.http_status());
                            // Le paquet repart en tête de file : rien n'est perdu.
                            queue.lock().unwrap().push_front(chunk);
                            match e {
                                ApiError::RateLimited { retry_after_s } => {
                                    aimd.on_rate_limited();
                                    tokio::time::sleep(Duration::from_secs_f64(
                                        retry_after_s.clamp(0.0, 60.0),
                                    )).await;
                                }
                                ApiError::Auth(_) | ApiError::ProxyAuth => {
                                    // Suspension immédiate de tous les workers ;
                                    // un seul événement émis.
                                    paused.store(true, Ordering::Relaxed);
                                    if !suspended.swap(true, Ordering::Relaxed) {
                                        let reason = if matches!(e, ApiError::ProxyAuth) {
                                            "auth_proxy"
                                        } else {
                                            "auth_api"
                                        };
                                        let _ = tx.send(EngineEvent::Suspended {
                                            reason: reason.into(),
                                            message: e.to_string(),
                                            retry_in_s: None,
                                        }).await;
                                    }
                                }
                                ApiError::Server(_) | ApiError::Network(_) => {
                                    let opened = breaker.lock().unwrap().on_failure();
                                    if let Some(d) = opened {
                                        paused.store(true, Ordering::Relaxed);
                                        let _ = tx.send(EngineEvent::Suspended {
                                            reason: "server_down".into(),
                                            message: e.to_string(),
                                            retry_in_s: Some(d.as_secs()),
                                        }).await;
                                        // Re-test automatique après le backoff.
                                        let paused2 = paused.clone();
                                        let tx2 = tx.clone();
                                        tokio::spawn(async move {
                                            tokio::time::sleep(d).await;
                                            paused2.store(false, Ordering::Relaxed);
                                            let _ = tx2.send(EngineEvent::Resumed).await;
                                        });
                                    } else {
                                        tokio::time::sleep(Duration::from_secs(1)).await;
                                    }
                                }
                            }
                        }
                    }
                    in_flight.fetch_sub(1, Ordering::SeqCst);
                }
            }));
        }

        // Superviseur : télémétrie 4×/s, détection de fin.
        {
            let (telemetry, tx, stop) = (telemetry.clone(), tx.clone(), stop.clone());
            tokio::spawn(async move {
                for w in workers {
                    let _ = w.await;
                }
                let s = telemetry.snapshot();
                let _ = tx.send(EngineEvent::Finished {
                    done: s.done,
                    failed: s.failed,
                    stopped: stop.load(Ordering::Relaxed),
                }).await;
            });
        }
        {
            let (telemetry, tx, stop) = (telemetry.clone(), tx, stop.clone());
            tokio::spawn(async move {
                loop {
                    tokio::time::sleep(Duration::from_millis(250)).await;
                    if tx.send(EngineEvent::Telemetry(telemetry.snapshot())).await.is_err() {
                        break; // le récepteur a disparu : run terminé
                    }
                    if stop.load(Ordering::Relaxed) {
                        break;
                    }
                }
            });
        }

        RunHandle { paused, stop, key_tx, telemetry }
    }
}
```

Ajouter `pub mod resolver;` dans `lib.rs`.

**Note d'implémentation pour l'exécutant :** si `cargo test tests_engine` révèle un blocage (workers qui dorment en pause alors que la file est vide), c'est le comportement voulu pour la reprise ; la sortie de boucle n'a lieu que file vide ET non suspendu — ajuster la condition `let Some(chunk) = chunk else { if suspended… sleep+continue; break }` si le test `cle_invalide_...` échoue : un worker ne doit pas se terminer pendant une suspension, car les paquets remis en tête de file doivent être repris après ressaisie de la clé. Le test est le juge.

- [ ] **Step 8 : Vérifier** — Run: `cargo test tests_engine` — Attendu : `4 passed` (et `cargo test` complet vert). Commit :

```bash
git add superpopaul/src-tauri/src
git commit -m "feat(superpopaul): moteur de résolution — workers, pause/stop, suspension 401/407, breaker"
```

- [ ] **Step 9 : Calibrage — test puis implémentation**

Test (dans `resolver.rs`) :

```rust
#[cfg(test)]
mod tests_calibrate {
    use super::*;
    use crate::api::ApiClient;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn calibrate_renvoie_un_debit_et_une_concurrence() {
        let server = MockServer::start().await;
        Mock::given(method("POST")).and(path("/resolve/batch"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_delay(std::time::Duration::from_millis(50))
                    .set_body_json(serde_json::json!({"results": [
                        {"participant_id": "a::1", "exists": true}
                    ]})),
            )
            .mount(&server).await;
        let c = ApiClient::new(&server.uri(), "K", None, None).unwrap();
        let sample: Vec<String> = (0..8).map(|i| format!("0009:{i}")).collect();
        let rep = calibrate(&c, &sample, 1, 8).await;
        assert!(rep.best_concurrency >= 1);
        assert!(rep.addr_per_s > 0.0);
        assert!(!rep.rate_limited);
    }
}
```

Implémentation :

```rust
#[derive(Debug, Clone, serde::Serialize)]
pub struct CalibrationReport {
    pub best_concurrency: u32,
    pub addr_per_s: f64,
    pub rate_limited: bool,
}

/// Salves à concurrence croissante (1, 2, 4, … ≤ max) : mesure le débit de
/// chaque palier, s'arrête au premier 429 ou quand le gain devient < 15 %.
pub async fn calibrate(
    client: &ApiClient,
    sample: &[String],
    batch_size: usize,
    max_concurrency: u32,
) -> CalibrationReport {
    let mut best = (1u32, 0.0f64);
    let mut rate_limited = false;
    let mut level = 1u32;
    while level <= max_concurrency {
        let t0 = std::time::Instant::now();
        let mut handles = Vec::new();
        for i in 0..level {
            let client = client.clone();
            let chunk: Vec<String> = sample
                .iter()
                .cycle()
                .skip((i as usize * batch_size) % sample.len().max(1))
                .take(batch_size)
                .cloned()
                .collect();
            handles.push(tokio::spawn(async move { client.resolve_batch(&chunk).await }));
        }
        let mut ok = 0usize;
        for h in handles {
            match h.await {
                Ok(Ok((items, _))) => ok += items.len(),
                Ok(Err(ApiError::RateLimited { .. })) => rate_limited = true,
                _ => {}
            }
        }
        let throughput = ok as f64 / t0.elapsed().as_secs_f64().max(0.001);
        if throughput > best.1 * 1.15 {
            best = (level, throughput);
        } else {
            break; // le palier n'apporte plus assez : on garde le précédent
        }
        if rate_limited {
            break;
        }
        level *= 2;
    }
    CalibrationReport {
        best_concurrency: best.0,
        addr_per_s: best.1,
        rate_limited,
    }
}
```

- [ ] **Step 10 : Vérifier** — Run: `cargo test tests_calibrate` puis `cargo test` — Attendu : tout vert. Commit :

```bash
git add superpopaul/src-tauri/src
git commit -m "feat(superpopaul): calibrage de concurrence (paliers croissants, arrêt sur 429)"
```

---

### Task 10 : `output.rs` — génération du CSV de sortie

**Files:**
- Create: `superpopaul/src-tauri/src/output.rs`
- Modify: `superpopaul/src-tauri/src/lib.rs` (ajout `pub mod output;`)
- Modify: `superpopaul/src-tauri/Cargo.toml` (ajout `chrono = { version = "0.4", default-features = false, features = ["clock"] }`)

Le timestamp est injecté en paramètre (`stamp: Option<&str>`) : la fonction reste testable sans horloge.

- [ ] **Step 1 : Écrire les tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ColumnSpec, PeppolField};
    use crate::csv_io::CsvMeta;
    use crate::store::Resolution;
    use std::collections::HashMap;
    use std::io::Write;

    fn resolutions() -> HashMap<String, Resolution> {
        let mut m = HashMap::new();
        m.insert(
            "iso6523-actorid-upis::0009:1".to_string(),
            Resolution {
                participant: "iso6523-actorid-upis::0009:1".into(),
                exists_in_peppol: Some(true),
                pa_code: Some("PA0042".into()),
                pa_name: Some("ACME PA".into()),
                pa_country: Some("FR".into()),
                extended_ctc_fr: Some(false),
                api_status: "ok".into(),
                resolved_at: 0,
            },
        );
        m
    }

    #[test]
    fn sortie_une_ligne_par_ligne_d_entree_meme_pid_duplique() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("in.csv");
        std::fs::File::create(&input).unwrap()
            .write_all(b"siren;nom\n0009:1;ACME\n0009:2;GLOBEX\n0009:1;ACME BIS\n").unwrap();
        let out = dir.path().join("out.csv");
        let cols = vec![
            ColumnSpec::Input { name: "nom".into() },
            ColumnSpec::Peppol { field: PeppolField::Exists },
            ColumnSpec::Peppol { field: PeppolField::PaCode },
        ];
        let meta = CsvMeta { delimiter: b';', encoding: "utf-8" };
        let written = generate(&input, &meta, "siren", &cols, &resolutions(), &out, None).unwrap();
        let content = std::fs::read_to_string(&written).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 4); // entête + 3 lignes (autant que l'entrée)
        assert_eq!(lines[0], "nom;exists;pa_code");
        assert_eq!(lines[1], "ACME;true;PA0042");
        assert_eq!(lines[2], "GLOBEX;;");          // non résolu : colonnes vides
        assert_eq!(lines[3], "ACME BIS;true;PA0042"); // même PID → mêmes infos (base)
    }

    #[test]
    fn suffixe_timestamp_insere_avant_l_extension() {
        let p = with_stamp(std::path::Path::new("/tmp/out.csv"), Some("20260712-1430"));
        assert_eq!(p, std::path::PathBuf::from("/tmp/out_20260712-1430.csv"));
        let p2 = with_stamp(std::path::Path::new("/tmp/out.csv"), None);
        assert_eq!(p2, std::path::PathBuf::from("/tmp/out.csv"));
    }
}
```

- [ ] **Step 2 : Vérifier l'échec** — Run: `cargo test output` — Attendu : erreur de compilation.

- [ ] **Step 3 : Implémenter**

```rust
use crate::config::{ColumnSpec, PeppolField};
use crate::csv_io::CsvMeta;
use crate::pid::canonical;
use crate::store::Resolution;
use encoding_rs_io::DecodeReaderBytesBuilder;
use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

fn fmt_bool(b: Option<bool>) -> &'static str {
    match b {
        Some(true) => "true",
        Some(false) => "false",
        None => "",
    }
}

pub fn field_name(f: PeppolField) -> &'static str {
    match f {
        PeppolField::Exists => "exists",
        PeppolField::PaCode => "pa_code",
        PeppolField::PaName => "pa_name",
        PeppolField::PaCountry => "pa_country",
        PeppolField::ExtendedCtcFr => "extended_ctc_fr",
    }
}

/// Insère `_<stamp>` avant l'extension.
pub fn with_stamp(path: &Path, stamp: Option<&str>) -> PathBuf {
    match stamp {
        None => path.to_path_buf(),
        Some(s) => {
            let stem = path.file_stem().and_then(|x| x.to_str()).unwrap_or("sortie");
            let ext = path.extension().and_then(|x| x.to_str()).unwrap_or("csv");
            path.with_file_name(format!("{stem}_{s}.{ext}"))
        }
    }
}

/// Écrit le CSV de sortie : une ligne par ligne d'entrée, colonnes selon le
/// mapping, infos Peppol lues dans `resolutions` (la base). UTF-8 en sortie.
pub fn generate(
    input_path: &Path,
    meta: &CsvMeta,
    pid_column: &str,
    columns: &[ColumnSpec],
    resolutions: &HashMap<String, Resolution>,
    out_path: &Path,
    stamp: Option<&str>,
) -> Result<PathBuf, String> {
    let f = File::open(input_path).map_err(|e| format!("ouverture {input_path:?} : {e}"))?;
    let enc = if meta.encoding == "utf-8" { encoding_rs::UTF_8 } else { encoding_rs::WINDOWS_1252 };
    let decoded: Box<dyn Read> = Box::new(
        DecodeReaderBytesBuilder::new().encoding(Some(enc)).bom_sniffing(true).build(f),
    );
    let mut rdr = csv::ReaderBuilder::new()
        .delimiter(meta.delimiter)
        .flexible(true)
        .from_reader(decoded);
    let headers = rdr.headers().map_err(|e| e.to_string())?.clone();
    let pid_idx = headers
        .iter()
        .position(|h| h == pid_column)
        .ok_or_else(|| format!("Colonne '{pid_column}' absente de l'entête"))?;
    // Index des colonnes d'entrée du mapping, résolus une fois.
    let col_idx: Vec<Option<usize>> = columns
        .iter()
        .map(|c| match c {
            ColumnSpec::Input { name } => headers.iter().position(|h| h == name),
            ColumnSpec::Peppol { .. } => None,
        })
        .collect();
    for (c, idx) in columns.iter().zip(&col_idx) {
        if let (ColumnSpec::Input { name }, None) = (c, idx) {
            return Err(format!("Colonne d'entrée '{name}' absente de l'entête"));
        }
    }

    let final_path = with_stamp(out_path, stamp);
    let mut wtr = csv::WriterBuilder::new()
        .delimiter(meta.delimiter)
        .from_path(&final_path)
        .map_err(|e| format!("écriture {final_path:?} : {e}"))?;
    // Entête de sortie.
    let out_headers: Vec<String> = columns
        .iter()
        .map(|c| match c {
            ColumnSpec::Input { name } => name.clone(),
            ColumnSpec::Peppol { field } => field_name(*field).to_string(),
        })
        .collect();
    wtr.write_record(&out_headers).map_err(|e| e.to_string())?;

    for rec in rdr.records() {
        let rec = rec.map_err(|e| e.to_string())?;
        let raw_pid = rec.get(pid_idx).unwrap_or("");
        let res = resolutions.get(&canonical(raw_pid));
        let row: Vec<String> = columns
            .iter()
            .zip(&col_idx)
            .map(|(c, idx)| match c {
                ColumnSpec::Input { .. } => rec.get(idx.unwrap()).unwrap_or("").to_string(),
                ColumnSpec::Peppol { field } => match res {
                    None => String::new(),
                    Some(r) => match field {
                        PeppolField::Exists => fmt_bool(r.exists_in_peppol).to_string(),
                        PeppolField::PaCode => r.pa_code.clone().unwrap_or_default(),
                        PeppolField::PaName => r.pa_name.clone().unwrap_or_default(),
                        PeppolField::PaCountry => r.pa_country.clone().unwrap_or_default(),
                        PeppolField::ExtendedCtcFr => fmt_bool(r.extended_ctc_fr).to_string(),
                    },
                },
            })
            .collect();
        wtr.write_record(&row).map_err(|e| e.to_string())?;
    }
    wtr.flush().map_err(|e| e.to_string())?;
    Ok(final_path)
}
```

Ajouter `pub mod output;` dans `lib.rs` et `chrono` dans `Cargo.toml` (utilisé par Task 11 pour générer le stamp réel).

- [ ] **Step 4 : Vérifier** — Run: `cargo test output` — Attendu : `2 passed`.

- [ ] **Step 5 : Commit**

```bash
git add superpopaul/src-tauri
git commit -m "feat(superpopaul): output — CSV de sortie (mapping, jointure base, suffixe timestamp)"
```

---

## Phase 2 — Pont Tauri

### Task 11 : `commands.rs` + câblage `lib.rs`

**Files:**
- Create: `superpopaul/src-tauri/src/commands.rs`
- Modify: `superpopaul/src-tauri/src/lib.rs` (réécriture de `run()`)
- Modify: `superpopaul/src-tauri/capabilities/default.json`

Couche fine sans logique métier : pas de tests unitaires dédiés (la logique est déjà testée dans les modules ; le pont est vérifié par le smoke test UI de la Task 12).

- [ ] **Step 1 : Écrire `commands.rs`**

```rust
use crate::api::{ApiClient, CallStats, ProxyCreds};
use crate::config::{self, Config};
use crate::csv_io;
use crate::modes::{compute_todo, RunMode};
use crate::output;
use crate::pid::unique_canonical;
use crate::resolver::{calibrate, CalibrationReport, Engine, EngineEvent, EngineParams, RunHandle};
use crate::store::Store;
use serde::Serialize;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, State};

pub struct AppState {
    pub store: Arc<Mutex<Store>>,
    /// (répertoire du YAML si chargé/sauvé — base des chemins relatifs, config)
    pub config: Mutex<Option<(Option<PathBuf>, Config)>>,
    pub proxy_creds: Mutex<Option<ProxyCreds>>,
    pub run: Mutex<Option<Arc<RunHandle>>>,
}

impl AppState {
    pub fn new(store: Store) -> Self {
        AppState {
            store: Arc::new(Mutex::new(store)),
            config: Mutex::new(None),
            proxy_creds: Mutex::new(None),
            run: Mutex::new(None),
        }
    }

    fn current_config(&self) -> Result<(Option<PathBuf>, Config), String> {
        self.config.lock().unwrap().clone().ok_or_else(|| "Aucune configuration active.".into())
    }

    fn input_path(&self) -> Result<PathBuf, String> {
        let (base, cfg) = self.current_config()?;
        Ok(match base {
            Some(dir) => config::resolve_relative(&dir.join("x.yaml"), &cfg.input.path),
            None => PathBuf::from(&cfg.input.path),
        })
    }

    fn client(&self) -> Result<ApiClient, String> {
        let (_, cfg) = self.current_config()?;
        let creds = self.proxy_creds.lock().unwrap().clone();
        ApiClient::new(
            &cfg.api.url,
            &cfg.api.key,
            cfg.api.proxy.as_ref().map(|p| p.url.as_str()),
            creds.as_ref(),
        )
    }

    /// PIDs uniques canoniques du fichier d'entrée (lecture complète).
    fn unique_pids(&self) -> Result<Vec<String>, String> {
        let (_, cfg) = self.current_config()?;
        let path = self.input_path()?;
        let meta = csv_io::sniff(&path)?;
        let vals = csv_io::read_column(&path, &meta, &cfg.input.pid_column)?;
        Ok(unique_canonical(vals))
    }
}

#[derive(Serialize)]
pub struct PreviewPayload {
    #[serde(flatten)]
    pub preview: csv_io::Preview,
    pub suggested_pid_column: Option<usize>,
}

#[tauri::command]
pub async fn preview_csv(path: String) -> Result<PreviewPayload, String> {
    tokio::task::spawn_blocking(move || {
        let p = csv_io::preview(std::path::Path::new(&path), 5)?;
        let suggested = csv_io::suggest_pid_column(&p);
        Ok(PreviewPayload { preview: p, suggested_pid_column: suggested })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn set_config(state: State<'_, AppState>, cfg: Config) -> Result<(), String> {
    cfg.validate()?;
    let mut guard = state.config.lock().unwrap();
    let base = guard.as_ref().and_then(|(b, _)| b.clone());
    *guard = Some((base, cfg));
    Ok(())
}

#[tauri::command]
pub fn load_config(state: State<'_, AppState>, path: String) -> Result<Config, String> {
    let p = PathBuf::from(&path);
    let cfg = config::load(&p)?;
    *state.config.lock().unwrap() = Some((p.parent().map(PathBuf::from), cfg.clone()));
    Ok(cfg)
}

#[tauri::command]
pub fn save_config(state: State<'_, AppState>, path: String, cfg: Config) -> Result<(), String> {
    let p = PathBuf::from(&path);
    config::save(&p, &cfg)?;
    *state.config.lock().unwrap() = Some((p.parent().map(PathBuf::from), cfg));
    Ok(())
}

#[tauri::command]
pub fn set_proxy_creds(state: State<'_, AppState>, username: String, password: String) {
    *state.proxy_creds.lock().unwrap() = Some(ProxyCreds { username, password });
}

#[tauri::command]
pub fn update_api_key(state: State<'_, AppState>, key: String) -> Result<(), String> {
    if let Some((_, cfg)) = state.config.lock().unwrap().as_mut() {
        cfg.api.key = key.clone();
    }
    if let Some(h) = state.run.lock().unwrap().as_ref() {
        h.update_key(&key);
        h.set_paused(false);
    }
    Ok(())
}

#[tauri::command]
pub async fn test_api(state: State<'_, AppState>) -> Result<CallStats, String> {
    let client = state.client()?;
    client.test_key().await.map_err(|e| e.to_string())
}

#[derive(Serialize)]
pub struct InputStats {
    pub unique: usize,
    pub resolved_ok: usize,
    pub failed: usize,
    pub stale: usize,
    pub missing: usize,
}

/// Compare le fichier d'entrée à la base : alimente la popup de reprise et la
/// présélection du mode.
#[tauri::command]
pub async fn analyze_input(state: State<'_, AppState>) -> Result<InputStats, String> {
    let (_, cfg) = state.current_config()?;
    let pids = state.unique_pids()?;
    let known = state.store.lock().unwrap().load_map(&pids)?;
    let now = chrono::Utc::now().timestamp();
    let max_age = cfg.api.refresh_days as i64 * 86400;
    let (mut ok, mut failed, mut stale) = (0, 0, 0);
    for p in &pids {
        match known.get(p) {
            None => {}
            Some(r) if r.api_status != "ok" => failed += 1,
            Some(r) if r.resolved_at < now - max_age => stale += 1,
            Some(_) => ok += 1,
        }
    }
    Ok(InputStats {
        unique: pids.len(),
        resolved_ok: ok,
        failed,
        stale,
        missing: pids.len() - ok - failed - stale,
    })
}

#[tauri::command]
pub async fn calibrate_api(state: State<'_, AppState>) -> Result<CalibrationReport, String> {
    let (_, cfg) = state.current_config()?;
    let client = state.client()?;
    let mut sample = state.unique_pids()?;
    sample.truncate(64);
    if sample.is_empty() {
        return Err("Aucun adressage dans le fichier d'entrée.".into());
    }
    Ok(calibrate(&client, &sample, cfg.api.batch_size as usize, cfg.api.concurrency.max(16)).await)
}

#[tauri::command]
pub async fn start_run(
    app: AppHandle,
    state: State<'_, AppState>,
    mode: RunMode,
) -> Result<u64, String> {
    if state.run.lock().unwrap().is_some() {
        return Err("Un run est déjà en cours.".into());
    }
    let (_, cfg) = state.current_config()?;
    let pids = state.unique_pids()?;
    let now = chrono::Utc::now().timestamp();
    let todo = {
        let store = state.store.lock().unwrap();
        compute_todo(&mode, &pids, &store, now)?
    };
    let total = todo.len() as u64;
    let client = state.client()?;
    let (tx, mut rx) = tokio::sync::mpsc::channel(256);
    let handle = Arc::new(Engine::start(
        client,
        EngineParams {
            batch_size: cfg.api.batch_size as usize,
            concurrency: cfg.api.concurrency,
        },
        todo,
        state.store.clone(),
        tx,
    ));
    *state.run.lock().unwrap() = Some(handle);
    // Pont événements moteur → webview.
    tauri::async_runtime::spawn(async move {
        while let Some(ev) = rx.recv().await {
            match ev {
                EngineEvent::Telemetry(s) => { let _ = app.emit("telemetry", &s); }
                EngineEvent::Suspended { reason, message, retry_in_s } => {
                    let _ = app.emit("run-suspended", serde_json::json!({
                        "reason": reason, "message": message, "retry_in_s": retry_in_s
                    }));
                }
                EngineEvent::Resumed => { let _ = app.emit("run-resumed", serde_json::json!({})); }
                EngineEvent::Finished { done, failed, stopped } => {
                    let _ = app.emit("run-finished", serde_json::json!({
                        "done": done, "failed": failed, "stopped": stopped
                    }));
                    break;
                }
            }
        }
    });
    Ok(total)
}

#[tauri::command]
pub fn pause_run(state: State<'_, AppState>, paused: bool) -> Result<(), String> {
    match state.run.lock().unwrap().as_ref() {
        Some(h) => { h.set_paused(paused); Ok(()) }
        None => Err("Aucun run en cours.".into()),
    }
}

#[tauri::command]
pub fn stop_run(state: State<'_, AppState>) -> Result<(), String> {
    let mut guard = state.run.lock().unwrap();
    match guard.as_ref() {
        Some(h) => { h.request_stop(); *guard = None; Ok(()) }
        None => Err("Aucun run en cours.".into()),
    }
}

/// À appeler quand run-finished est reçu côté UI, pour libérer le slot.
#[tauri::command]
pub fn clear_run(state: State<'_, AppState>) {
    *state.run.lock().unwrap() = None;
}

#[tauri::command]
pub async fn generate_output(state: State<'_, AppState>) -> Result<String, String> {
    let (base, cfg) = state.current_config()?;
    let input = state.input_path()?;
    let meta = csv_io::sniff(&input)?;
    let pids = state.unique_pids()?;
    let resolutions = state.store.lock().unwrap().load_map(&pids)?;
    let out = match &base {
        Some(dir) => config::resolve_relative(&dir.join("x.yaml"), &cfg.output.path),
        None => PathBuf::from(&cfg.output.path),
    };
    let stamp = cfg
        .output
        .timestamp_suffix
        .then(|| chrono::Local::now().format("%Y%m%d-%H%M").to_string());
    let written = output::generate(
        &input, &meta, &cfg.input.pid_column, &cfg.output.columns,
        &resolutions, &out, stamp.as_deref(),
    )?;
    Ok(written.display().to_string())
}
```

- [ ] **Step 2 : Réécrire `lib.rs` (version finale)**

```rust
pub mod api;
pub mod commands;
pub mod config;
pub mod csv_io;
pub mod modes;
pub mod output;
pub mod pid;
pub mod resolver;
pub mod store;
pub mod telemetry;

use tauri::Manager;

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let dir = app.path().app_data_dir()?;
            let store = store::Store::open(&dir.join("superpopaul.db"))
                .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
            app.manage(commands::AppState::new(store));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::preview_csv,
            commands::set_config,
            commands::load_config,
            commands::save_config,
            commands::set_proxy_creds,
            commands::update_api_key,
            commands::test_api,
            commands::analyze_input,
            commands::calibrate_api,
            commands::start_run,
            commands::pause_run,
            commands::stop_run,
            commands::clear_run,
            commands::generate_output
        ])
        .run(tauri::generate_context!())
        .expect("erreur au lancement de Super Popaul");
}
```

- [ ] **Step 3 : Vérifier `capabilities/default.json`**

Doit contenir les permissions core et dialog :

```json
{
  "$schema": "gen/schemas/desktop-schema.json",
  "identifier": "default",
  "windows": ["main"],
  "permissions": ["core:default", "dialog:default"]
}
```

- [ ] **Step 4 : Vérifier** — Run: `cargo check && cargo test` — Attendu : compilation OK, tous les tests verts.

- [ ] **Step 5 : Commit**

```bash
git add superpopaul/src-tauri
git commit -m "feat(superpopaul): commandes Tauri + événements (pont moteur ↔ UI)"
```

---

## Phase 3 — Frontend (vanilla, thème sombre cockpit)

Les tâches UI se vérifient visuellement (`cargo tauri dev`) — pas de tests E2E (spec). Chaque tâche liste sa checklist de vérification manuelle. Créer une fois un CSV d'essai :

```bash
printf 'siren;nom\n0009:552100554;ACME\n0009:404833048;GLOBEX\n0009:552100554;ACME BIS\n' > /tmp/essai.csv
```

Palette (maquettes validées) : fond `#0d1117`, cartes `#161b22`, bordures `#30363d`, texte `#e6edf3`, secondaire `#8b949e`, vert `#3fb950`, bleu `#58a6ff`, ambre `#d29922`, rouge `#f85149`.

**Règle de sécurité UI** : le contenu d'un CSV et les messages d'erreur sont des données non fiables — le DOM dynamique est construit exclusivement via le helper `h()` (nœuds + `textContent`), jamais par `innerHTML`.

### Task 12 : coquille — HTML complet, CSS, navigation wizard, splash

**Files:**
- Modify: `superpopaul/src/index.html` (remplacement complet)
- Create: `superpopaul/src/styles.css`, `superpopaul/src/app.js`

- [ ] **Step 1 : Écrire `index.html`** (tout le markup statique de l'app ; les étapes 2-4 sont câblées par les tâches suivantes)

```html
<!doctype html>
<html lang="fr">
<head>
  <meta charset="utf-8">
  <title>Super Popaul</title>
  <link rel="stylesheet" href="styles.css">
</head>
<body>
  <div id="splash"><div class="splash-inner">🍿<h1>Super Popaul</h1><p>Résolution Peppol en masse</p></div></div>

  <header>
    <span class="logo">🍿 Super Popaul</span>
    <nav id="stepper">
      <button data-step="file"    class="step active">1. Fichier</button>
      <button data-step="columns" class="step" disabled>2. Colonnes</button>
      <button data-step="output"  class="step" disabled>3. Sortie &amp; API</button>
      <button data-step="run"     class="step" disabled>▶ Run</button>
    </nav>
    <span class="cfg-btns">
      <button id="btn-load-cfg">Charger…</button>
      <button id="btn-save-cfg">Sauvegarder…</button>
    </span>
  </header>

  <div id="banner" class="hidden"></div>

  <main>
    <!-- Étape 1 : fichier d'entrée -->
    <section id="step-file" class="panel">
      <h2>Fichier d'entrée</h2>
      <div id="dropzone">Dépose un fichier CSV ici, ou <button id="btn-browse">Parcourir…</button></div>
      <div id="file-info" class="hidden">
        <p id="file-meta" class="muted"></p>
        <table id="preview-table"></table>
        <p>Colonne des adressages : <select id="pid-column"></select>
           <span id="pid-hint" class="muted"></span></p>
      </div>
    </section>

    <!-- Étape 2 : colonnes de sortie (aperçu manipulable) -->
    <section id="step-columns" class="panel hidden">
      <h2>Colonnes du fichier de sortie</h2>
      <p class="muted">Glisse les en-têtes pour réordonner, ✕ pour exclure. L'aperçu montre le résultat final.</p>
      <div><button id="btn-add-col">+ Ajouter une colonne ⚡</button>
           <span id="add-col-menu" class="hidden"></span></div>
      <table id="out-preview"></table>
    </section>

    <!-- Étape 3 : sortie & API -->
    <section id="step-output" class="panel hidden">
      <h2>Sortie &amp; API</h2>
      <fieldset><legend>Fichier de sortie</legend>
        <p><input id="out-path" size="50"> <button id="btn-out-browse">…</button></p>
        <label><input type="checkbox" id="out-stamp" checked> Suffixer avec la date/heure</label>
      </fieldset>
      <fieldset><legend>API</legend>
        <p>URL <input id="api-url" size="40" value="https://peppol.gavini.cloud">
           Clé <input id="api-key" type="password" size="26">
           <button id="btn-test-api">Tester</button> <span id="api-test-result"></span></p>
        <p>Proxy (optionnel) <input id="proxy-url" size="30" placeholder="http://proxy:3128">
           <span class="muted">identifiants demandés au lancement, jamais enregistrés</span></p>
        <p>Concurrence <input id="api-conc" type="number" value="8" min="1" max="256">
           Taille de paquet <input id="api-batch" type="number" value="50" min="1" max="50">
           Ancienneté refresh (jours) <input id="api-refresh" type="number" value="30" min="1">
           <button id="btn-calibrate">Calibrer</button> <span id="calibrate-result"></span></p>
      </fieldset>
    </section>

    <!-- Étape 4 : cockpit -->
    <section id="step-run" class="panel hidden">
      <div id="run-header">
        <span id="run-title" class="muted"></span>
        <span>
          <select id="run-mode">
            <option value="full">Full — tout résoudre</option>
            <option value="reprise">Reprise — seulement les manquants</option>
            <option value="reprise-retry">Reprise + re-tenter les échecs</option>
            <option value="refresh">Refresh — manquants + périmés</option>
          </select>
          <button id="btn-start">▶ Lancer</button>
          <button id="btn-pause" class="hidden">⏸ Pause</button>
          <button id="btn-stop" class="hidden">■ Stop</button>
        </span>
      </div>
      <div id="cockpit" class="hidden">
        <div id="ring-block">
          <div id="ring"><div id="ring-center"><b id="ring-pct">0%</b><span id="ring-abs" class="muted"></span></div></div>
          <p>ETA <b id="eta">—</b></p>
        </div>
        <div id="tiles">
          <div class="tile">🟢 Dans Peppol<br><b id="t-exists">—</b></div>
          <div class="tile">🇫🇷 CTC-FR<br><b id="t-ctc">—</b></div>
          <div class="tile">Débit<br><b id="t-rate">—</b></div>
          <div class="tile">Concurrence / échecs<br><b id="t-misc">—</b></div>
        </div>
        <div class="wide">
          <div class="tile">Codes HTTP<div id="http-bars"></div><div id="http-legend" class="hbar-legend"></div></div>
          <div class="tile">Latence (ms)<div id="latency" class="hbar-legend"></div></div>
        </div>
      </div>
      <p id="run-result" class="hidden"></p>
    </section>
  </main>

  <footer>
    <button id="btn-prev" class="hidden">← Précédent</button>
    <button id="btn-next">Suivant →</button>
  </footer>

  <div id="modal-backdrop" class="hidden"><div id="modal"></div></div>

  <script src="app.js"></script>
  <script src="columns.js"></script>
  <script src="cockpit.js"></script>
</body>
</html>
```

(Note : `columns.js` et `cockpit.js` n'existent pas encore — créer deux fichiers vides à cette étape pour que la page charge sans erreur : `touch superpopaul/src/columns.js superpopaul/src/cockpit.js`.)

- [ ] **Step 2 : Écrire `styles.css`**

```css
:root {
  --bg: #0d1117; --card: #161b22; --border: #30363d;
  --fg: #e6edf3; --muted: #8b949e;
  --green: #3fb950; --blue: #58a6ff; --amber: #d29922; --red: #f85149;
}
* { box-sizing: border-box; }
body {
  margin: 0; background: var(--bg); color: var(--fg);
  font: 14px/1.5 -apple-system, "Segoe UI", system-ui, sans-serif;
  display: flex; flex-direction: column; min-height: 100vh;
}
h1, h2 { font-weight: 600; }
button {
  background: #21262d; color: var(--fg); border: 1px solid var(--border);
  border-radius: 6px; padding: 5px 14px; cursor: pointer;
}
button:hover:not(:disabled) { border-color: var(--blue); }
button:disabled { opacity: .45; cursor: default; }
input, select {
  background: var(--bg); color: var(--fg); border: 1px solid var(--border);
  border-radius: 6px; padding: 5px 8px;
}
.muted { color: var(--muted); }
.hidden { display: none !important; }

#splash {
  position: fixed; inset: 0; z-index: 99; background: var(--bg);
  display: flex; align-items: center; justify-content: center;
  font-size: 42px; transition: opacity .4s;
}
#splash.fade { opacity: 0; pointer-events: none; }
.splash-inner { text-align: center; }
.splash-inner h1 { margin: 8px 0 0; }
.splash-inner p { font-size: 14px; color: var(--muted); }

header {
  display: flex; align-items: center; gap: 16px;
  padding: 10px 16px; border-bottom: 1px solid var(--border);
}
.logo { font-weight: 700; }
#stepper { flex: 1; display: flex; gap: 6px; justify-content: center; }
.step { border-radius: 14px; }
.step.active { background: var(--blue); color: #fff; border-color: var(--blue); }
.step.done { color: var(--green); }

main { flex: 1; padding: 18px; overflow-y: auto; }
.panel { max-width: 900px; margin: 0 auto; }
fieldset { border: 1px solid var(--border); border-radius: 8px; margin: 12px 0; }

#banner {
  padding: 10px 16px; font-weight: 600; display: flex; gap: 12px;
  align-items: center; justify-content: center;
}
#banner.error { background: #3d1214; color: var(--red); }
#banner.warn { background: #3a2c10; color: var(--amber); }

#dropzone {
  border: 2px dashed var(--border); border-radius: 10px; padding: 40px;
  text-align: center; color: var(--muted);
}
#dropzone.over { border-color: var(--blue); color: var(--fg); }
table { border-collapse: collapse; margin: 12px 0; width: 100%; }
th, td { border: 1px solid var(--border); padding: 4px 10px; text-align: left; }
th { background: var(--card); }
#out-preview th { cursor: grab; user-select: none; white-space: nowrap; }
#out-preview th.peppol { color: var(--blue); border-color: var(--blue); }
#out-preview th.dragover { border-left: 3px solid var(--blue); }
th .rm { color: var(--muted); cursor: pointer; margin-left: 6px; }
th .rm:hover { color: var(--red); }
#add-col-menu button { margin: 4px 4px 0 0; }

#run-header { display: flex; justify-content: space-between; align-items: center; margin-bottom: 14px; }
#cockpit { display: flex; flex-wrap: wrap; gap: 14px; }
#ring-block { text-align: center; }
#ring {
  width: 150px; height: 150px; border-radius: 50%;
  background: conic-gradient(var(--green) 0%, #21262d 0%);
  display: flex; align-items: center; justify-content: center;
}
#ring-center {
  width: 116px; height: 116px; border-radius: 50%; background: var(--bg);
  display: flex; flex-direction: column; align-items: center; justify-content: center;
}
#ring-pct { font-size: 26px; }
#tiles { flex: 1; display: grid; grid-template-columns: 1fr 1fr; gap: 10px; }
.tile { background: var(--card); border-radius: 8px; padding: 10px 14px; }
.tile b { font-size: 18px; }
.wide { width: 100%; display: flex; gap: 10px; }
.wide .tile { flex: 1; }
.hbar { display: flex; height: 14px; border-radius: 4px; overflow: hidden; margin-top: 8px; }
.hbar span { display: block; height: 100%; }
.hbar-legend { font-size: 12px; color: var(--muted); margin-top: 4px; }

footer {
  display: flex; justify-content: space-between; padding: 10px 18px;
  border-top: 1px solid var(--border);
}
#modal-backdrop {
  position: fixed; inset: 0; background: rgba(0,0,0,.6); z-index: 50;
  display: flex; align-items: center; justify-content: center;
}
#modal {
  background: var(--card); border: 1px solid var(--border); border-radius: 10px;
  padding: 22px; max-width: 460px;
}
#modal input { display: block; margin: 8px 0; width: 100%; }
```

- [ ] **Step 3 : Écrire `app.js` (helper DOM sûr, état, navigation, splash, étape 1)**

```javascript
const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const { open, save } = window.__TAURI__.dialog;

const $ = (id) => document.getElementById(id);

/** Construit un élément DOM. Les enfants chaîne deviennent des nœuds texte :
 *  les données dynamiques (CSV, erreurs) ne passent JAMAIS par innerHTML. */
function h(tag, attrs = {}, ...children) {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (k.startsWith("on")) el.addEventListener(k.slice(2), v);
    else if (k === "class") el.className = v;
    else el.setAttribute(k, v);
  }
  el.append(...children);
  return el;
}

// --- État global -------------------------------------------------------------
const state = {
  inputPath: null,
  preview: null, // {headers, rows, delimiter, encoding, suggested_pid_column}
  config: {
    version: 1,
    api: { url: "https://peppol.gavini.cloud", key: "", batch_size: 50,
           concurrency: 8, proxy: null, refresh_days: 30 },
    input: { path: "", delimiter: ";", encoding: "utf-8", pid_column: "" },
    output: { path: "", timestamp_suffix: true, columns: [] },
  },
};

// --- Wizard --------------------------------------------------------------------
const STEPS = ["file", "columns", "output", "run"];
let current = 0;

function showStep(i) {
  current = i;
  STEPS.forEach((s, j) => {
    $(`step-${s}`).classList.toggle("hidden", j !== i);
    const btn = document.querySelector(`#stepper [data-step="${s}"]`);
    btn.classList.toggle("active", j === i);
    btn.classList.toggle("done", j < i);
    if (j <= i) btn.disabled = false;
  });
  $("btn-prev").classList.toggle("hidden", i === 0);
  $("btn-next").classList.toggle("hidden", i === STEPS.length - 1);
  if (STEPS[i] === "columns") renderOutPreview(); // columns.js
  if (STEPS[i] === "run") enterRunStep();          // cockpit.js
}

/** Message d'erreur si l'étape courante est incomplète, sinon null. */
function validateStep() {
  const s = STEPS[current];
  if (s === "file") {
    if (!state.inputPath) return "Choisis d'abord un fichier CSV.";
    if (!state.config.input.pid_column) return "Indique la colonne des adressages.";
  }
  if (s === "columns" && state.config.output.columns.length === 0)
    return "Il faut au moins une colonne en sortie.";
  if (s === "output") {
    syncOutputForm();
    if (!state.config.output.path) return "Indique le fichier de sortie.";
    if (!state.config.api.key) return "Saisis la clé API (bouton Tester pour vérifier).";
  }
  return null;
}

$("btn-next").addEventListener("click", () => {
  const err = validateStep();
  if (err) return banner("warn", err);
  hideBanner();
  showStep(current + 1);
});
$("btn-prev").addEventListener("click", () => { hideBanner(); showStep(current - 1); });
document.querySelectorAll("#stepper .step").forEach((b, j) =>
  b.addEventListener("click", () => !b.disabled && showStep(j)));

// --- Bannière / modale (textContent + nœuds : jamais d'innerHTML) --------------
function banner(kind, text, ...actionNodes) {
  const el = $("banner");
  el.className = kind;
  el.replaceChildren(text, ...actionNodes);
}
function hideBanner() { $("banner").className = "hidden"; }
function modal(...nodes) {
  $("modal").replaceChildren(...nodes);
  $("modal-backdrop").classList.remove("hidden");
}
function closeModal() { $("modal-backdrop").classList.add("hidden"); }

// --- Étape 1 : fichier -----------------------------------------------------------
async function pickInput(path) {
  try {
    const p = await invoke("preview_csv", { path });
    state.inputPath = path;
    state.preview = p;
    state.config.input = {
      path, delimiter: p.delimiter, encoding: p.encoding,
      pid_column: p.suggested_pid_column != null ? p.headers[p.suggested_pid_column] : "",
    };
    // Mapping par défaut : toutes les colonnes d'entrée + les 4 champs Peppol.
    state.config.output.columns = [
      ...p.headers.map((name) => ({ source: "input", name })),
      { source: "peppol", field: "exists" },
      { source: "peppol", field: "pa_code" },
      { source: "peppol", field: "pa_country" },
      { source: "peppol", field: "extended_ctc_fr" },
    ];
    if (!state.config.output.path)
      state.config.output.path = path.replace(/\.csv$/i, "") + "_enrichi.csv";
    renderFilePanel();
    hideBanner();
  } catch (e) {
    banner("error", `Impossible de lire ce fichier : ${e}`);
  }
}

function renderFilePanel() {
  const p = state.preview;
  $("file-info").classList.remove("hidden");
  $("file-meta").textContent =
    `${state.inputPath} — séparateur « ${p.delimiter} », encodage ${p.encoding}`;
  $("preview-table").replaceChildren(
    h("tr", {}, ...p.headers.map((hd) => h("th", {}, hd))),
    ...p.rows.map((r) => h("tr", {}, ...r.map((c) => h("td", {}, c)))),
  );
  $("pid-column").replaceChildren(...p.headers.map((hd) => {
    const o = h("option", {}, hd);
    o.selected = hd === state.config.input.pid_column;
    return o;
  }));
  $("pid-hint").textContent =
    p.suggested_pid_column != null ? "(suggestion automatique)" : "";
}

$("btn-browse").addEventListener("click", async () => {
  const f = await open({ multiple: false, filters: [{ name: "CSV", extensions: ["csv", "txt"] }] });
  if (f) pickInput(f);
});
$("pid-column").addEventListener("change", (e) => { state.config.input.pid_column = e.target.value; });
const dz = $("dropzone");
dz.addEventListener("dragover", (e) => { e.preventDefault(); dz.classList.add("over"); });
dz.addEventListener("dragleave", () => dz.classList.remove("over"));
// Le drop de fichier natif arrive par l'événement Tauri drag-drop.
listen("tauri://drag-drop", (e) => {
  const paths = e.payload.paths || [];
  if (paths.length && STEPS[current] === "file") pickInput(paths[0]);
  dz.classList.remove("over");
});

// --- Étape 3 : formulaire ↔ état ---------------------------------------------------
function syncOutputForm() {
  const c = state.config;
  c.output.path = $("out-path").value.trim();
  c.output.timestamp_suffix = $("out-stamp").checked;
  c.api.url = $("api-url").value.trim();
  c.api.key = $("api-key").value.trim();
  const proxyUrl = $("proxy-url").value.trim();
  c.api.proxy = proxyUrl ? { url: proxyUrl } : null;
  c.api.concurrency = +$("api-conc").value || 8;
  c.api.batch_size = +$("api-batch").value || 50;
  c.api.refresh_days = +$("api-refresh").value || 30;
}
function fillOutputForm() {
  const c = state.config;
  $("out-path").value = c.output.path;
  $("out-stamp").checked = c.output.timestamp_suffix;
  $("api-url").value = c.api.url;
  $("api-key").value = c.api.key;
  $("proxy-url").value = c.api.proxy ? c.api.proxy.url : "";
  $("api-conc").value = c.api.concurrency;
  $("api-batch").value = c.api.batch_size;
  $("api-refresh").value = c.api.refresh_days;
}
$("btn-out-browse").addEventListener("click", async () => {
  const f = await save({ filters: [{ name: "CSV", extensions: ["csv"] }] });
  if (f) $("out-path").value = f;
});

// --- Splash --------------------------------------------------------------------------
window.addEventListener("DOMContentLoaded", () => {
  fillOutputForm();
  setTimeout(() => $("splash").classList.add("fade"), 700);
});
```

- [ ] **Step 4 : Vérification manuelle**

Run: `cd superpopaul && cargo tauri dev` — Checklist :
- le splash 🍿 apparaît puis s'estompe ;
- « Parcourir… » ouvre le sélecteur natif ; choisir `/tmp/essai.csv` affiche entêtes + 3 lignes, séparateur « ; », colonne `siren` suggérée ;
- « Suivant » sans fichier affiche la bannière d'avertissement ;
- navigation Précédent/Suivant et stepper OK (étapes 2-4 encore squelettiques).

- [ ] **Step 5 : Commit**

```bash
git add superpopaul/src
git commit -m "feat(superpopaul): UI — coquille sombre, splash, wizard, étape fichier"
```

---

### Task 13 : `columns.js` — étape 2, aperçu manipulable

**Files:**
- Modify: `superpopaul/src/columns.js` (remplacement du fichier vide)

- [ ] **Step 1 : Écrire `columns.js`** (tout le DOM via `h()` — pas d'innerHTML)

```javascript
// Étape 2 : le tableau de sortie AVEC données d'exemple est l'outil de
// configuration : drag latéral des en-têtes, ✕ pour exclure, menu « + » pour
// ajouter champs Peppol ou colonnes exclues.
// Source de vérité : state.config.output.columns.

const PEPPOL_FIELDS = [
  ["exists", "exists"], ["pa_code", "code PA"], ["pa_name", "nom PA"],
  ["pa_country", "pays PA"], ["extended_ctc_fr", "CTC-FR"],
];
const PEPPOL_SAMPLE = { exists: "true", pa_code: "PA0042", pa_name: "ACME PA",
                        pa_country: "FR", extended_ctc_fr: "false" };

function colLabel(c) {
  return c.source === "input" ? c.name
       : "⚡ " + PEPPOL_FIELDS.find(([f]) => f === c.field)[1];
}

let dragFrom = null;

function makeHeader(c, i) {
  const rm = h("span", {
    class: "rm", title: "Exclure",
    onclick: (e) => {
      e.stopPropagation();
      state.config.output.columns.splice(i, 1);
      renderOutPreview();
    },
  }, "✕");
  const th = h("th", { class: c.source, draggable: "true" }, `⠿ ${colLabel(c)} `, rm);
  th.addEventListener("dragstart", () => { dragFrom = i; });
  th.addEventListener("dragover", (e) => { e.preventDefault(); th.classList.add("dragover"); });
  th.addEventListener("dragleave", () => th.classList.remove("dragover"));
  th.addEventListener("drop", (e) => {
    e.preventDefault();
    if (dragFrom === null || dragFrom === i) return;
    const cols = state.config.output.columns;
    cols.splice(i, 0, cols.splice(dragFrom, 1)[0]);
    dragFrom = null;
    renderOutPreview();
  });
  return th;
}

function renderOutPreview() {
  const cols = state.config.output.columns;
  const rows = state.preview ? state.preview.rows : [];
  const cell = (c, r) => {
    if (c.source === "peppol") return h("td", { class: "muted" }, PEPPOL_SAMPLE[c.field]);
    const idx = state.preview.headers.indexOf(c.name);
    return h("td", {}, idx >= 0 ? (r[idx] ?? "") : "");
  };
  $("out-preview").replaceChildren(
    h("tr", {}, ...cols.map(makeHeader)),
    ...rows.map((r) => h("tr", {}, ...cols.map((c) => cell(c, r)))),
  );
  renderAddColMenu();
}

/** Menu « + » : champs Peppol absents puis colonnes d'entrée exclues. */
function renderAddColMenu() {
  const cols = state.config.output.columns;
  const addBtn = (label, spec, cls) =>
    h("button", { class: cls || "", onclick: () => { cols.push(spec); renderOutPreview(); } }, label);
  const peppol = PEPPOL_FIELDS
    .filter(([f]) => !cols.some((c) => c.source === "peppol" && c.field === f))
    .map(([f, label]) => addBtn(`⚡ ${label}`, { source: "peppol", field: f }));
  const inputs = (state.preview ? state.preview.headers : [])
    .filter((name) => !cols.some((c) => c.source === "input" && c.name === name))
    .map((name) => addBtn(name, { source: "input", name }));
  const all = [...peppol, ...inputs];
  $("add-col-menu").replaceChildren(
    ...(all.length ? all : [h("span", { class: "muted" }, "tout est déjà inclus")]),
  );
}

$("btn-add-col").addEventListener("click", () =>
  $("add-col-menu").classList.toggle("hidden"));
```

- [ ] **Step 2 : Vérification manuelle**

Run: `cargo tauri dev`, charger `/tmp/essai.csv`, aller à l'étape 2 — Checklist :
- le tableau montre `siren`, `nom` + les 4 colonnes ⚡ (bleues) avec données d'exemple ;
- glisser un en-tête réordonne ; ✕ exclut ; « + » repropose la colonne exclue et `nom PA` ;
- tout exclure puis « Suivant » → bannière « au moins une colonne ».

- [ ] **Step 3 : Commit**

```bash
git add superpopaul/src
git commit -m "feat(superpopaul): UI — mapping colonnes par aperçu manipulable (DOM sûr)"
```

---

### Task 14 : étape 3 — test API et calibrage branchés

**Files:**
- Modify: `superpopaul/src/app.js` (ajout en fin de fichier)

- [ ] **Step 1 : Ajouter à `app.js`**

```javascript
// --- Étape 3 : test API et calibrage -----------------------------------------
$("btn-test-api").addEventListener("click", async () => {
  syncOutputForm();
  const out = $("api-test-result");
  out.textContent = "test en cours…";
  try {
    await invoke("set_config", { cfg: state.config });
    await ensureProxyCreds();
    const stats = await invoke("test_api");
    out.textContent = `✅ clé valide (${stats.latency_ms} ms)`;
  } catch (e) {
    out.textContent = `❌ ${e}`;
  }
});

$("btn-calibrate").addEventListener("click", async () => {
  syncOutputForm();
  const out = $("calibrate-result");
  out.textContent = "calibrage en cours…";
  try {
    await invoke("set_config", { cfg: state.config });
    await ensureProxyCreds();
    const r = await invoke("calibrate_api");
    $("api-conc").value = r.best_concurrency;
    state.config.api.concurrency = r.best_concurrency;
    out.textContent = `→ ${r.best_concurrency} sessions, ~${Math.round(r.addr_per_s)} adr/s` +
      (r.rate_limited ? " (clé rate-limitée)" : "");
  } catch (e) {
    out.textContent = `❌ ${e}`;
  }
});

/** Si un proxy est configuré et les identifiants pas encore saisis dans cette
 *  session, les demander (mémoire seulement — jamais persistés). */
let proxyCredsGiven = false;
async function ensureProxyCreds(force = false) {
  if (!state.config.api.proxy || (proxyCredsGiven && !force)) return;
  return new Promise((resolve) => {
    const user = h("input", { placeholder: "login" });
    const pass = h("input", { type: "password", placeholder: "mot de passe" });
    modal(
      h("h3", {}, "Identifiants proxy"),
      h("p", { class: "muted" }, "Conservés en mémoire uniquement, jamais enregistrés."),
      user, pass,
      h("button", {
        onclick: async () => {
          await invoke("set_proxy_creds", { username: user.value, password: pass.value });
          proxyCredsGiven = true;
          closeModal();
          resolve();
        },
      }, "Valider"),
    );
  });
}
```

- [ ] **Step 2 : Vérification manuelle**

Run: `cargo tauri dev` — Checklist :
- URL bidon → « Tester » affiche ❌ avec message réseau clair ;
- API réelle + clé valide → ✅ et latence ;
- URL proxy renseignée → la modale d'identifiants s'affiche avant le test ;
- sauvegarder ensuite un YAML (Task 16) et vérifier qu'il ne contient ni login ni mot de passe.

- [ ] **Step 3 : Commit**

```bash
git add superpopaul/src
git commit -m "feat(superpopaul): UI — test de clé, calibrage, identifiants proxy en mémoire"
```

---

### Task 15 : `cockpit.js` — dashboard temps réel, bannières intelligentes

**Files:**
- Modify: `superpopaul/src/cockpit.js` (remplacement du fichier vide)

- [ ] **Step 1 : Écrire `cockpit.js`**

```javascript
// Étape 4 : cockpit temps réel. Écoute les événements Rust :
// "telemetry" (4×/s), "run-suspended", "run-resumed", "run-finished".

let running = false;

async function enterRunStep() {
  $("run-title").textContent = state.inputPath ?? "";
  try {
    await invoke("set_config", { cfg: state.config });
    const s = await invoke("analyze_input");
    $("run-title").textContent = `${state.inputPath} — ${fmt(s.unique)} adressages uniques`;
    suggestMode(s);
  } catch (e) {
    banner("error", `${e}`);
  }
}

/** Aides intelligentes : détection de run incomplet, présélection du mode. */
function suggestMode(s) {
  const known = s.resolved_ok + s.failed + s.stale;
  if (running) return;
  if (s.missing > 0 && known > 0) {
    $("run-mode").value = "reprise";
    banner("warn",
      `Run incomplet détecté : ${fmt(known)}/${fmt(s.unique)} adressages déjà en base. `,
      h("button", { onclick: () => { hideBanner(); startRun(); } }, "Reprendre maintenant"));
  } else if (s.missing === 0 && s.unique > 0) {
    $("run-mode").value = "refresh";
    banner("warn", `Tous les adressages sont déjà en base (${fmt(s.stale)} périmés, ` +
      `${fmt(s.failed)} en échec) — mode refresh présélectionné.`);
  }
}

function modeFromSelect() {
  switch ($("run-mode").value) {
    case "full":          return { mode: "full" };
    case "reprise":       return { mode: "reprise", retry_failures: false };
    case "reprise-retry": return { mode: "reprise", retry_failures: true };
    case "refresh":       return { mode: "refresh", max_age_days: state.config.api.refresh_days };
  }
}

async function startRun() {
  try {
    await invoke("set_config", { cfg: state.config });
    await ensureProxyCreds();
    const total = await invoke("start_run", { mode: modeFromSelect() });
    running = true;
    $("cockpit").classList.remove("hidden");
    $("run-result").classList.add("hidden");
    $("btn-start").classList.add("hidden");
    $("btn-pause").classList.remove("hidden");
    $("btn-stop").classList.remove("hidden");
    hideBanner();
    if (total === 0) banner("warn", "Rien à résoudre dans ce mode — fichier généré directement.");
  } catch (e) {
    banner("error", `${e}`);
  }
}
$("btn-start").addEventListener("click", startRun);

$("btn-pause").addEventListener("click", async () => {
  const pausing = $("btn-pause").textContent.includes("Pause");
  await invoke("pause_run", { paused: pausing });
  $("btn-pause").textContent = pausing ? "▶ Reprendre" : "⏸ Pause";
});
$("btn-stop").addEventListener("click", () => invoke("stop_run"));

// --- Télémétrie -----------------------------------------------------------------
function httpColor(code) {
  if (code === 200) return "var(--green)";
  if (code === 429) return "var(--amber)";
  if (code === 0) return "var(--muted)";
  return code >= 500 ? "var(--red)" : code >= 400 ? "var(--amber)" : "var(--blue)";
}

listen("telemetry", (e) => {
  const s = e.payload;
  const pct = s.total ? (100 * s.done / s.total) : 0;
  $("ring").style.background = `conic-gradient(var(--green) ${pct}%, #21262d ${pct}%)`;
  $("ring-pct").textContent = `${pct.toFixed(pct < 10 ? 1 : 0)}%`;
  $("ring-abs").textContent = `${fmt(s.done)} / ${fmt(s.total)}`;
  $("eta").textContent = s.eta_s != null ? fmtDuration(s.eta_s) : "—";
  $("t-exists").textContent = s.done ? `${(100 * s.exists / s.done).toFixed(1)} %` : "—";
  $("t-ctc").textContent = s.done ? `${(100 * s.ctc / s.done).toFixed(1)} %` : "—";
  $("t-rate").textContent = `${s.req_per_s.toFixed(1)} req/s · ${Math.round(s.addr_per_s)} adr/s`;
  $("t-misc").textContent = `${fmt(s.failed)} échecs`;
  renderHttpBars(s.http);
  const l = s.latency;
  $("latency").textContent = l
    ? `min ${l.min} · moy ${l.mean} · p50 ${l.p50} · p90 ${l.p90} · p99 ${l.p99} · max ${l.max}`
    : "—";
});

function renderHttpBars(http) {
  const entries = Object.entries(http);
  const total = entries.reduce((a, [, n]) => a + n, 0) || 1;
  $("http-bars").replaceChildren(h("div", { class: "hbar" },
    ...entries.map(([code, n]) => {
      const span = h("span", {});
      span.style.width = `${(100 * n / total)}%`;
      span.style.background = httpColor(+code);
      return span;
    })));
  $("http-legend").textContent =
    entries.map(([c, n]) => `${c === "0" ? "réseau" : c}×${fmt(n)}`).join("   ");
}

function fmt(n) { return Number(n).toLocaleString("fr-FR"); }
function fmtDuration(s) {
  if (s < 60) return `${s} s`;
  const m = Math.round(s / 60);
  return m < 60 ? `${m} min` : `${Math.floor(m / 60)} h ${String(m % 60).padStart(2, "0")}`;
}

// --- Suspension / reprise / fin -------------------------------------------------
listen("run-suspended", (e) => {
  const { reason, message, retry_in_s } = e.payload;
  if (reason === "auth_api") {
    const key = h("input", { type: "password", placeholder: "nouvelle clé API" });
    banner("error", `⛔ ${message} Le traitement est en pause. `, key,
      h("button", { onclick: async () => {
        state.config.api.key = key.value;
        $("api-key").value = key.value;
        await invoke("update_api_key", { key: key.value });
        hideBanner();
      } }, "Reprendre avec cette clé"));
  } else if (reason === "auth_proxy") {
    banner("error", `⛔ ${message} `, h("button", { onclick: async () => {
      await ensureProxyCreds(true);
      await invoke("pause_run", { paused: false });
      hideBanner();
    } }, "Ressaisir les identifiants"));
  } else { // server_down
    banner("warn",
      `🛑 Serveur indisponible (${message}). Nouvel essai automatique dans ${retry_in_s} s. `,
      h("button", { onclick: () => invoke("pause_run", { paused: false }).then(hideBanner) },
        "Réessayer maintenant"));
  }
});
listen("run-resumed", hideBanner);

listen("run-finished", async (e) => {
  const { done, failed, stopped } = e.payload;
  running = false;
  await invoke("clear_run");
  $("btn-start").classList.remove("hidden");
  $("btn-pause").classList.add("hidden");
  $("btn-stop").classList.add("hidden");
  $("btn-pause").textContent = "⏸ Pause";
  const res = $("run-result");
  res.classList.remove("hidden");
  if (stopped) {
    res.replaceChildren(
      `Run arrêté : ${fmt(done)} résolus, rien n'est perdu (mode reprise pour continuer). `,
      h("button", { onclick: writeOutput }, "Générer quand même le fichier"));
  } else {
    res.textContent = `✅ Terminé : ${fmt(done)} résolus, ${fmt(failed)} échecs. Écriture du fichier…`;
    await writeOutput();
  }
});

async function writeOutput() {
  const res = $("run-result");
  try {
    const path = await invoke("generate_output");
    res.textContent = `✅ Fichier de sortie écrit : ${path}`;
  } catch (err) {
    res.textContent = `⚠️ Écriture du fichier impossible : ${err}`;
  }
}
```

- [ ] **Step 2 : Vérification manuelle (contre l'API réelle ou un mock local)**

Pour un test sans toucher la prod : `python3 ../peppol_api.py --help` (serveur local, voir README racine) ou l'API réelle avec `/tmp/essai.csv` (3 lignes, 2 adressages uniques). Checklist :
- « Lancer » en mode full : le ring monte, tuiles et latences bougent, codes HTTP s'accumulent ;
- Pause → le débit tombe à 0, Reprendre → ça repart ;
- Stop → message « rien n'est perdu » + bouton de génération ;
- relancer en reprise → 0 à résoudre (déjà en base) et fichier généré ;
- ouvrir le CSV généré : entête = mapping choisi, 3 lignes, valeurs Peppol identiques pour les 2 lignes ACME ;
- avec une clé invalide : bannière rouge + champ de ressaisie, la reprise fonctionne après correction.

- [ ] **Step 3 : Commit**

```bash
git add superpopaul/src
git commit -m "feat(superpopaul): UI — cockpit temps réel, bannières 401/407/5xx, génération sortie"
```

---

### Task 16 : sauvegarde/chargement YAML + reprise inter-sessions

**Files:**
- Modify: `superpopaul/src/app.js` (ajout en fin de fichier)
- Modify: `superpopaul/src-tauri/src/commands.rs` (commande `resolved_input_path`)
- Modify: `superpopaul/src-tauri/src/lib.rs` (enregistrer la commande)

- [ ] **Step 1 : Ajouter la commande backend**

Dans `commands.rs` :

```rust
/// Chemin absolu du fichier d'entrée (résolu relativement au YAML chargé) —
/// l'UI ne duplique pas la logique de résolution de chemins.
#[tauri::command]
pub fn resolved_input_path(state: State<'_, AppState>) -> Result<String, String> {
    Ok(state.input_path()?.display().to_string())
}
```

Dans `lib.rs`, ajouter `commands::resolved_input_path` à `generate_handler![…]`.

- [ ] **Step 2 : Ajouter à `app.js`**

```javascript
// --- Config YAML : sauvegarde / chargement -------------------------------------
$("btn-save-cfg").addEventListener("click", async () => {
  if ($("out-path").value) syncOutputForm();
  const f = await save({ filters: [{ name: "YAML", extensions: ["yaml", "yml"] }] });
  if (!f) return;
  try {
    await invoke("save_config", { path: f, cfg: state.config });
    banner("warn", "⚠️ Config enregistrée — la clé API y est stockée en clair. " +
      "Ne partage ce fichier qu'avec des collègues de confiance.");
  } catch (e) {
    banner("error", `${e}`);
  }
});

$("btn-load-cfg").addEventListener("click", async () => {
  const f = await open({ multiple: false, filters: [{ name: "YAML", extensions: ["yaml", "yml"] }] });
  if (!f) return;
  try {
    state.config = await invoke("load_config", { path: f });
    fillOutputForm();
    // Recharge l'aperçu du fichier d'entrée SANS écraser le mapping du YAML.
    const path = await invoke("resolved_input_path");
    state.preview = await invoke("preview_csv", { path });
    state.inputPath = path;
    renderFilePanel();
    hideBanner();
    showStep(3); // directement à l'étape Run (spec) — analyze_input y détecte la reprise
  } catch (e) {
    banner("error", `Chargement impossible : ${e}`);
  }
});
```

- [ ] **Step 3 : Vérifications manuelles**

Run: `cargo tauri dev` — Checklist :
- config complète → Sauvegarder → le YAML contient bien `url` du proxy mais **ni login ni mot de passe**, chemins et mapping fidèles (`cat` le fichier) ;
- relancer l'app → Charger → l'app saute à l'étape Run, l'aperçu et le formulaire sont repeuplés, le mapping de colonnes est celui du YAML (vérifier étape 2) ;
- avec une base partiellement remplie (lancer un run sur un CSV plus gros puis Stop) : recharger le YAML → bannière « Run incomplet détecté … Reprendre maintenant ».

- [ ] **Step 4 : Commit**

```bash
git add superpopaul/src superpopaul/src-tauri/src
git commit -m "feat(superpopaul): config YAML save/load + détection de run incomplet"
```

---

## Phase 4 — Distribution

### Task 17 : icônes, builds, notice, garde-fou de taille

**Files:**
- Create: `superpopaul/scripts/make_icon.py`, `superpopaul/NOTICE-OUVERTURE.md`
- Create: `.github/workflows/superpopaul-windows.yml` (racine du repo)

- [ ] **Step 1 : Icône placeholder (stdlib pure, remplaçable plus tard)**

`superpopaul/scripts/make_icon.py` :

```python
"""Génère app-icon.png (1024×1024, aplat orange popcorn) sans dépendance."""
import struct, zlib

W = H = 1024
row = b"\x00" + bytes([245, 166, 35, 255]) * W  # filtre None + RGBA #f5a623

def chunk(tag: bytes, data: bytes) -> bytes:
    return (struct.pack(">I", len(data)) + tag + data
            + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF))

png = (b"\x89PNG\r\n\x1a\n"
       + chunk(b"IHDR", struct.pack(">IIBBBBB", W, H, 8, 6, 0, 0, 0))
       + chunk(b"IDAT", zlib.compress(row * H))
       + chunk(b"IEND", b""))
open("app-icon.png", "wb").write(png)
print("app-icon.png écrit")
```

```bash
cd superpopaul && python3 scripts/make_icon.py && cargo tauri icon app-icon.png
```

Attendu : `src-tauri/icons/` régénéré (icns, ico, png).

- [ ] **Step 2 : Build macOS + garde-fou de taille**

```bash
cd superpopaul/src-tauri && cargo tauri build
APP="target/release/bundle/macos/Super Popaul.app"
du -sh "$APP"
SIZE=$(du -sk "$APP" | cut -f1)
test "$SIZE" -lt 20480 && echo "OK taille < 20 Mo" || echo "TROP GROS : arbitrage nécessaire (spec)"
ditto -c -k --keepParent "$APP" "target/release/bundle/macos/SuperPopaul-macos.zip"
```

Attendu : `OK taille < 20 Mo` (une release Tauri 2 stripped+LTO tourne autour de 6-12 Mo).

- [ ] **Step 3 : `NOTICE-OUVERTURE.md`**

```markdown
# Ouvrir Super Popaul (binaires non signés)

## macOS
1. Dézippe `SuperPopaul-macos.zip`.
2. **Clic droit** sur `Super Popaul.app` → **Ouvrir** → bouton **Ouvrir**.
   (Un double-clic direct est bloqué par Gatekeeper : c'est normal, l'app
   n'est pas signée. Le clic droit > Ouvrir n'est nécessaire qu'une fois.)
   Si macOS refuse quand même : Réglages Système → Confidentialité et
   sécurité → « Ouvrir quand même ».

## Windows
1. Lance `Super Popaul.exe`.
2. Si SmartScreen affiche « Windows a protégé votre ordinateur » :
   **Informations complémentaires** → **Exécuter quand même**.
3. Prérequis : WebView2 (préinstallé sur Windows 10/11 récents). Si l'app
   ne démarre pas, installer « WebView2 Evergreen » depuis le site Microsoft.
```

- [ ] **Step 4 : Workflow GitHub Actions Windows**

`.github/workflows/superpopaul-windows.yml` :

```yaml
name: superpopaul-windows
on:
  workflow_dispatch:
  push:
    tags: ["superpopaul-v*"]
jobs:
  build:
    runs-on: windows-latest
    defaults:
      run: { working-directory: superpopaul }
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
        with: { workspaces: "superpopaul/src-tauri" }
      - name: Install tauri-cli
        run: cargo install tauri-cli --locked
      - name: Build
        run: cargo tauri build
      - name: Check size (< 20 Mo)
        shell: bash
        run: |
          EXE="src-tauri/target/release/superpopaul.exe"
          SIZE=$(stat -c%s "$EXE")
          echo "Taille : $((SIZE / 1024 / 1024)) Mo"
          test "$SIZE" -lt 20971520
      - uses: actions/upload-artifact@v4
        with:
          name: SuperPopaul-windows
          path: |
            superpopaul/src-tauri/target/release/superpopaul.exe
            superpopaul/src-tauri/target/release/bundle/nsis/*.exe
```

(`superpopaul.exe` nu = binaire portable ; le bundle NSIS est fourni en bonus pour qui préfère un installeur.)

- [ ] **Step 5 : Vérifier le workflow**

```bash
git add superpopaul .github/workflows/superpopaul-windows.yml
git commit -m "feat(superpopaul): distribution — icônes, notice, CI Windows, garde-fou 20 Mo"
git push && gh workflow run superpopaul-windows --ref main
gh run watch   # attendu : job vert, artefact SuperPopaul-windows présent
```

- [ ] **Step 6 : Mettre à jour le README racine** (section « clients ») : une ligne mentionnant `superpopaul/` comme client graphique de l'API, à côté de `popaul.py`/`popaul.ps1`. Commit :

```bash
git add README.md
git commit -m "docs(readme): mentionne Super Popaul (client graphique)"
```

---

## Auto-revue du plan (exécutée à la rédaction)

1. **Couverture spec** : wizard (T12-14), mapping aperçu manipulable (T13), cockpit ring/tuiles/latences/codes HTTP/2 débits/ETA (T8, T15), modes full/reprise/refresh (T5), base globale (T4, lib.rs T11), YAML sans credentials proxy (T3, T14), pause/stop/reprise inter-sessions (T9, T15, T16), 401/407/429/5xx + AIMD + breaker (T7, T9, T15), calibrage + ETA (T9, T14), suffixe timestamp (T10), splash (T12), suggestion colonne PID (T6), distribution non signée + notice + taille (T17). Pas de trou identifié.
2. **Placeholders** : aucun TBD/TODO ; chaque étape code contient le code.
3. **Cohérence des types** : `RunMode` sérialisé `{mode: "reprise", retry_failures}` identique côté `modes.rs` (serde tag) et `modeFromSelect()` ; `Snapshot` (telemetry.rs) ↔ champs lus par `cockpit.js` (`done,total,exists,ctc,failed,http,latency,req_per_s,addr_per_s,eta_s`) ; `ColumnSpec` `{source,name|field}` identique config.rs ↔ columns.js ↔ output.rs ; événements `telemetry`/`run-suspended`/`run-resumed`/`run-finished` émis (T11) = écoutés (T15).

## Exécution

Deux options de mise en œuvre (voir en-tête) : subagent-driven (un subagent frais par tâche, revue entre chaque) ou exécution inline avec checkpoints.






