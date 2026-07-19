# Sécurisation de la montée en charge — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ajouter au rapport HTML une section « Sécurisation de la montée en charge » (entonnoir d'attrition + synthèse, en lignes/`record_label`) et annoter l'unité dans deux titres existants.

**Architecture :** Fonction pure `securisation::compute` (par ligne, pondérée), alimentée à l'export par `securisation_from_scan` (jointure `load_map` × `ppf_flags` × `directory_present`, gate = 2 annuaires chargés, `ctc_ready` via `output::ctc_status` réutilisé). Rendu par `report::securisation_section`. Aucun calcul existant modifié.

**Tech Stack :** Rust (Tauri, rusqlite, serde, chrono).

Tests Rust depuis `client/src-tauri/` : `cargo test --color=never` (JAMAIS `--nocolor`). Clippy : le repo a **5 lints PRÉEXISTANTS** (`direct.rs`, `directory.rs:47`, `resolver.rs:142`, `commands.rs:88`) — les ignorer ; ne vérifier que l'absence de NOUVEAU lint sur les fichiers touchés.

---

## File Structure

- **Create** `client/src-tauri/src/securisation.rs` — `LineFlags`, `Securisation`, `compute`, tests. Cœur pur.
- **Modify** `client/src-tauri/src/lib.rs` — `pub mod securisation;`.
- **Modify** `client/src-tauri/src/output.rs` — `ctc_status` passe `pub(crate)` (réutilisé pour la parité).
- **Modify** `client/src-tauri/src/report.rs` — champ `ReportData.securisation`, CSS `--sec-*`, `securisation_section`, titres d'unité, tests.
- **Modify** `client/src-tauri/src/commands.rs` — `securisation_from_scan`, intégration `export_report`.

`ReportData.securisation` (report.rs) et son site d'appel `export_report` (commands.rs) sont **mutuellement dépendants** → committés **ensemble** (Task 4), comme la leçon du chantier couverture.

---

## Task 1 : Module pur `securisation.rs`

**Files:** Create `client/src-tauri/src/securisation.rs` ; Modify `client/src-tauri/src/lib.rs`.

- [ ] **Step 1 : Écrire le module + tests**

Créer `client/src-tauri/src/securisation.rs` :

```rust
//! Sécurisation de la montée en charge : par ligne du fichier principal, croise
//! l'état de résolution (réseau Peppol + extension FR prête), le drapeau PPF
//! `usable` et la présence en annuaire Peppol. Agrégat PUR (aucune DB, aucune
//! UI) ; la jointure vit dans `commands::securisation_from_scan`. Chaque niveau
//! de l'entonnoir est un sous-ensemble strict du précédent.

use serde::{Deserialize, Serialize};

/// Drapeaux d'une ligne d'entrée (poids = nombre de lignes du PID canonique).
pub struct LineFlags {
    pub weight: usize,
    pub in_peppol: bool,    // exists_in_peppol == Some(true)
    pub ctc_ready: bool,    // extension FR prête aujourd'hui (output::ctc_status == "ready")
    pub ppf_usable: bool,   // drapeau PPF usable (0225)
    pub in_directory: bool, // présent annuaire Peppol (0225)
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Securisation {
    pub total_lines: usize,
    pub provisionnes: usize,    // in_peppol
    pub avec_extension: usize,  // in_peppol ∧ ctc_ready
    pub coeur: usize,           // in_peppol ∧ ctc_ready ∧ ppf_usable
    pub pleinement: usize,      // coeur ∧ in_directory
    pub ppf_usable_seul: usize, // composante autonome
    pub ctc_ready_seul: usize,  // composante autonome
}

pub fn compute(lines: &[LineFlags]) -> Securisation {
    let mut s = Securisation {
        total_lines: 0,
        provisionnes: 0,
        avec_extension: 0,
        coeur: 0,
        pleinement: 0,
        ppf_usable_seul: 0,
        ctc_ready_seul: 0,
    };
    for f in lines {
        s.total_lines += f.weight;
        if f.in_peppol {
            s.provisionnes += f.weight;
        }
        if f.in_peppol && f.ctc_ready {
            s.avec_extension += f.weight;
        }
        if f.in_peppol && f.ctc_ready && f.ppf_usable {
            s.coeur += f.weight;
        }
        if f.in_peppol && f.ctc_ready && f.ppf_usable && f.in_directory {
            s.pleinement += f.weight;
        }
        if f.ppf_usable {
            s.ppf_usable_seul += f.weight;
        }
        if f.ctc_ready {
            s.ctc_ready_seul += f.weight;
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(weight: usize, in_peppol: bool, ctc_ready: bool, ppf_usable: bool, in_directory: bool) -> LineFlags {
        LineFlags { weight, in_peppol, ctc_ready, ppf_usable, in_directory }
    }

    // Emboîtement : tout-vrai compte à tous les niveaux.
    #[test]
    fn emboitement_tout_vrai() {
        let s = compute(&[line(1, true, true, true, true)]);
        assert_eq!((s.provisionnes, s.avec_extension, s.coeur, s.pleinement), (1, 1, 1, 1));
    }

    // Sans in_directory : cœur oui, pleinement non.
    #[test]
    fn sans_annuaire_pas_pleinement() {
        let s = compute(&[line(1, true, true, true, false)]);
        assert_eq!(s.coeur, 1);
        assert_eq!(s.pleinement, 0);
    }

    // Sans ppf_usable : avec_extension oui, cœur non.
    #[test]
    fn sans_ppf_usable_pas_coeur() {
        let s = compute(&[line(1, true, true, false, true)]);
        assert_eq!(s.avec_extension, 1);
        assert_eq!(s.coeur, 0);
    }

    // Pondération par le poids.
    #[test]
    fn ponderation() {
        let s = compute(&[line(4, true, true, true, true)]);
        assert_eq!(s.total_lines, 4);
        assert_eq!(s.pleinement, 4);
    }

    // Composantes autonomes : ppf_usable/ctc_ready comptent sans in_peppol.
    #[test]
    fn composantes_autonomes() {
        let s = compute(&[line(1, false, true, true, true)]); // pas provisionné
        assert_eq!(s.provisionnes, 0);
        assert_eq!(s.avec_extension, 0);
        assert_eq!(s.coeur, 0);
        assert_eq!(s.ppf_usable_seul, 1);
        assert_eq!(s.ctc_ready_seul, 1);
    }

    // Non-0225 (ppf_usable=false, in_directory=false) mais provisionné+extension :
    // entre dans l'entonnoir haut, jamais dans cœur/pleinement.
    #[test]
    fn non_0225_reste_en_haut_de_l_entonnoir() {
        let s = compute(&[line(3, true, true, false, false)]);
        assert_eq!(s.provisionnes, 3);
        assert_eq!(s.avec_extension, 3);
        assert_eq!(s.coeur, 0);
        assert_eq!(s.pleinement, 0);
    }

    #[test]
    fn total_vide() {
        let s = compute(&[]);
        assert_eq!(s.total_lines, 0);
        assert_eq!(s.provisionnes, 0);
    }
}
```

- [ ] **Step 2 : Déclarer le module**

Dans `client/src-tauri/src/lib.rs`, ajouter `pub mod securisation;` en respectant l'ordre alphabétique (après `pub mod resolver;` / avant `pub mod store;` — insérer là où l'ordre est cohérent avec les `pub mod` existants).

- [ ] **Step 3 : Tests + clippy**

Run : `cargo test securisation::tests --color=never` → 7 tests verts.
Run : `cargo clippy --all-targets --color=never -- -D warnings 2>&1 | grep -i securisation.rs` → vide (clippy-clean sur le module).

- [ ] **Step 4 : Commit**

```bash
git add client/src-tauri/src/securisation.rs client/src-tauri/src/lib.rs
git commit -m "feat(superpopaul): module securisation — entonnoir de montée en charge (pur, TDD)

Claude-Session: https://claude.ai/code/session_01TDtNYu8g39HHSUxDrYSGPs"
```

---

## Task 2 : `output::ctc_status` réutilisable

**Files:** Modify `client/src-tauri/src/output.rs`.

- [ ] **Step 1 : Passer la visibilité à `pub(crate)`**

Dans `client/src-tauri/src/output.rs`, la fonction `fn ctc_status(` (vers ligne 15) devient `pub(crate) fn ctc_status(`. Aucun autre changement. Garder le commentaire de doc au-dessus.

- [ ] **Step 2 : Compile + clippy**

Run : `cargo build` → OK.
Run : `cargo clippy --all-targets --color=never -- -D warnings 2>&1 | grep -i "output.rs"` → aucun NOUVEAU lint (le module en a peut-être des préexistants ; vérifier qu'aucun ne concerne `ctc_status`).

- [ ] **Step 3 : Commit**

```bash
git add client/src-tauri/src/output.rs
git commit -m "refactor(superpopaul): output::ctc_status en pub(crate) (réutilisé par la sécurisation)

Claude-Session: https://claude.ai/code/session_01TDtNYu8g39HHSUxDrYSGPs"
```

---

## Task 3 : Rapport — section + titres d'unité + tests

**Files:** Modify `client/src-tauri/src/report.rs`.

> Cette task ajoute le champ `securisation` à `ReportData` et casse donc la compilation de `export_report` (commands.rs) jusqu'à Task 4. **Ne pas committer Task 3 seule** : Task 3 et Task 4 forment **un seul commit** (Step de commit en Task 4). Implémenter Task 3 puis Task 4, vérifier, committer une fois.

- [ ] **Step 1 : Écrire les tests d'abord**

Dans le module `#[cfg(test)] mod tests` de `report.rs`, après `data_with` (et l'usage existant de `use crate::coverage::...`), ajouter :

```rust
    fn secu_full() -> crate::securisation::Securisation {
        crate::securisation::Securisation {
            total_lines: 126_316,
            provisionnes: 98_000,
            avec_extension: 61_000,
            coeur: 12_340,
            pleinement: 9_210,
            ppf_usable_seul: 18_609,
            ctc_ready_seul: 61_000,
        }
    }

    #[test]
    fn securisation_section_rendue() {
        let s = snap();
        let secu = secu_full();
        let d = ReportData { securisation: Some(&secu), ..data(&s) };
        let html = render(&d);
        assert!(html.contains("Sécurisation de la montée en charge"), "titre");
        assert!(html.contains("Entonnoir d'attrition"), "sous-titre entonnoir");
        assert!(html.contains("Synthèse"), "sous-titre synthèse");
        assert!(html.contains("Cœur sécurisé"), "libellé cœur");
        assert!(html.contains("Pleinement sécurisés"), "libellé pleinement");
        assert!(html.contains(&fmt_int(12_340)), "compte cœur");
        assert!(html.contains(&fmt_int(9_210)), "compte pleinement");
        assert!(html.contains(&fmt_int(18_609)), "composante PPF utilisable");
    }

    #[test]
    fn securisation_absente_si_none() {
        let html = render(&data(&snap())); // securisation = None via data()
        assert!(!html.contains("Sécurisation de la montée en charge"));
    }

    #[test]
    fn titre_pa_annote_en_adressages() {
        let html = render(&data(&snap())); // snap() a des PA
        assert!(html.contains("Plateformes de dématérialisation constatées"));
        assert!(html.contains("en adressages uniques"), "unité PA absente du titre");
    }

    #[test]
    fn titre_couverture_annote_avec_record_label() {
        let s = snap();
        let cov = cov_full(); // couverture présente → section rendue
        let d = ReportData { coverage: &cov, ..data(&s) };
        let html = render(&d);
        assert!(html.contains("Présence déclarative en annuaire"));
        // record_plural de data() = "lignes" (label par défaut ⇒ « en lignes » est correct ici)
        assert!(html.contains("en lignes"), "unité couverture absente du titre");
    }

    #[test]
    fn securisation_respecte_record_label() {
        let s = snap();
        let secu = secu_full();
        let cov = cov_full();
        let mut d = ReportData { securisation: Some(&secu), coverage: &cov, ..data(&s) };
        d.record_plural = "abonnés";
        let html = render(&d);
        assert!(!html.contains("lignes"), "aucun « lignes » codé en dur avec un autre label");
        assert!(html.contains("abonnés"), "le libellé record doit apparaître");
    }
```

- [ ] **Step 2 : Ajouter le champ `securisation` à `ReportData` + au `data()` de test**

Dans la struct `ReportData<'a>`, après `coverage`, ajouter :

```rust
    /// Sécurisation de la montée en charge (jointure résolutions × annuaires).
    /// `None` = les 2 annuaires ne sont pas chargés → section non rendue.
    pub securisation: Option<&'a crate::securisation::Securisation>,
```

Et dans le `data()` de test, ajouter le champ :

```rust
            securisation: None,
```

- [ ] **Step 3 : CSS — variables `--sec-*` + classes de section**

Dans la constante `CSS` :

Bloc `:root` (après les `--ppf-l*`) — ajouter :

```css
    --sec-1: #3f7d54; --sec-2: #46a862; --sec-3: #5bd07d; --sec-4: #8be0a3;
```

Bloc `@media (prefers-color-scheme: light)` **et** bloc `@media print` (dans chaque `:root {...}`) — ajouter (verts plus soutenus, lisibles sur blanc, strict = plus foncé) :

```css
 --sec-1: #6fb589; --sec-2: #4fa06d; --sec-3: #2f8b50; --sec-4: #1a7340;
```

Avant la fermeture `"#;` de `CSS`, ajouter les classes (réutilise `.cov-name`/`.cov-sw`/`.cov-n`/`.bar`/`.kpi`/`.kpis` existants) :

```css
  .unit { color: var(--muted); font-weight: 400; font-size: 12.5px; }
  .h2sub { color: var(--muted); font-size: 12.5px; margin: 8px 0 14px; }
  .sec-subh { font-size: 11px; font-weight: 700; letter-spacing: .1em; text-transform: uppercase;
    color: var(--muted); margin: 18px 0 8px; }
  .sec-row { display: grid; grid-template-columns: 260px 1fr 110px; gap: 14px; align-items: center;
    padding: 6px 0; font-size: 14px; }
  .sec-row .tag { color: var(--muted); font-size: 11.5px; }
  .kpi.sec .v { color: var(--sec-3); }
  .kpi.sec.strong .v { color: var(--sec-4); }
  .kpi .abs { color: var(--muted); font-size: 12.5px; margin-top: 1px; }
  .kpi .abs b { color: var(--fg); }
  .compo { color: var(--muted); font-size: 12.5px; margin: 12px 0 0; padding-top: 10px;
    border-top: 1px solid var(--border); }
  .compo b { color: var(--fg); }
```

- [ ] **Step 4 : Annoter les deux titres existants**

Dans `render`, la ligne du titre PA :

```rust
    html.push_str("<h2>Plateformes de dématérialisation constatées</h2>\n<div class=\"pa\">\n");
```
devient :
```rust
    html.push_str("<h2>Plateformes de dématérialisation constatées <span class=\"unit\">· en adressages uniques</span></h2>\n<div class=\"pa\">\n");
```

Dans `coverage_section`, la ligne du titre couverture :

```rust
    html.push_str("<h2>Présence déclarative en annuaire</h2>\n<div class=\"pa\">\n");
```
devient :
```rust
    html.push_str(&format!(
        "<h2>Présence déclarative en annuaire <span class=\"unit\">· en {record_plural}</span></h2>\n<div class=\"pa\">\n"
    ));
```

- [ ] **Step 5 : Ajouter `securisation_section` et l'appeler dans `render`**

Ajouter cette fonction juste après `coverage_section` :

```rust
// Section « Sécurisation de la montée en charge » : entonnoir d'attrition
// (niveaux emboîtés) + synthèse (2 chiffres + composantes). En lignes du
// fichier, libellées `record_plural` — jamais « ligne » en dur.
fn securisation_section(html: &mut String, s: &crate::securisation::Securisation, record_plural: &str) {
    let denom = s.total_lines;
    let pct = |n: usize| -> String {
        match (n * 100 + denom / 2).checked_div(denom) {
            Some(v) => format!("{v}\u{202F}%"),
            None => "—".to_string(),
        }
    };
    let width = |n: usize| -> f64 {
        if denom == 0 { 0.0 } else { n as f64 * 100.0 / denom as f64 }
    };
    let row = |html: &mut String, color: &str, name: &str, tag: &str, n: usize, bold: bool| {
        let nm = if bold { format!("<b>{name}</b>") } else { name.to_string() };
        let tg = if tag.is_empty() { String::new() } else { format!(" <span class=\"tag\">{tag}</span>") };
        html.push_str(&format!(
            "<div class=\"sec-row\"><span class=\"cov-name\">\
             <span class=\"cov-sw\" style=\"background:var(--{color})\"></span>{nm}{tg}</span>\
             <span class=\"bar\"><i style=\"width:{:.0}%;background:var(--{color})\"></i></span>\
             <span class=\"cov-n\"><b>{}</b> · {}</span></div>\n",
            width(n), fmt_int(n as u64), pct(n)
        ));
    };

    html.push_str(&format!(
        "<h2>Sécurisation de la montée en charge <span class=\"unit\">· en {record_plural}</span></h2>\n"
    ));
    html.push_str(&format!(
        "<p class=\"h2sub\">Sur <b>{}</b> {record_plural} — la part prête sur tous les axes de la \
         facturation électronique française.</p>\n<div class=\"pa\">\n",
        fmt_int(denom as u64)
    ));

    html.push_str("<div class=\"sec-subh\">Entonnoir d'attrition</div>\n");
    row(html, "sec-1", "Provisionnés réseau Peppol", "", s.provisionnes, false);
    row(html, "sec-2", "+ extension FR prête", "", s.avec_extension, false);
    row(html, "sec-3", "+ PPF utilisable", "= cœur sécurisé", s.coeur, true);
    row(html, "sec-4", "+ annuaire Peppol", "= pleinement sécurisés", s.pleinement, true);

    html.push_str("<div class=\"sec-subh\">Synthèse</div>\n<div class=\"kpis\">\n");
    html.push_str(&format!(
        "<div class=\"kpi sec\"><div class=\"v\">{}</div><div class=\"l\">Cœur sécurisé</div>\
         <div class=\"abs\"><b>{}</b> {record_plural}</div>\
         <div class=\"d\">PPF utilisable + provisionné Peppol + extension FR prête</div></div>\n",
        pct(s.coeur), fmt_int(s.coeur as u64)
    ));
    html.push_str(&format!(
        "<div class=\"kpi sec strong\"><div class=\"v\">{}</div><div class=\"l\">Pleinement sécurisés</div>\
         <div class=\"abs\"><b>{}</b> {record_plural}</div>\
         <div class=\"d\">… et aussi présents dans l'annuaire Peppol</div></div>\n",
        pct(s.pleinement), fmt_int(s.pleinement as u64)
    ));
    html.push_str("</div>\n");

    html.push_str(&format!(
        "<p class=\"compo\">Composantes, chacune prise seule : <b>{}</b> PPF utilisable · \
         <b>{}</b> provisionnés Peppol · <b>{}</b> extension FR prête.</p>\n</div>\n",
        fmt_int(s.ppf_usable_seul as u64),
        fmt_int(s.provisionnes as u64),
        fmt_int(s.ctc_ready_seul as u64)
    ));
}
```

Puis, dans `render`, appeler la section **après** `coverage_section` (juste avant le pied de page) :

```rust
    if let Some(secu) = d.securisation {
        securisation_section(&mut html, secu, d.record_plural);
    }
```

- [ ] **Step 6 : Compilera avec Task 4** — passer à Task 4, puis vérifier et committer une fois.

---

## Task 4 : Jointure à l'export + intégration (commit atomique avec Task 3)

**Files:** Modify `client/src-tauri/src/commands.rs`.

- [ ] **Step 1 : Ajouter le helper `securisation_from_scan`**

Dans `commands.rs`, juste après `coverage_from_scan`, ajouter :

```rust
/// Sécurisation de la montée en charge à partir d'un scan déjà fait. Gate : les
/// DEUX annuaires doivent être chargés (sinon cœur/pleinement seraient des zéros
/// trompeurs) → `Ok(None)`. Population : lignes du fichier courant, dernier état
/// de résolution connu en base (`load_map`). `ctc_ready` réutilise
/// `output::ctc_status` (parité colonne CSV).
fn securisation_from_scan(
    store: &Store,
    pids: &[String],
    line_counts: &HashMap<String, u64>,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<Option<crate::securisation::Securisation>, String> {
    if store.peppol_directory_status()?.is_none() || store.ppf_summary()?.distinct_addr == 0 {
        return Ok(None);
    }
    let resolutions = store.load_map(pids)?;
    let values: Vec<String> = pids
        .iter()
        .filter_map(|p| crate::directory::parse_0225_value(p))
        .collect();
    let present = store.directory_present(&values)?;
    let ppf = store.ppf_flags(&values)?;

    let mut lines: Vec<crate::securisation::LineFlags> = Vec::with_capacity(pids.len());
    for p in pids {
        let weight = *line_counts.get(p).unwrap_or(&0) as usize;
        let r = resolutions.get(p);
        let in_peppol = r.map(|r| r.exists_in_peppol == Some(true)).unwrap_or(false);
        let ctc_ready = r.map(|r| crate::output::ctc_status(r, now) == "ready").unwrap_or(false);
        let (ppf_usable, in_directory) = match crate::directory::parse_0225_value(p) {
            Some(v) => (ppf.get(&v).map(|f| f.usable).unwrap_or(false), present.contains(&v)),
            None => (false, false),
        };
        lines.push(crate::securisation::LineFlags {
            weight,
            in_peppol,
            ctc_ready,
            ppf_usable,
            in_directory,
        });
    }
    Ok(Some(crate::securisation::compute(&lines)))
}
```

- [ ] **Step 2 : Intégrer dans `export_report` (scan unique → couverture + sécurisation)**

Dans le corps `spawn_blocking` d'`export_report`, remplacer le bloc actuel de calcul de la couverture (le `match scan_unique_pids { ... }` qui produit `let coverage = ...`) par :

```rust
        // Agrégats annuaire/sécurisation sur l'entrée COURANTE, un seul scan.
        // Tolérant : entrée illisible → rapport sans ces sections.
        let (coverage, securisation) = match scan_unique_pids(&input, &cfg.input.pid_column) {
            Ok((_, pids, line_counts)) => {
                let now_utc = chrono::Utc::now();
                let store_g = store.lock().unwrap();
                let cov = coverage_from_scan(&store_g, &pids, &line_counts)
                    .unwrap_or(crate::coverage::Coverage::EMPTY);
                let secu = securisation_from_scan(&store_g, &pids, &line_counts, now_utc)
                    .ok()
                    .flatten();
                (cov, secu)
            }
            Err(_) => (crate::coverage::Coverage::EMPTY, None),
        };
```

Puis, dans la construction de `report::ReportData { ... }`, ajouter le champ après `coverage: &coverage,` :

```rust
            securisation: securisation.as_ref(),
```

(Vérifier qu'il ne reste qu'UN seul `scan_unique_pids` dans `export_report`.)

- [ ] **Step 3 : Compile + suite complète + clippy**

Run (depuis `client/src-tauri/`) :
- `cargo build` → OK.
- `cargo test --color=never` → suite complète verte (report + securisation + existants). Noter le total (base 245 + 7 securisation + ~5 report = ~257).
- `cargo clippy --all-targets --color=never -- -D warnings 2>&1 | grep -iE "report.rs|commands.rs|securisation.rs|output.rs"` → aucun NOUVEAU lint de notre fait (rappel `commands.rs:88` préexistant).

- [ ] **Step 4 : Commit atomique Task 3 + Task 4**

```bash
git add client/src-tauri/src/report.rs client/src-tauri/src/commands.rs
git commit -m "feat(superpopaul): rapport — section « sécurisation de la montée en charge » + titres d'unité

Entonnoir d'attrition (provisionné → +extension FR → +PPF utilisable → +annuaire)
+ synthèse (2 chiffres + composantes), en lignes/record_label. Jointure post-run
résolutions × PPF × annuaire Peppol à l'export (gate 2 annuaires, tolérant),
ctc_ready via output::ctc_status. Titres PA « en adressages uniques » et
couverture « en {record_label} ». report.rs + commands.rs committés ensemble
(ReportData.securisation et son site d'appel export_report sont mutuellement
dépendants — commit atomique pour ne pas casser le build).

Claude-Session: https://claude.ai/code/session_01TDtNYu8g39HHSUxDrYSGPs"
```

---

## Task 5 : Vérification finale + échantillon réel

**Files:** aucun (vérification).

- [ ] **Step 1 : Suite complète + clippy**

Run : `cargo test --color=never` → tout vert, 0 échec.
Run : `cargo clippy --all-targets --color=never -- -D warnings 2>&1 | grep -iE "securisation|report.rs|commands.rs|output.rs"` → aucun NOUVEAU lint.

- [ ] **Step 2 : Rendre un échantillon réel (test jetable) et l'inspecter**

Ajouter temporairement, dans `report.rs` `mod tests` (après `secu_full`), un test qui écrit le HTML à la magnitude réelle (labels « CFs »), pour validation visuelle :

```rust
    #[test]
    fn dump_sample_securisation() {
        let s = snap();
        let secu = secu_full();
        let cov = cov_full();
        let d = ReportData { securisation: Some(&secu), coverage: &cov, record_plural: "CFs", ..data(&s) };
        std::fs::write(
            "/private/tmp/claude-501/-Users-jean-paulgavini-Documents-Dev-superpopaul/0072deba-a2f5-4f96-9435-cc28c1e0727b/scratchpad/rapport-securisation.html",
            render(&d),
        )
        .unwrap();
    }
```

Run : `cargo test dump_sample_securisation --color=never`. Envoyer le fichier au demandeur pour validation (entonnoir + synthèse, thème clair/sombre). **Puis retirer ce test** (il ne doit pas être committé) et re-vérifier `cargo test --color=never` (retour au compte sans le test jetable).

- [ ] **Step 3 : Validation GUI** — non applicable (rapport HTML pur, validé par l'échantillon ci-dessus ; pas de cockpit dans ce chantier).

---

## Self-Review (rempli à l'écriture)

**Couverture spec :**
- Titres d'unité PA + couverture → Task 3 Step 4 + tests `titre_pa_*`/`titre_couverture_*`. ✅
- Section Sécurisation (entonnoir + synthèse, record_label) → Task 3 Step 5 + tests. ✅
- Calcul par ligne, emboîtement, composantes, non-0225 → Task 1 (compute + 7 tests). ✅
- Prédicats exacts (`in_peppol`, `ctc_ready` via `output::ctc_status`, `ppf_usable`, `in_directory`) → Task 2 (pub(crate)) + Task 4 (`securisation_from_scan`). ✅
- Gate 2 annuaires + tolérance export → Task 4 (`securisation_from_scan` + `export_report`). ✅
- Jamais « ligne » en dur → test `securisation_respecte_record_label`. ✅
- Aucun calcul existant modifié → seuls ajouts (titres = annotation, pas de recompute). ✅

**Placeholders :** aucun.

**Cohérence des types :** `Securisation`/`LineFlags` (champs `provisionnes`/`avec_extension`/`coeur`/`pleinement`/`ppf_usable_seul`/`ctc_ready_seul`) identiques Task 1 → 3 → 4. `ReportData.securisation: Option<&Securisation>` cohérent report.rs ↔ commands.rs. `output::ctc_status(r, now) -> &'static str` comparé à `"ready"`.
