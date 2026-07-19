# Champs PPF (annuaire_ppf, ppf_active, pdp_definie, ppf_usable) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ajouter quatre champs de sortie booléens (`annuaire_ppf`, `ppf_active`, `pdp_definie`, `ppf_usable`) calculés par jointure de chaque adressage avec `ppf_directory`, sélectionnables comme colonnes dans l'étape 2 (onglet Format).

**Architecture:** Miroir strict de la colonne `in_directory`. Une méthode `store::ppf_flags` agrège les 4 drapeaux par identifiant (une requête SQL `GROUP BY`), `output::generate` reçoit la map et l'applique par ligne hors du gate `res`, `commands::generate_output` construit la map sous garde (colonne demandée + annuaire non vide). Le frontend ajoute les 4 champs avec l'icône 🏛️ et un accent violet en dégradé (4 classes CSS).

**Tech Stack:** Rust (rusqlite, csv, serde) côté `client/src-tauri/` ; HTML/CSS/JS vanilla côté `client/src/`. Tests : `cargo test` (aucun test JS ni CLI — PPF est client-only).

**Spec de référence :** `docs/superpowers/specs/2026-07-19-champs-ppf-format-design.md`

---

## File Structure

- `client/src-tauri/src/store.rs` — **Modify** : struct `PpfFlags` + méthode `ppf_flags` (près de `ppf_summary`). Tests dans le `mod tests` existant.
- `client/src-tauri/src/config.rs` — **Modify** : 4 variantes dans l'enum `PeppolField` (l. 229-245).
- `client/src-tauri/src/output.rs` — **Modify** : import `PpfFlags` ; `field_name` (l. 34-45) ; signatures `generate`/`write_output` (+ param `ppf`) ; calcul par ligne (l. 262-299) ; tests.
- `client/src-tauri/src/commands.rs` — **Modify** : gate `ppf` dans `generate_output` (l. 528-564).
- `client/src/columns.js` — **Modify** : `PEPPOL_FIELDS`, `PEPPOL_SAMPLE`, `colLabel`, `colClass`, `makeHeader` (tooltips).
- `client/src/styles.css` — **Modify** : variables `--ppf-l1..l4` + règles `th.ppf-*` / `.chip.ppf-*`.

Aucun fichier créé. Aucun changement `cli/`, `server/`, `report.rs`.

---

## Task 1 : `store::PpfFlags` + `ppf_flags`

**Files:**
- Modify: `client/src-tauri/src/store.rs` (ajouter struct + méthode près de `ppf_summary`, l. ~399 ; tests dans `mod tests`)

- [ ] **Step 1: Écrire les tests qui échouent**

Ajouter dans le `mod tests` de `store.rs` (le helper `ppf_row(id, motif, pdp)` existe déjà l. 701) :

```rust
    #[test]
    fn ppf_flags_calcule_les_quatre_drapeaux() {
        let s = Store::open_in_memory().unwrap();
        // id_all    : présent, motif V + pdp fictive → annuaire seul.
        // id_active : motif C, pdp fictive → active seul.
        // id_pdp    : motif V, pdp réelle → pdp_definie seul.
        // id_split  : (C,1) + (V,0) → active ET pdp_definie mais PAS usable.
        // id_usable : (P,0) → les quatre vrais.
        s.ingest_ppf(
            "f.csv",
            "h",
            &[
                ppf_row("id_all", "V", 1),
                ppf_row("id_active", "C", 1),
                ppf_row("id_pdp", "V", 0),
                ppf_row("id_split", "C", 1),
                ppf_row("id_split", "V", 0),
                ppf_row("id_usable", "P", 0),
            ],
            6,
            1,
        )
        .unwrap();

        let ids: Vec<String> =
            ["id_all", "id_active", "id_pdp", "id_split", "id_usable", "id_absent"]
                .iter()
                .map(|x| x.to_string())
                .collect();
        let m = s.ppf_flags(&ids).unwrap();

        assert_eq!(
            m.get("id_all").copied(),
            Some(PpfFlags { in_ppf: true, active: false, pdp_definie: false, usable: false })
        );
        assert_eq!(
            m.get("id_active").copied(),
            Some(PpfFlags { in_ppf: true, active: true, pdp_definie: false, usable: false })
        );
        assert_eq!(
            m.get("id_pdp").copied(),
            Some(PpfFlags { in_ppf: true, active: false, pdp_definie: true, usable: false })
        );
        assert_eq!(
            m.get("id_split").copied(),
            Some(PpfFlags { in_ppf: true, active: true, pdp_definie: true, usable: false }),
            "C et pdp=0 sur des lignes DIFFÉRENTES → usable faux"
        );
        assert_eq!(
            m.get("id_usable").copied(),
            Some(PpfFlags { in_ppf: true, active: true, pdp_definie: true, usable: true })
        );
        assert!(!m.contains_key("id_absent"), "absent de l'annuaire → pas dans la map");
    }

    #[test]
    fn ppf_flags_traverse_plusieurs_lots() {
        let s = Store::open_in_memory().unwrap();
        let rows: Vec<_> = (0..600).map(|i| ppf_row(&format!("id{i}"), "C", 0)).collect();
        s.ingest_ppf("f.csv", "h", &rows, 600, 1).unwrap();
        let ids: Vec<String> = (0..600).map(|i| format!("id{i}")).collect();
        let m = s.ppf_flags(&ids).unwrap();
        assert_eq!(m.len(), 600);
        assert!(m.values().all(|f| f.usable), "tous (C,0) → usable");
    }
```

- [ ] **Step 2: Lancer les tests → échec de compilation**

Run: `cd client/src-tauri && cargo test ppf_flags`
Expected: FAIL — `cannot find type PpfFlags` / `no method named ppf_flags`.

- [ ] **Step 3: Implémenter la struct et la méthode**

Ajouter dans `store.rs`, juste avant `impl Store` OU parmi les structs publiques de tête (à côté de `PpfSummary`/`PpfFile`), la struct :

```rust
/// Drapeaux PPF dérivés d'un identifiant par jointure sur `ppf_directory`
/// (un identifiant = ≥1 ligne (motif, pdp_fictive)). `in_ppf` est toujours
/// vrai pour un identifiant présent ; les absents ne sont pas dans la map.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PpfFlags {
    pub in_ppf: bool,      // annuaire_ppf — ≥1 ligne
    pub active: bool,      // ppf_active   — ≥1 ligne motif C|P
    pub pdp_definie: bool, // pdp_definie  — ≥1 ligne pdp_fictive=0
    pub usable: bool,      // ppf_usable   — ≥1 même ligne (C|P) ET pdp_fictive=0
}
```

Ajouter la méthode dans `impl Store`, après `ppf_summary` :

```rust
    /// Drapeaux PPF pour chaque `identifiant` présent en table (les absents ne
    /// figurent pas dans la map → tout `false` côté appelant). Par lots de 500
    /// (limite de variables SQLite), motif `directory_present`. `ppf_usable`
    /// exige (motif C|P) ET pdp_fictive=0 sur la MÊME ligne, d'où l'agrégat
    /// `MAX(... AND ...)` distinct de `active AND pdp_definie`.
    pub fn ppf_flags(&self, identifiants: &[String]) -> Result<HashMap<String, PpfFlags>, String> {
        let mut out = HashMap::new();
        for chunk in identifiants.chunks(500) {
            let placeholders = vec!["?"; chunk.len()].join(",");
            let sql = format!(
                "SELECT identifiant, \
                        MAX(motif IN ('C','P')), \
                        MAX(pdp_fictive = 0), \
                        MAX(motif IN ('C','P') AND pdp_fictive = 0) \
                 FROM ppf_directory WHERE identifiant IN ({placeholders}) \
                 GROUP BY identifiant"
            );
            let mut stmt = self.conn.prepare_cached(&sql).map_err(|e| e.to_string())?;
            let rows = stmt
                .query_map(rusqlite::params_from_iter(chunk), |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        PpfFlags {
                            in_ppf: true,
                            active: r.get::<_, i64>(1)? != 0,
                            pdp_definie: r.get::<_, i64>(2)? != 0,
                            usable: r.get::<_, i64>(3)? != 0,
                        },
                    ))
                })
                .map_err(|e| e.to_string())?;
            for row in rows {
                let (id, flags) = row.map_err(|e| e.to_string())?;
                out.insert(id, flags);
            }
        }
        Ok(out)
    }
```

Note : `HashMap` et `rusqlite::params_from_iter` sont déjà importés/utilisés dans `store.rs` (`load_map`, `directory_present`).

- [ ] **Step 4: Lancer les tests → succès**

Run: `cd client/src-tauri && cargo test ppf_flags`
Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
git add client/src-tauri/src/store.rs
git commit -m "feat(superpopaul): store::ppf_flags — 4 drapeaux PPF par identifiant (jointure ppf_directory)"
```

---

## Task 2 : enum `PeppolField` + calcul dans `output.rs`

Les variantes de l'enum et les deux `match` exhaustifs d'`output.rs` (`field_name` l. 34-45 et construction de ligne l. 287-295) sont couplés par l'exhaustivité Rust : cette tâche les traite ensemble et thread le nouveau paramètre `ppf`.

**Files:**
- Modify: `client/src-tauri/src/config.rs` (enum, l. 229-245)
- Modify: `client/src-tauri/src/output.rs` (import, `field_name`, signatures `generate`/`write_output`, calcul par ligne, tests)
- Modify: `client/src-tauri/src/commands.rs` (call site `generate` — ajout `None` temporaire, câblé en Task 3)

- [ ] **Step 1: Écrire les tests de sortie qui échouent**

Ajouter dans le `mod tests` d'`output.rs` (à côté de `in_directory_true_false_vide_selon_annuaire`). D'abord, compléter l'import du module de test : remplacer `use crate::store::Resolution;` par `use crate::store::{PpfFlags, Resolution};`.

```rust
    #[test]
    fn ppf_champs_true_false_vide() {
        // "111" présent+usable ; "222" présent annuaire seul ; "0009:333"
        // (non-0225) → vide. resolutions VIDE : le calcul ne dépend pas de la
        // résolution.
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("in.csv");
        std::fs::File::create(&input)
            .unwrap()
            .write_all(b"siren\n111\n222\n0009:333\n")
            .unwrap();
        let out = dir.path().join("out.csv");
        let meta = CsvMeta { delimiter: b';', encoding: "utf-8" };
        let mut map = HashMap::new();
        map.insert(
            "111".to_string(),
            PpfFlags { in_ppf: true, active: true, pdp_definie: true, usable: true },
        );
        map.insert(
            "222".to_string(),
            PpfFlags { in_ppf: true, active: false, pdp_definie: false, usable: false },
        );
        let cols = vec![
            ColumnSpec::Peppol { field: PeppolField::AnnuairePpf },
            ColumnSpec::Peppol { field: PeppolField::PpfUsable },
        ];
        let written = generate(
            &input, &meta, "siren", &out_cfg(cols), &HashMap::new(), None, Some(&map), &out, None,
        )
        .unwrap();
        let content = std::fs::read_to_string(&written).unwrap();
        let lines: Vec<&str> = content.trim_start_matches('\u{feff}').lines().collect();
        assert_eq!(lines[0], "annuaire_ppf;ppf_usable");
        assert_eq!(lines[1], "true;true", "111 usable");
        assert_eq!(lines[2], "true;false", "222 annuaire seul");
        assert_eq!(lines[3], ";", "non-0225 → deux vides");
    }

    #[test]
    fn ppf_champs_vides_si_annuaire_ppf_absent() {
        // ppf = None → les 4 colonnes vides même pour un 0225.
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("in.csv");
        std::fs::File::create(&input).unwrap().write_all(b"siren\n111\n").unwrap();
        let out = dir.path().join("out.csv");
        let meta = CsvMeta { delimiter: b';', encoding: "utf-8" };
        let cols = vec![
            ColumnSpec::Peppol { field: PeppolField::AnnuairePpf },
            ColumnSpec::Peppol { field: PeppolField::PpfActive },
            ColumnSpec::Peppol { field: PeppolField::PdpDefinie },
            ColumnSpec::Peppol { field: PeppolField::PpfUsable },
        ];
        let written = generate(
            &input, &meta, "siren", &out_cfg(cols), &HashMap::new(), None, None, &out, None,
        )
        .unwrap();
        let content = std::fs::read_to_string(&written).unwrap();
        let lines: Vec<&str> = content.trim_start_matches('\u{feff}').lines().collect();
        assert_eq!(lines[0], "annuaire_ppf;ppf_active;pdp_definie;ppf_usable");
        assert_eq!(lines[1], ";;;", "annuaire PPF absent → 4 vides");
    }
```

- [ ] **Step 2: Lancer → échec de compilation**

Run: `cd client/src-tauri && cargo test ppf_champs`
Expected: FAIL — variantes `AnnuairePpf`/`PpfActive`/`PdpDefinie`/`PpfUsable` inconnues, `generate` prend 8 arguments pas 9.

- [ ] **Step 3: Ajouter les 4 variantes à l'enum**

Dans `config.rs`, à la fin de l'enum `PeppolField` (après `InDirectory`, avant l'accolade fermante l. 245) :

```rust
    /// Jointure avec l'annuaire PPF (`ppf_directory`, client-only). Chaque champ
    /// est indépendant de la résolution et de l'annuaire Peppol.
    /// `annuaire_ppf` : identifiant présent (≥1 ligne).
    AnnuairePpf,
    /// ≥1 ligne au motif C ou P.
    PpfActive,
    /// ≥1 ligne avec pdp_fictive = 0.
    PdpDefinie,
    /// ≥1 même ligne (motif C|P) ET pdp_fictive = 0.
    PpfUsable,
```

- [ ] **Step 4: Ajouter les 4 en-têtes dans `field_name`**

Dans `output.rs`, dans `field_name` (après `PeppolField::InDirectory => "in_directory",` l. 44) :

```rust
        PeppolField::AnnuairePpf => "annuaire_ppf",
        PeppolField::PpfActive => "ppf_active",
        PeppolField::PdpDefinie => "pdp_definie",
        PeppolField::PpfUsable => "ppf_usable",
```

- [ ] **Step 5: Importer `PpfFlags` et threader le paramètre `ppf`**

Dans `output.rs` en tête (l. 5), remplacer `use crate::store::Resolution;` par :

```rust
use crate::store::{PpfFlags, Resolution};
```

Dans la signature de `generate` (l. 87-96), ajouter le paramètre après `directory` :

```rust
    directory: Option<&HashSet<String>>,
    ppf: Option<&HashMap<String, PpfFlags>>,
    out_path: &Path,
```

Dans la signature de `write_output` (l. 176-184), idem après `directory` :

```rust
    directory: Option<&HashSet<String>>,
    ppf: Option<&HashMap<String, PpfFlags>>,
    tmp_path: &Path,
```

Dans `generate`, propager `ppf` à l'appel interne de `write_output` (l. ~118) — ajouter `ppf` après `directory` :

```rust
    if let Err(e) = write_output(input_path, meta, pid_column, output, resolutions, directory, ppf, &tmp_path) {
```

- [ ] **Step 6: Calculer les 4 cellules par ligne**

Dans `write_output`, dans la boucle par ligne (l. 262-299), remplacer le bloc actuel (calcul `in_dir` + construction `row`) par la version qui partage `parse_0225_value` et calcule les 4 cellules PPF. Le bloc actuel est :

```rust
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
```

Le remplacer par :

```rust
        let cpid = canonical(raw_pid);
        let res = resolutions.get(&cpid);
        // Valeur 0225 (partagée annuaire Peppol + PPF), calculée une fois.
        let v0225 = parse_0225_value(&cpid);
        // Présence annuaire Peppol : hors du gate `res` (un déclaré non
        // provisionné n'a pas de Resolution mais doit ressortir "true").
        let in_dir: &str = match directory {
            None => "",
            Some(set) => match &v0225 {
                Some(v) if set.contains(v) => "true",
                Some(_) => "false",
                None => "",
            },
        };
        // Drapeaux PPF (hors gate `res`, comme in_dir). None = annuaire vide OU
        // non-0225 → cellule vide ; Some(défaut) = 0225 absent de l'annuaire →
        // tout "false".
        let ppf_flags: Option<PpfFlags> = match (ppf, &v0225) {
            (Some(map), Some(v)) => Some(map.get(v).copied().unwrap_or_default()),
            _ => None,
        };
        let ppf_ann = match ppf_flags { Some(f) => fmt_bool(Some(f.in_ppf)), None => "" };
        let ppf_act = match ppf_flags { Some(f) => fmt_bool(Some(f.active)), None => "" };
        let ppf_pdp = match ppf_flags { Some(f) => fmt_bool(Some(f.pdp_definie)), None => "" };
        let ppf_use = match ppf_flags { Some(f) => fmt_bool(Some(f.usable)), None => "" };
```

Puis, dans la construction de `row` (le `.map(|(c, idx)| match c { ... })`), ajouter les 4 bras dédiés juste après le bras `InDirectory` (l. 283) :

```rust
                ColumnSpec::Peppol { field: PeppolField::InDirectory } => in_dir,
                ColumnSpec::Peppol { field: PeppolField::AnnuairePpf } => ppf_ann,
                ColumnSpec::Peppol { field: PeppolField::PpfActive } => ppf_act,
                ColumnSpec::Peppol { field: PeppolField::PdpDefinie } => ppf_pdp,
                ColumnSpec::Peppol { field: PeppolField::PpfUsable } => ppf_use,
```

Et dans le `match field` interne (branche `Some(r)`, l. 286-295), ajouter les 4 bras `unreachable!` (l'exhaustivité l'exige) après `InDirectory` (l. 295) :

```rust
                        PeppolField::InDirectory => unreachable!("traité par le bras dédié ci-dessus"),
                        PeppolField::AnnuairePpf
                        | PeppolField::PpfActive
                        | PeppolField::PdpDefinie
                        | PeppolField::PpfUsable => {
                            unreachable!("champs PPF traités par les bras dédiés ci-dessus")
                        }
```

- [ ] **Step 7: Réparer les appels existants de `generate`**

`generate` prend désormais 9 arguments : insérer l'argument `ppf` (un `None` sauf indication) **juste après l'argument `directory`** dans chaque appel existant.

- `commands.rs` (l. ~561) : après `directory.as_ref(),` ajouter `None,` (le vrai gate arrive en Task 3).
- `output.rs`, tous les appels `generate(...)` des tests **autres** que les deux nouveaux : insérer `None,` après l'argument directory. Repérage : les appels passant `..., None, &out, None)` deviennent `..., None, None, &out, None)` ; celui passant `..., Some(&set), &out, None)` (test `in_directory_true_false_vide...`) devient `..., Some(&set), None, &out, None)`.

Le compilateur signale chaque site manqué (erreur d'arité) — `cargo build` doit finir sans erreur avant l'étape suivante.

Run: `cd client/src-tauri && cargo build`
Expected: compile sans erreur.

- [ ] **Step 8: Lancer les tests → succès**

Run: `cd client/src-tauri && cargo test`
Expected: PASS (dont `ppf_champs_true_false_vide`, `ppf_champs_vides_si_annuaire_ppf_absent`, et tous les tests `in_directory_*` inchangés).

- [ ] **Step 9: Commit**

```bash
git add client/src-tauri/src/config.rs client/src-tauri/src/output.rs client/src-tauri/src/commands.rs
git commit -m "feat(superpopaul): 4 champs PPF en sortie (config + output, jointure par ligne)"
```

---

## Task 3 : gate `ppf` dans `commands::generate_output`

Câble la map réelle sous garde. Pas de test unitaire : `generate_output` est lié à Tauri `State` (comme le gate `directory`, non testé unitairement) ; vérifié en Task 6 via l'app. Le comportement pur est déjà couvert par les tests `output.rs` de la Task 2.

**Files:**
- Modify: `client/src-tauri/src/commands.rs` (l. 528-564)

- [ ] **Step 1: Construire la map PPF sous garde**

Dans `generate_output`, juste après le bloc `let directory = if wants_dir { ... };` (fin l. 548), ajouter :

```rust
        // Drapeaux PPF : uniquement si une colonne PPF est demandée ET
        // l'annuaire PPF est non vide (sinon None → colonnes vides). Miroir du
        // gate `directory` ci-dessus.
        let wants_ppf = cfg.output.columns.iter().any(|c| {
            matches!(
                c,
                ColumnSpec::Peppol { field: PeppolField::AnnuairePpf }
                    | ColumnSpec::Peppol { field: PeppolField::PpfActive }
                    | ColumnSpec::Peppol { field: PeppolField::PdpDefinie }
                    | ColumnSpec::Peppol { field: PeppolField::PpfUsable }
            )
        });
        let ppf = if wants_ppf {
            let s = store.lock().unwrap();
            if s.ppf_summary()?.distinct_addr > 0 {
                let ids: Vec<String> = pids
                    .iter()
                    .filter_map(|p| crate::directory::parse_0225_value(p))
                    .collect();
                Some(s.ppf_flags(&ids)?)
            } else {
                None
            }
        } else {
            None
        };
```

- [ ] **Step 2: Passer la map à `generate`**

Dans l'appel `output::generate(...)` (l. 555-564), remplacer le `None` (argument `ppf` posé en Task 2 après `directory.as_ref()`) par `ppf.as_ref()` :

```rust
            &resolutions,
            directory.as_ref(),
            ppf.as_ref(),
            &out,
            stamp.as_deref(),
```

- [ ] **Step 3: Vérifier compilation + suite complète**

Run: `cd client/src-tauri && cargo test`
Expected: PASS (aucune régression).

- [ ] **Step 4: Commit**

```bash
git add client/src-tauri/src/commands.rs
git commit -m "feat(superpopaul): generate_output — gate PPF (colonne demandée + annuaire non vide)"
```

---

## Task 4 : frontend `columns.js`

Aucun test JS dans le projet (UI sans logique métier, CLAUDE.md) : vérifié en Task 6.

**Files:**
- Modify: `client/src/columns.js`

- [ ] **Step 1: Étendre `PEPPOL_FIELDS` et `PEPPOL_SAMPLE`**

Dans `PEPPOL_FIELDS` (l. 7-13), ajouter après `["in_directory", "annuaire Peppol"],` :

```js
  ["annuaire_ppf", "annuaire PPF"], ["ppf_active", "PPF actif"],
  ["pdp_definie", "PDP définie"], ["ppf_usable", "PPF utilisable"],
```

Dans `PEPPOL_SAMPLE` (l. 14-17), ajouter les valeurs d'exemple (avant l'accolade fermante) :

```js
                        annuaire_ppf: "true", ppf_active: "true",
                        pdp_definie: "false", ppf_usable: "false" };
```

(intégrer proprement dans l'objet existant — retirer/rajouter le `}` final).

- [ ] **Step 2: Déclarer les tables de correspondance PPF**

Juste après la constante `PEPPOL_SAMPLE`, ajouter :

```js
// Famille PPF : icône commune 🏛️, accent violet en dégradé (une classe par
// champ, l'entonnoir présence → utilisable ; cf. spec 2026-07-19-champs-ppf).
const PPF_CLASS = {
  annuaire_ppf: "ppf-annuaire", ppf_active: "ppf-active",
  pdp_definie: "ppf-pdp", ppf_usable: "ppf-usable",
};
const PPF_TIP = {
  annuaire_ppf: "Adressage présent dans l'annuaire PPF chargé (au moins une ligne).",
  ppf_active: "Annuaire PPF : au moins une ligne au motif C ou P.",
  pdp_definie: "Annuaire PPF : au moins une ligne avec une PDP réelle (pdp_fictive = 0).",
  ppf_usable: "Annuaire PPF : au moins une même ligne au motif C ou P ET PDP réelle (pdp_fictive = 0).",
};
```

- [ ] **Step 3: Icône 🏛️ dans `colLabel`**

Remplacer la ligne `const icon = c.field === "in_directory" ? "📇" : "⚡";` (l. 44) par :

```js
  const icon = c.field === "in_directory" ? "📇"
             : PPF_CLASS[c.field] ? "🏛️" : "⚡";
```

- [ ] **Step 4: Classes d'accent dans `colClass`**

Remplacer le `return` de `colClass` (l. 52) `return c.field === "in_directory" ? "dir" : "peppol";` par :

```js
  if (c.field === "in_directory") return "dir";
  return PPF_CLASS[c.field] ?? "peppol";
```

- [ ] **Step 5: Tooltips par champ dans `makeHeader`**

Remplacer le bloc `if (c.source === "peppol") attrs.title = ...` (l. 59-61) par :

```js
  if (c.source === "peppol")
    attrs.title = c.field === "in_directory"
      ? "Présence déclarative dans l'annuaire Peppol chargé — indépendant du provisionning Peppol"
      : PPF_TIP[c.field]
      ?? "Champ calculé par l'API Peppol — les valeurs affichées sont un exemple.";
```

- [ ] **Step 6: Commit**

```bash
git add client/src/columns.js
git commit -m "feat(superpopaul): étape 2 — 4 champs PPF (icône 🏛️, accents, tooltips)"
```

---

## Task 5 : styles `client/src/styles.css`

**Files:**
- Modify: `client/src/styles.css`

- [ ] **Step 1: Variables du dégradé violet**

Dans le bloc `:root` (près de `--pid` l. 12), ajouter :

```css
  /* famille PPF (onglet Format) : violet en dégradé, entonnoir large → strict */
  --ppf-l1: #6f6aa8; --ppf-l2: #8a80d4; --ppf-l3: #a892ff; --ppf-l4: #c3b6ff;
```

- [ ] **Step 2: Accents en-tête**

Après `#out-preview th.dir { ... }` (l. 132), ajouter :

```css
#out-preview th.ppf-annuaire { color: var(--ppf-l1); box-shadow: inset 0 0 0 1px var(--ppf-l1); }
#out-preview th.ppf-active   { color: var(--ppf-l2); box-shadow: inset 0 0 0 1px var(--ppf-l2); }
#out-preview th.ppf-pdp      { color: var(--ppf-l3); box-shadow: inset 0 0 0 1px var(--ppf-l3); }
#out-preview th.ppf-usable   { color: var(--ppf-l4); box-shadow: inset 0 0 0 1px var(--ppf-l4); }
```

- [ ] **Step 3: Accents chip (drop zone)**

Après `.chip.dir { color: var(--green); border-color: var(--green); }` (l. 156), ajouter :

```css
.chip.ppf-annuaire { color: var(--ppf-l1); border-color: var(--ppf-l1); }
.chip.ppf-active   { color: var(--ppf-l2); border-color: var(--ppf-l2); }
.chip.ppf-pdp      { color: var(--ppf-l3); border-color: var(--ppf-l3); }
.chip.ppf-usable   { color: var(--ppf-l4); border-color: var(--ppf-l4); }
```

- [ ] **Step 4: Commit**

```bash
git add client/src/styles.css
git commit -m "feat(superpopaul): styles — dégradé violet des 4 champs PPF (onglet Format)"
```

---

## Task 6 : vérification finale (tests + clippy + GUI)

**Files:** aucun (vérification).

- [ ] **Step 1: Suite Rust complète + clippy**

Run: `cd client/src-tauri && cargo test && cargo clippy -- -D warnings`
Expected: tous verts, zéro warning clippy.

- [ ] **Step 2: Vérification GUI (parité maquette)**

Lancer l'app (`cd client && npm run tauri dev` ou l'équivalent du projet). Puis :
1. Onglet Fichiers → déposer un export PPF de test (ou réutiliser un annuaire déjà chargé).
2. Onglet Format → vérifier que les chips **🏛️ annuaire PPF / PPF actif / PDP définie / PPF utilisable** apparaissent dans la drop zone, en dégradé violet (l1→l4).
3. Glisser les 4 en sortie ; survoler chaque en-tête → tooltip correct.
4. Générer un CSV sur un fichier d'entrée dont on connaît quelques adressages ; ouvrir le CSV et vérifier `annuaire_ppf`/`ppf_active`/`pdp_definie`/`ppf_usable` (dont un cas `usable=false` avec `active=true`+`pdp_definie=true` sur lignes séparées, et un non-0225 → vide).
5. Reset annuaire PPF puis re-générer avec les 4 colonnes → cellules vides (gate table vide).

Expected: rendu conforme à `scratchpad/maquette-ppf-champs.html` (Option C, icône 🏛️) ; valeurs CSV conformes à la table de vérité de la spec.

- [ ] **Step 3: Attendre le go GUI explicite**

Ne pas clore tant que le rendu et les valeurs ne sont pas validés de visu (règle projet « maquette avant code » / validation GUI).

---

## Self-Review (fait à la rédaction)

- **Couverture spec** : enum (T2) ✓ ; `store::ppf_flags` (T1) ✓ ; `output` param+calcul (T2) ✓ ; gate `commands` (T3) ✓ ; `columns.js` libellés/icône/classes/tooltips (T4) ✓ ; `styles.css` dégradé (T5) ✓ ; sémantique vide/false/true + cas séparateur `usable` (tests T1/T2) ✓ ; pas de CLI/serveur/report ✓.
- **Placeholders** : aucun — code complet à chaque étape.
- **Cohérence des types** : `PpfFlags { in_ppf, active, pdp_definie, usable }` identique en T1/T2/T3 ; `ppf_flags(&[String]) -> HashMap<String, PpfFlags>` ; param `ppf: Option<&HashMap<String, PpfFlags>>` cohérent `generate`/`write_output`/appel `commands` ; classes CSS `ppf-annuaire/active/pdp/usable` identiques `columns.js` ↔ `styles.css` ; variantes `AnnuairePpf/PpfActive/PdpDefinie/PpfUsable` ↔ en-têtes `annuaire_ppf/ppf_active/pdp_definie/ppf_usable` cohérentes enum ↔ `field_name` ↔ `PEPPOL_FIELDS`.
