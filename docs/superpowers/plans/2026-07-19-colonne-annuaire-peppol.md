# Colonne « annuaire Peppol » (in_directory) — plan d'implémentation

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ajouter une colonne calculée `in_directory` au tableau de sortie : présence déclarative de l'adressage dans `peppol_directory` (jointure exacte 0225 → true/false, vide hors-0225), distincte de « existe » (provisionné), avec accent vert 📇.

**Architecture:** `store::directory_present` (quelles valeurs 0225 sont dans la table) → `output::generate` gagne un `directory: Option<&HashSet>` et calcule la cellule `in_directory` hors du gate résolution → `commands::generate_output` calcule la présence si la colonne est demandée et l'annuaire chargé → frontend : champ 📇 vert « annuaire Peppol » + bannière si annuaire non chargé.

**Tech Stack:** Rust (rusqlite, chrono), Tauri v2, JS vanilla.

**Spec :** `docs/superpowers/specs/2026-07-19-colonne-annuaire-peppol-design.md`

---

## Structure des fichiers

- `client/src-tauri/src/store.rs` — `directory_present` (+ tests).
- `client/src-tauri/src/config.rs` — variant `PeppolField::InDirectory` (+ test serde).
- `client/src-tauri/src/output.rs` — `field_name`, signature `generate` + calcul (+ tests).
- `client/src-tauri/src/commands.rs` — `generate_output` : calcul de la présence.
- `client/src/columns.js` — champ, libellé, icône, classe d'accent.
- `client/src/styles.css` — accent vert (`th.dir`, `.chip.dir`).
- `client/src/cockpit.js` — bannière annuaire non chargé.

Tests Rust depuis `client/src-tauri/` : `cargo test <filtre>` (pas de flag `--nocolor`).

---

## Task 1 : `store.rs` — `directory_present`

**Files:** Modify `client/src-tauri/src/store.rs`

- [ ] **Step 1 : Écrire les tests d'abord**

Dans `mod tests` de `store.rs`, ajouter :

```rust
    #[test]
    fn directory_present_renvoie_le_sous_ensemble_present() {
        let s = Store::open_in_memory().unwrap();
        s.replace_peppol_directory(&["a".into(), "b".into(), "c".into()], "file", 1).unwrap();
        let got = s.directory_present(&["a".into(), "x".into(), "c".into()]).unwrap();
        assert_eq!(got.len(), 2);
        assert!(got.contains("a") && got.contains("c") && !got.contains("x"));
    }

    #[test]
    fn directory_present_traverse_plusieurs_lots() {
        let s = Store::open_in_memory().unwrap();
        let vals: Vec<String> = (0..600).map(|i| format!("v{i}")).collect();
        s.replace_peppol_directory(&vals, "file", 1).unwrap();
        assert_eq!(s.directory_present(&vals).unwrap().len(), 600);
    }

    #[test]
    fn directory_present_annuaire_vide() {
        let s = Store::open_in_memory().unwrap();
        s.replace_peppol_directory(&[], "file", 1).unwrap(); // table existe, vide
        assert!(s.directory_present(&["a".into()]).unwrap().is_empty());
    }
```

- [ ] **Step 2 : Lancer — doit échouer**

Run : `cargo test store::tests::directory_present`
Expected : FAIL — `no method named directory_present`.

- [ ] **Step 3 : Importer HashSet + implémenter**

Dans `store.rs`, remplacer l'import `use std::collections::HashMap;` par :
```rust
use std::collections::{HashMap, HashSet};
```

Dans `impl Store`, ajouter (à côté de `load_map`) :
```rust
    /// Sous-ensemble de `values` réellement présents dans `peppol_directory`.
    /// Par lots de 500 (limite de variables SQLite), motif `load_map`. La table
    /// doit exister : appeler après `peppol_directory_status()` == Some.
    pub fn directory_present(&self, values: &[String]) -> Result<HashSet<String>, String> {
        let mut out = HashSet::new();
        for chunk in values.chunks(500) {
            let placeholders = vec!["?"; chunk.len()].join(",");
            let sql = format!("SELECT value FROM peppol_directory WHERE value IN ({placeholders})");
            let mut stmt = self.conn.prepare_cached(&sql).map_err(|e| e.to_string())?;
            let rows = stmt
                .query_map(rusqlite::params_from_iter(chunk), |r| r.get::<_, String>(0))
                .map_err(|e| e.to_string())?;
            for v in rows {
                out.insert(v.map_err(|e| e.to_string())?);
            }
        }
        Ok(out)
    }
```

- [ ] **Step 4 : Lancer — doit passer**

Run : `cargo test store::tests`
Expected : PASS (les 3 nouveaux + existants).

- [ ] **Step 5 : Commit**

```bash
git add client/src-tauri/src/store.rs
git commit -m "feat(superpopaul): store::directory_present — quelles valeurs 0225 sont dans l'annuaire"
```
(trailer `Claude-Session: https://claude.ai/code/session_01TDtNYu8g39HHSUxDrYSGPs`)

---

## Task 2 : `config.rs` + `output.rs` — champ InDirectory et calcul

**Files:** Modify `client/src-tauri/src/config.rs`, `client/src-tauri/src/output.rs`, `client/src-tauri/src/commands.rs` (une ligne : passer `None` au nouveau paramètre)

- [ ] **Step 1 : Écrire les tests output d'abord**

Dans `mod tests` de `output.rs`, ajouter :

```rust
    #[test]
    fn in_directory_true_false_vide_selon_annuaire() {
        // Ligne "111" (0225 présent) → true ; "222" (0225 absent) → false ;
        // "0009:333" (non-0225) → vide. resolutions VIDE : prouve que le calcul
        // ne dépend pas de la résolution.
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("in.csv");
        std::fs::File::create(&input).unwrap()
            .write_all(b"siren\n111\n222\n0009:333\n").unwrap();
        let out = dir.path().join("out.csv");
        let meta = CsvMeta { delimiter: b';', encoding: "utf-8" };
        let mut set = std::collections::HashSet::new();
        set.insert("111".to_string());
        let cols = vec![ColumnSpec::Peppol { field: PeppolField::InDirectory }];
        let written = generate(&input, &meta, "siren", &out_cfg(cols),
                               &HashMap::new(), Some(&set), &out, None).unwrap();
        let content = std::fs::read_to_string(&written).unwrap();
        let lines: Vec<&str> = content.trim_start_matches('\u{feff}').lines().collect();
        assert_eq!(lines[0], "in_directory");
        assert_eq!(lines[1], "true", "0225 présent dans l'annuaire");
        assert_eq!(lines[2], "false", "0225 absent");
        assert_eq!(lines[3], "", "non-0225 → vide");
    }

    #[test]
    fn in_directory_vide_si_annuaire_non_charge() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("in.csv");
        std::fs::File::create(&input).unwrap().write_all(b"siren\n111\n").unwrap();
        let out = dir.path().join("out.csv");
        let meta = CsvMeta { delimiter: b';', encoding: "utf-8" };
        let cols = vec![ColumnSpec::Peppol { field: PeppolField::InDirectory }];
        // directory = None → colonne vide même pour un 0225.
        let written = generate(&input, &meta, "siren", &out_cfg(cols),
                               &HashMap::new(), None, &out, None).unwrap();
        let content = std::fs::read_to_string(&written).unwrap();
        let lines: Vec<&str> = content.trim_start_matches('\u{feff}').lines().collect();
        assert_eq!(lines[0], "in_directory");
        assert_eq!(lines[1], "", "annuaire non chargé → vide");
    }
```

Dans `mod tests` de `config.rs`, ajouter (garde le nom sérialisé = en-tête CSV) :
```rust
    #[test]
    fn peppol_field_in_directory_serialise_snake_case() {
        assert_eq!(serde_yaml::to_string(&PeppolField::InDirectory).unwrap().trim(), "in_directory");
        assert_eq!(serde_yaml::from_str::<PeppolField>("in_directory").unwrap(), PeppolField::InDirectory);
    }
```
(Si `PeppolField` n'est pas déjà importé dans `config.rs::tests`, il l'est via `use super::*;` — vérifier la présence de cette ligne dans le mod tests, sinon utiliser `super::PeppolField`.)

- [ ] **Step 2 : Lancer — doit échouer (compilation)**

Run : `cargo test output::tests::in_directory`
Expected : FAIL — variant `InDirectory` inconnu / `generate` prend 7 arguments.

- [ ] **Step 3 : Ajouter le variant**

Dans `config.rs`, enum `PeppolField`, après `CtcStatus,` :
```rust
    CtcStatus,
    /// Présence dans l'annuaire Peppol (table peppol_directory, déclaratif) —
    /// calculée par jointure, indépendamment de la résolution.
    InDirectory,
```

- [ ] **Step 4 : output.rs — imports, field_name, signature, calcul**

Dans `output.rs` :

(a) imports — remplacer `use std::collections::HashMap;` par :
```rust
use std::collections::{HashMap, HashSet};
```
et ajouter, sous `use crate::pid::canonical;` :
```rust
use crate::directory::parse_0225_value;
```

(b) `field_name` — ajouter le bras :
```rust
        PeppolField::CtcStatus => "ctc_status",
        PeppolField::InDirectory => "in_directory",
```

(c) signature `generate` — insérer `directory` après `resolutions` :
```rust
    resolutions: &HashMap<String, Resolution>,
    directory: Option<&HashSet<String>>,
    out_path: &Path,
```

(d) boucle des lignes — remplacer le bloc actuel (calcul de `res` + le `.map(...)`) par :
```rust
        let raw_pid = rec.get(pid_idx).unwrap_or("");
        let cpid = canonical(raw_pid);
        let res = resolutions.get(&cpid);
        // Présence annuaire : hors du gate `res` (un déclaré non provisionné
        // n'a pas de Resolution mais doit ressortir "true").
        let in_dir: &str = match directory {
            None => "",
            Some(set) => match parse_0225_value(&cpid) {
                Some(v) if set.contains(&v) => "true",
                Some(_) => "false",
                None => "",
            },
        };
        let row: Vec<&str> = columns
            .iter()
            .zip(&col_idx)
            .map(|(c, idx)| match c {
                ColumnSpec::Input { .. } => rec.get(idx.unwrap()).unwrap_or(""),
                ColumnSpec::Peppol { field: PeppolField::InDirectory } => in_dir,
                ColumnSpec::Peppol { field } => match res {
                    None => "",
                    Some(r) => match field {
                        PeppolField::InPeppol => fmt_bool(r.exists_in_peppol),
                        PeppolField::PaCode => r.pa_code.as_deref().unwrap_or(""),
                        PeppolField::PaName => r.pa_name.as_deref().unwrap_or(""),
                        PeppolField::PaCountry => r.pa_country.as_deref().unwrap_or(""),
                        PeppolField::UblExtended => fmt_bool(r.extended_ctc_fr),
                        PeppolField::CtcActivation => r.ctc_activation.as_deref().unwrap_or(""),
                        PeppolField::CtcExpiration => r.ctc_expiration.as_deref().unwrap_or(""),
                        PeppolField::CtcStatus => ctc_status(r, now),
                        PeppolField::InDirectory => unreachable!("traité par le bras dédié ci-dessus"),
                    },
                },
            })
            .collect();
```

- [ ] **Step 5 : Mettre à jour les appelants de `generate` (param ajouté)**

Run : `cargo build`
Le compilateur liste chaque appel `generate(...)` auquel il manque un argument.
- Dans `output.rs` (mod tests) : pour CHAQUE appel `generate(...)` existant, insérer `None` en 6ᵉ position — juste après l'argument des résolutions (`&m` / `&resolutions()` / `&HashMap::new()`) et avant `&out`. (Les 2 nouveaux tests passent déjà `Some(&set)`/`None`.)
- Dans `commands.rs` `generate_output` : dans l'appel `output::generate(...)`, insérer `None` après `&resolutions` (temporaire — remplacé en Task 3).

- [ ] **Step 6 : Lancer les tests**

Run : `cargo test` (depuis `client/src-tauri/`)
Expected : PASS, zéro régression (les tests CTC/output existants inchangés dans leur attendu).

- [ ] **Step 7 : Commit**

```bash
git add client/src-tauri/src/config.rs client/src-tauri/src/output.rs client/src-tauri/src/commands.rs
git commit -m "feat(superpopaul): champ in_directory — présence annuaire calculée à l'export"
```
(trailer)

---

## Task 3 : `commands.rs` — calcul de la présence dans generate_output

**Files:** Modify `client/src-tauri/src/commands.rs`

- [ ] **Step 1 : Importer ColumnSpec + PeppolField**

Remplacer `use crate::config::{self, ApiMode, Config};` par :
```rust
use crate::config::{self, ApiMode, ColumnSpec, Config, PeppolField};
```

- [ ] **Step 2 : Calculer `directory` et le passer à generate**

Dans `generate_output`, à l'intérieur du `spawn_blocking`, remplacer le corps depuis le `load_map` jusqu'à l'appel `output::generate(...)` par :

```rust
        let resolutions = store.lock().unwrap().load_map(&pids)?;
        // Présence annuaire : uniquement si la colonne est demandée ET
        // l'annuaire chargé (sinon None → colonne vide côté output).
        let wants_dir = cfg
            .output
            .columns
            .iter()
            .any(|c| matches!(c, ColumnSpec::Peppol { field: PeppolField::InDirectory }));
        let directory = if wants_dir {
            let s = store.lock().unwrap();
            if s.peppol_directory_status()?.is_some() {
                let vals: Vec<String> = pids
                    .iter()
                    .filter_map(|p| crate::directory::parse_0225_value(p))
                    .collect();
                Some(s.directory_present(&vals)?)
            } else {
                None
            }
        } else {
            None
        };
        let out = resolved_out_dir(&input, &cfg.output.dir)
            .join(output::out_file_name(&input, &cfg.output.suffix));
        let stamp = cfg
            .output
            .timestamp_suffix
            .then(|| chrono::Local::now().format("%Y%m%d-%H%M").to_string());
        let written = output::generate(
            &input,
            &meta,
            &cfg.input.pid_column,
            &cfg.output,
            &resolutions,
            directory.as_ref(),
            &out,
            stamp.as_deref(),
        )?;
        Ok(written.display().to_string())
```

- [ ] **Step 3 : Compiler + tests**

Run : `cargo build` puis `cargo test`
Expected : compile sans warning ; suite verte.

- [ ] **Step 4 : Commit**

```bash
git add client/src-tauri/src/commands.rs
git commit -m "feat(superpopaul): generate_output calcule la présence annuaire pour in_directory"
```
(trailer)

---

## Task 4 : `columns.js` — champ, libellé 📇, classe d'accent

**Files:** Modify `client/src/columns.js`

- [ ] **Step 1 : Ajouter le champ et l'échantillon**

`PEPPOL_FIELDS` — ajouter l'entrée finale :
```js
  ["ctc_status", "état CTC"],
  ["in_directory", "annuaire Peppol"],
];
```
`PEPPOL_SAMPLE` — ajouter la clé :
```js
                        ctc_status: "later", in_directory: "true" };
```

- [ ] **Step 2 : Icône par champ dans `colLabel`**

Remplacer `colLabel` par :
```js
function colLabel(c) {
  if (c.source === "input") return c.name;
  const icon = c.field === "in_directory" ? "📇" : "⚡";
  return icon + " " + PEPPOL_FIELDS.find(([f]) => f === c.field)[1];
}
```

- [ ] **Step 3 : Helper de classe visuelle + usages**

Juste après `colLabel`, ajouter :
```js
// Classe CSS visuelle d'une colonne (accent). La source « peppol » reste la
// vérité métier ; seul l'accent visuel diffère pour l'annuaire (vert).
function colClass(c) {
  if (c.source === "input") return isPidSpec(c) ? "input pid" : "input";
  return c.field === "in_directory" ? "dir" : "peppol";
}
```

Dans `makeHeader`, remplacer :
```js
  const attrs = { class: pid ? "input pid" : c.source, "data-key": colKey(c) };
```
par :
```js
  const attrs = { class: colClass(c), "data-key": colKey(c) };
```

Dans le rendu de la zone (`$("col-zone").replaceChildren(...)`), remplacer :
```js
    h("div", { class: `chip ${c.source}`, "data-key": colKey(c) }, `⠿ ${colLabel(c)}`)));
```
par :
```js
    h("div", { class: `chip ${colClass(c)}`, "data-key": colKey(c) }, `⠿ ${colLabel(c)}`)));
```

- [ ] **Step 4 : Vérification syntaxe**

Run : `node --check client/src/columns.js`
Expected : OK. (Le rendu visuel est vérifié en Task 7.)

- [ ] **Step 5 : Commit**

```bash
git add client/src/columns.js
git commit -m "feat(superpopaul): colonne annuaire Peppol — champ, libellé 📇, accent dédié (columns.js)"
```
(trailer)

---

## Task 5 : `styles.css` — accent vert de l'annuaire

**Files:** Modify `client/src/styles.css`

- [ ] **Step 1 : Ajouter les règles d'accent vert**

Après la règle `#out-preview th.peppol { … }`, ajouter :
```css
#out-preview th.dir { color: var(--green); box-shadow: inset 0 0 0 1px var(--green); }
```
Après la règle `.chip.peppol { … }`, ajouter :
```css
.chip.dir { color: var(--green); border-color: var(--green); }
```

- [ ] **Step 2 : Vérification**

Vérifier que `--green` existe dans `:root` (oui : `--green: #4cc268;`) et que les accolades sont équilibrées.

- [ ] **Step 3 : Commit**

```bash
git add client/src/styles.css
git commit -m "feat(superpopaul): accent vert de la colonne annuaire Peppol (th.dir / .chip.dir)"
```
(trailer)

---

## Task 6 : `cockpit.js` — bannière annuaire non chargé

**Files:** Modify `client/src/cockpit.js`

- [ ] **Step 1 : Avertir si la colonne est présente et l'annuaire non chargé**

Dans `writeOutput()`, dans le bloc `try`, APRÈS le `row.replaceChildren(...)` du succès (et avant le `} catch`), ajouter :
```js
    const hasDir = state.config.output.columns.some(
      (c) => c.source === "peppol" && c.field === "in_directory");
    if (hasDir) {
      const st = await invoke("directory_status").catch(() => null);
      if (!st)
        banner("warn",
          "La colonne « annuaire Peppol » est vide : l'annuaire n'a pas été chargé (onglet Fichiers).");
    }
```
(`state`, `invoke`, `banner` sont globaux — définis dans app.js, partagés sans bundler.)

- [ ] **Step 2 : Vérification syntaxe**

Run : `node --check client/src/cockpit.js`
Expected : OK.

- [ ] **Step 3 : Commit**

```bash
git add client/src/cockpit.js
git commit -m "feat(superpopaul): avertit si la colonne annuaire est demandée sans annuaire chargé"
```
(trailer)

---

## Task 7 : Vérification end-to-end

**Files:** aucun (vérification).

- [ ] **Step 1 : Contrôle automatisé de la jointure sur données réelles**

Écrire un exemple JETABLE `client/src-tauri/examples/verif_jointure.rs` (non commité) qui : charge l'annuaire réel via `stream_0225_values` + `replace_peppol_directory`, prend un échantillon de valeurs 0225 présentes et une valeur bidon, et vérifie `directory_present` :
```rust
use std::fs::File;
use std::io::BufReader;
use superpopaul_lib::directory::stream_0225_values;
use superpopaul_lib::store::Store;
fn main() {
    let path = std::env::args().nth(1).unwrap();
    let vals = stream_0225_values(BufReader::new(File::open(&path).unwrap()), |_| {}).unwrap();
    let store = Store::open_in_memory().unwrap();
    store.replace_peppol_directory(&vals, "file", 1).unwrap();
    let sample: Vec<String> = vals.iter().take(3).cloned().collect();
    let mut probe = sample.clone();
    probe.push("SIREN_BIDON_ABSENT".into());
    let present = store.directory_present(&probe).unwrap();
    println!("échantillon présents attendus 3 → {}", sample.iter().filter(|v| present.contains(*v)).count());
    println!("bidon absent (attendu false) → {}", present.contains("SIREN_BIDON_ABSENT"));
}
```
Run : `cargo run --release --example verif_jointure -- ../../deaddrop/in/export-all-participants.csv`
Expected : « présents attendus 3 → 3 » et « bidon absent → false ». Puis **supprimer** l'exemple (`rm client/src-tauri/examples/verif_jointure.rs`), vérifier `git status` propre.

- [ ] **Step 2 : Vérification GUI manuelle (par l'utilisateur)**

Lancer le client. Charger un fichier + l'annuaire (onglet Fichiers). Onglet Format : glisser la puce **📇 annuaire Peppol** (vert) dans le tableau. Onglet Run : générer. Contrôler dans le CSV : `in_directory` = true/false sur des adressages 0225 connus, vide sur un non-0225. Puis, annuaire non chargé (base neuve) : générer avec la colonne → colonne vide + **bannière** d'avertissement.

- [ ] **Step 3 : Commit éventuel de finition** (sinon rien).

---

## Auto-revue

- **Couverture spec** : `directory_present` (Task 1) ; variant + calcul true/false/vide + indépendance résolution + None→vide (Task 2) ; wiring generate_output avec garde colonne+chargé (Task 3) ; champ/libellé/📇/accent (Task 4-5) ; bannière non chargé (Task 6) ; vérif réelle (Task 7). Hors périmètre (match SIREN, autres schemes, blocage) non implémenté — conforme.
- **Placeholders** : aucun — code réel à chaque step.
- **Cohérence des types** : `PeppolField::InDirectory` (config) ↔ `field_name`→`"in_directory"` (output) ↔ `["in_directory","annuaire Peppol"]` (columns.js) ↔ `c.field === "in_directory"` (columns.js, cockpit.js). `generate(..., directory: Option<&HashSet<String>>, ...)` ↔ `directory.as_ref()` (commands) ↔ `Some(&set)`/`None` (tests). `directory_present(&[String]) -> HashSet<String>` ↔ `Some(s.directory_present(&vals)?)`. Accent : classe `dir` (columns.js) ↔ `th.dir` / `.chip.dir` (styles.css). OK.
