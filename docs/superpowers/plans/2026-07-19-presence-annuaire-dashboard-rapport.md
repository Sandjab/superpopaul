# Présence en annuaire (cockpit + rapport) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Afficher la couverture des identifiants d'entrée par les annuaires Peppol (`in_directory`) et PPF (4 drapeaux) dans le cockpit (avant run) et le rapport HTML.

**Architecture :** Une fonction pure `coverage::compute` (testable sans DB ni UI) agrège, **par ligne d'entrée** et **par éligibilité 0225**, les présences déclaratives. Elle est alimentée par un helper `coverage_from_scan` (gates + requêtes store). Côté cockpit, la couverture est **repliée dans `analyze_input`** (`InputStats.coverage`) — pas de commande dédiée, pas de second scan CSV. Côté rapport, elle est **calculée à l'export** (`export_report` passe async). Le panneau UI est un **frère juste au-dessus de `#cockpit`** (visible avant le run).

**Tech Stack :** Rust (Tauri 2, rusqlite, serde), JS vanilla + `h()`, CSS tokens « Bleu nuit & or ».

---

## Écarts assumés vs spec (à confirmer avant exécution)

Trois raffinements découverts en planifiant, qui satisfont **mieux** les contraintes validées :

1. **Cockpit : couverture repliée dans `analyze_input`** (champ `InputStats.coverage`) au lieu d'une commande `directory_coverage` séparée. Raison : `analyze_input` scanne déjà le CSV (500k lignes possibles) à l'entrée de l'étape 3 ; une commande séparée doublerait ce scan. `coverage::compute` reste testable indépendamment.
2. **Rapport : couverture calculée à l'export** (`export_report` devient `async` + `spawn_blocking`) au lieu d'être figée dans `LastRun`. Raison : `clear_run` est synchrone et léger ; y faire un scan CSV bloquerait le thread. Population = l'entrée courante, exactement ce que le cockpit affiche. (La spec anticipait déjà ce léger décalage possible entrée-courante vs entrée-du-run.)
3. **Placement UI : frère juste au-dessus de `#cockpit`** (pas *dedans*). Raison : `#cockpit` est masqué avant le run (révélé par `startRun`) ; or le panneau doit être visible **avant** le run (décision validée « visible dès le chargement »). Il apparaît donc au-dessus du bandeau métier, pas entre bandeau et Télémétrie.

Comportement observable identique à la spec ; seules l'implémentation et la position exacte changent.

---

## File Structure

- **Create** `client/src-tauri/src/coverage.rs` — structs `Coverage`/`PeppolCoverage`/`PpfCoverage` + fonction pure `compute` + tests unitaires. Responsabilité unique : agrégation de couverture, sans DB ni UI.
- **Modify** `client/src-tauri/src/lib.rs` — déclarer `pub mod coverage;`.
- **Modify** `client/src-tauri/src/commands.rs` — helper `coverage_from_scan`, extension d'`analyze_input` (`InputStats.coverage`), `export_report` async.
- **Modify** `client/src-tauri/src/report.rs` — champ `ReportData.coverage`, variables CSS `--ppf-l*`, `coverage_section`, tests.
- **Modify** `client/src/index.html` — panneau `#coverage` (frère avant `#cockpit`).
- **Modify** `client/src/styles.css` — styles du panneau.
- **Modify** `client/src/cockpit.js` — `renderCoverage` + appel après `analyze_input`.

Commande de test Rust (depuis `client/src-tauri/`) : `cargo test`. Un seul test cible : `cargo test <nom> -- --nocolor`.

---

## Task 1 : Module `coverage.rs` — structs + fonction pure + tests

**Files:**
- Create: `client/src-tauri/src/coverage.rs`

- [ ] **Step 1 : Écrire le module complet avec ses tests (test d'abord — le module est neuf, tests et impl vont ensemble mais les tests décrivent le contrat)**

Créer `client/src-tauri/src/coverage.rs` :

```rust
//! Couverture déclarative des identifiants d'entrée par les annuaires chargés
//! (Peppol `in_directory` + 4 drapeaux PPF). Agrégat PUR : aucune DB, aucune
//! UI — la partie impure (scan CSV, requêtes store) vit dans `commands.rs`.
//! Comptage PAR LIGNE (chaque identifiant unique pondéré par son nombre de
//! lignes) et PAR ÉLIGIBILITÉ 0225 (seuls les identifiants 0225 comptent au
//! dénominateur ; les autres sont « non applicables »).

use crate::store::PpfFlags;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PeppolCoverage {
    /// Lignes 0225 présentes dans l'annuaire Peppol chargé.
    pub present: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PpfCoverage {
    pub present: usize,     // annuaire_ppf
    pub active: usize,      // ppf_active
    pub pdp_definie: usize, // pdp_definie
    pub usable: usize,      // ppf_usable
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Coverage {
    pub total_lines: usize,
    /// Dénominateur : lignes portant un identifiant 0225.
    pub eligible_0225: usize,
    /// Lignes non-0225 (hors calcul), affichées en clair.
    pub non_applicable: usize,
    /// `None` = annuaire Peppol non chargé (bloc masqué).
    pub peppol: Option<PeppolCoverage>,
    /// `None` = annuaire PPF non chargé (bloc masqué).
    pub ppf: Option<PpfCoverage>,
}

/// Agrège la couverture.
///
/// - `eligible` : `(valeur 0225, nombre de lignes)` pour chaque identifiant
///   0225 unique de l'entrée.
/// - `non_applicable` : total des lignes non-0225.
/// - `present` : `Some(ensemble des valeurs présentes en annuaire Peppol)`, ou
///   `None` si l'annuaire Peppol n'est pas chargé (gate).
/// - `ppf` : `Some(map identifiant → drapeaux)`, ou `None` si l'annuaire PPF
///   n'est pas chargé. Une valeur absente de la map compte 0 (drapeaux défaut).
pub fn compute(
    eligible: &[(String, usize)],
    non_applicable: usize,
    present: Option<&HashSet<String>>,
    ppf: Option<&HashMap<String, PpfFlags>>,
) -> Coverage {
    let eligible_0225: usize = eligible.iter().map(|(_, n)| n).sum();

    let peppol = present.map(|set| PeppolCoverage {
        present: eligible
            .iter()
            .filter(|(v, _)| set.contains(v))
            .map(|(_, n)| n)
            .sum(),
    });

    let ppf = ppf.map(|map| {
        let mut c = PpfCoverage { present: 0, active: 0, pdp_definie: 0, usable: 0 };
        for (v, n) in eligible {
            let f = map.get(v).copied().unwrap_or_default();
            if f.in_ppf {
                c.present += n;
            }
            if f.active {
                c.active += n;
            }
            if f.pdp_definie {
                c.pdp_definie += n;
            }
            if f.usable {
                c.usable += n;
            }
        }
        c
    });

    Coverage {
        total_lines: eligible_0225 + non_applicable,
        eligible_0225,
        non_applicable,
        peppol,
        ppf,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flags(in_ppf: bool, active: bool, pdp_definie: bool, usable: bool) -> PpfFlags {
        PpfFlags { in_ppf, active, pdp_definie, usable }
    }

    // Peppol seul chargé → bloc PPF masqué, présence Peppol comptée par ligne.
    #[test]
    fn peppol_seul_charge() {
        let eligible = vec![("a".into(), 2usize), ("b".into(), 1usize)];
        let present: HashSet<String> = ["a".to_string()].into_iter().collect();
        let cov = compute(&eligible, 0, Some(&present), None);
        assert_eq!(cov.peppol, Some(PeppolCoverage { present: 2 })); // "a" sur 2 lignes
        assert_eq!(cov.ppf, None);
        assert_eq!(cov.eligible_0225, 3);
    }

    // PPF seul chargé → bloc Peppol masqué.
    #[test]
    fn ppf_seul_charge() {
        let eligible = vec![("a".into(), 1usize)];
        let mut map = HashMap::new();
        map.insert("a".to_string(), flags(true, true, true, true));
        let cov = compute(&eligible, 0, None, Some(&map));
        assert_eq!(cov.peppol, None);
        assert_eq!(cov.ppf, Some(PpfCoverage { present: 1, active: 1, pdp_definie: 1, usable: 1 }));
    }

    // Aucun annuaire → les deux blocs masqués.
    #[test]
    fn aucun_annuaire() {
        let cov = compute(&[("a".into(), 1)], 0, None, None);
        assert_eq!(cov.peppol, None);
        assert_eq!(cov.ppf, None);
    }

    // Dénominateur : les non-applicables ne comptent ni au num ni au dénom.
    #[test]
    fn denominateur_eligibles_et_non_applicables() {
        let eligible = vec![("a".into(), 3usize)];
        let present: HashSet<String> = ["a".to_string()].into_iter().collect();
        let cov = compute(&eligible, 7, Some(&present), None);
        assert_eq!(cov.eligible_0225, 3);
        assert_eq!(cov.non_applicable, 7);
        assert_eq!(cov.total_lines, 10);
        assert_eq!(cov.peppol.unwrap().present, 3);
    }

    // Comptage par ligne : même valeur sur N lignes comptée N fois.
    #[test]
    fn comptage_par_ligne() {
        let eligible = vec![("dup".into(), 4usize)];
        let present: HashSet<String> = ["dup".to_string()].into_iter().collect();
        let cov = compute(&eligible, 0, Some(&present), None);
        assert_eq!(cov.peppol.unwrap().present, 4);
    }

    // Entonnoir PPF : usable ≠ (active ∧ pdp_definie). Une valeur active + PDP
    // réelle mais sur des lignes DIFFÉRENTES compte en active et pdp_definie,
    // PAS en usable (miroir du cas store `id_split`).
    #[test]
    fn entonnoir_usable_strict() {
        let eligible = vec![("split".into(), 1usize)];
        let mut map = HashMap::new();
        map.insert("split".to_string(), flags(true, true, true, false)); // usable=false
        let cov = compute(&eligible, 0, None, Some(&map));
        let p = cov.ppf.unwrap();
        assert_eq!(p.active, 1);
        assert_eq!(p.pdp_definie, 1);
        assert_eq!(p.usable, 0, "usable exige la MÊME ligne active+PDP réelle");
    }

    // Éligible mais absent de l'annuaire → compté au dénom, 0 au num (distinct
    // de « non applicable »).
    #[test]
    fn eligible_absent_de_l_annuaire() {
        let eligible = vec![("absent".into(), 5usize)];
        let present: HashSet<String> = HashSet::new(); // annuaire chargé mais vide
        let map: HashMap<String, PpfFlags> = HashMap::new();
        let cov = compute(&eligible, 0, Some(&present), Some(&map));
        assert_eq!(cov.eligible_0225, 5);
        assert_eq!(cov.peppol.unwrap().present, 0);
        assert_eq!(cov.ppf.unwrap(), PpfCoverage { present: 0, active: 0, pdp_definie: 0, usable: 0 });
    }

    // Round-trip serde (contrat cockpit JS ↔ rapport).
    #[test]
    fn round_trip_serde() {
        let cov = Coverage {
            total_lines: 1000,
            eligible_0225: 900,
            non_applicable: 100,
            peppol: Some(PeppolCoverage { present: 812 }),
            ppf: Some(PpfCoverage { present: 640, active: 590, pdp_definie: 610, usable: 570 }),
        };
        let json = serde_json::to_string(&cov).unwrap();
        let back: Coverage = serde_json::from_str(&json).unwrap();
        assert_eq!(cov, back);
        // Contrat de noms de champs consommés côté JS.
        assert!(json.contains("\"eligible_0225\""));
        assert!(json.contains("\"non_applicable\""));
        assert!(json.contains("\"pdp_definie\""));
        assert!(json.contains("\"usable\""));
    }
}
```

- [ ] **Step 2 : Le module ne compile pas encore (pas déclaré). Le déclarer d'abord (Task 2), puis lancer les tests. Ici, juste vérifier qu'il n'y a pas de faute de frappe en le relisant.**

(Les tests tournent au Step de Task 2, une fois le module branché.)

---

## Task 2 : Déclarer le module + faire passer les tests de Task 1

**Files:**
- Modify: `client/src-tauri/src/lib.rs:4` (bloc des `pub mod`)

- [ ] **Step 1 : Ajouter la déclaration du module**

Dans `client/src-tauri/src/lib.rs`, après `pub mod config;` (ligne 3) et avant `pub mod ctc;` (ligne 4), insérer :

```rust
pub mod coverage;
```

- [ ] **Step 2 : Lancer les tests du module `coverage`**

Run (depuis `client/src-tauri/`) : `cargo test coverage::tests -- --nocolor`
Expected : PASS — 8 tests (`peppol_seul_charge`, `ppf_seul_charge`, `aucun_annuaire`, `denominateur_eligibles_et_non_applicables`, `comptage_par_ligne`, `entonnoir_usable_strict`, `eligible_absent_de_l_annuaire`, `round_trip_serde`).

- [ ] **Step 3 : Commit**

```bash
git add client/src-tauri/src/coverage.rs client/src-tauri/src/lib.rs
git commit -m "feat(superpopaul): module coverage — agrégat pur de présence annuaire (TDD)"
```

---

## Task 3 : Helper `coverage_from_scan` + extension d'`analyze_input`

**Files:**
- Modify: `client/src-tauri/src/commands.rs` (helper après `scan_unique_pids` ~ligne 93 ; struct `InputStats` ~194 ; corps `analyze_input` ~205-235)

- [ ] **Step 1 : Ajouter le helper `coverage_from_scan`**

Dans `client/src-tauri/src/commands.rs`, juste après la fonction `scan_unique_pids` (elle se termine ligne 93), insérer :

```rust
/// Couverture annuaire déclarative à partir d'un scan déjà effectué. Gate
/// INDÉPENDANT par annuaire (chargé ou non) — miroir des gates de
/// `generate_output`, mais SANS condition « colonne demandée » : le panneau de
/// couverture est indépendant de la config des colonnes de sortie. Comptage
/// par ligne : chaque PID unique est pondéré par son nombre de lignes.
fn coverage_from_scan(
    store: &Store,
    pids: &[String],
    line_counts: &HashMap<String, u64>,
) -> Result<crate::coverage::Coverage, String> {
    let mut eligible: Vec<(String, usize)> = Vec::new();
    let mut non_applicable: usize = 0;
    for p in pids {
        let n = *line_counts.get(p).unwrap_or(&0) as usize;
        match crate::directory::parse_0225_value(p) {
            Some(v) => eligible.push((v, n)),
            None => non_applicable += n,
        }
    }
    let values: Vec<String> = eligible.iter().map(|(v, _)| v.clone()).collect();
    let present = if store.peppol_directory_status()?.is_some() {
        Some(store.directory_present(&values)?)
    } else {
        None
    };
    let ppf = if store.ppf_summary()?.distinct_addr > 0 {
        Some(store.ppf_flags(&values)?)
    } else {
        None
    };
    Ok(crate::coverage::compute(
        &eligible,
        non_applicable,
        present.as_ref(),
        ppf.as_ref(),
    ))
}
```

- [ ] **Step 2 : Ajouter le champ `coverage` à `InputStats`**

Dans `client/src-tauri/src/commands.rs`, la struct `InputStats` (lignes 193-200) devient :

```rust
#[derive(Serialize)]
pub struct InputStats {
    pub unique: usize,
    pub resolved_ok: usize,
    pub failed: usize,
    pub stale: usize,
    pub missing: usize,
    pub coverage: crate::coverage::Coverage,
}
```

- [ ] **Step 3 : Calculer la couverture dans `analyze_input`**

Dans le corps de `analyze_input` (lignes 211-232), remplacer le bloc `spawn_blocking` par :

```rust
    tokio::task::spawn_blocking(move || {
        let (_, pids, line_counts) = scan_unique_pids(&input, &cfg.input.pid_column)?;
        let store_g = store.lock().unwrap();
        let known = store_g.load_map(&pids)?;
        let coverage = coverage_from_scan(&store_g, &pids, &line_counts)?;
        drop(store_g);
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
            coverage,
        })
    })
    .await
    .map_err(|e| e.to_string())?
```

(Seuls changements : capture de `line_counts`, une seule prise de verrou `store_g` couvrant `load_map` + `coverage_from_scan`, et le champ `coverage` dans `InputStats`.)

- [ ] **Step 4 : Vérifier la compilation**

Run (depuis `client/src-tauri/`) : `cargo build`
Expected : compile sans erreur (avertissements éventuels tolérés). `Store` et `HashMap` sont déjà importés dans `commands.rs`.

- [ ] **Step 5 : Commit**

```bash
git add client/src-tauri/src/commands.rs
git commit -m "feat(superpopaul): analyze_input calcule la couverture annuaire (repli, pas de 2e scan)"
```

---

## Task 4 : Rapport HTML — champ, variables CSS, section, tests

**Files:**
- Modify: `client/src-tauri/src/report.rs` (struct `ReportData` ~10-26 ; CSS `:root`+médias ~32-103 ; `render` avant le pied de page ~253 ; section helper ; module de tests ~487+)

- [ ] **Step 1 : Écrire d'abord les tests de la section (test d'abord)**

Dans `client/src-tauri/src/report.rs`, module `#[cfg(test)] mod tests`, ajouter en tête du module (après les `use`) le fixture vide `static` et deux helpers, puis trois tests. Insérer juste avant `fn snap()` (ligne 447) :

```rust
    use crate::coverage::{Coverage, PeppolCoverage, PpfCoverage};

    // Couverture « vide » (aucun annuaire chargé) : référence 'static, permet de
    // garder `data()` inchangé et laisse la section absente du rendu.
    static EMPTY_COV: Coverage = Coverage {
        total_lines: 0,
        eligible_0225: 0,
        non_applicable: 0,
        peppol: None,
        ppf: None,
    };

    fn cov_full() -> Coverage {
        Coverage {
            total_lines: 1000,
            eligible_0225: 900,
            non_applicable: 100,
            peppol: Some(PeppolCoverage { present: 812 }),
            ppf: Some(PpfCoverage { present: 640, active: 590, pdp_definie: 610, usable: 570 }),
        }
    }

    fn data_with<'a>(s: &'a Snapshot, cov: &'a Coverage) -> ReportData<'a> {
        ReportData { coverage: cov, ..data(s) }
    }
```

Puis ajouter ces trois tests à la fin du module (avant l'accolade fermante `}` du `mod tests`) :

```rust
    #[test]
    fn couverture_section_rendue_et_distincte_du_reseau() {
        let s = snap();
        let cov = cov_full();
        let html = render(&data_with(&s, &cov));
        assert!(html.contains("Présence déclarative en annuaire"), "titre section");
        assert!(html.contains("Annuaire Peppol"), "ligne Peppol");
        assert!(html.contains("PPF utilisable"), "ligne usable");
        assert!(html.contains("812"), "présent Peppol");
        assert!(html.contains("570"), "usable PPF");
        // Coexiste avec le réseau, sans le remplacer.
        assert!(html.contains("Provisionnés Réseau Peppol"), "KPI réseau conservé");
    }

    #[test]
    fn couverture_absente_si_aucun_annuaire() {
        // data() utilise EMPTY_COV (les deux annuaires None) → pas de section.
        let html = render(&data(&snap()));
        assert!(!html.contains("Présence déclarative en annuaire"));
    }

    #[test]
    fn couverture_variables_ppf_dans_les_trois_themes() {
        // --ppf-l4 défini pour dark (racine), clair (@media) et impression.
        let html = render(&data(&snap()));
        assert!(
            html.matches("--ppf-l4").count() >= 3,
            "variable PPF absente d'un contexte de thème"
        );
    }
```

- [ ] **Step 2 : Lancer les tests → ils échouent à la compilation (`coverage` inconnu de `ReportData`)**

Run : `cargo test --lib report -- --nocolor`
Expected : FAIL — erreur de compilation « missing field `coverage` » / champ inconnu.

- [ ] **Step 3 : Ajouter le champ `coverage` à `ReportData`**

Dans `report.rs`, struct `ReportData<'a>` (lignes 10-26), ajouter après `record_plural` :

```rust
    /// Couverture déclarative des annuaires (Peppol + PPF). Section masquée si
    /// aucun annuaire n'est chargé (`peppol` et `ppf` tous deux `None`).
    pub coverage: &'a crate::coverage::Coverage,
```

Et dans le `data()` de test (lignes 488-496), ajouter le champ pour pointer sur le fixture vide :

```rust
    fn data(s: &Snapshot) -> ReportData<'_> {
        ReportData {
            file_name: "clients_2026.csv",
            date_longue: "16 juillet 2026",
            date_heure: "16/07/2026 18:42",
            today: "2026-07-16".parse().unwrap(),
            version: "0.3.4",
            snapshot: s,
            record_plural: "lignes",
            coverage: &EMPTY_COV,
        }
    }
```

- [ ] **Step 4 : Ajouter les variables `--ppf-l*` au CSS du rapport (dark + clair + impression)**

Dans la constante `CSS` de `report.rs` :

Bloc `:root` (après la ligne `--green-later: ...;`, ligne 37) — ajouter :

```css
    --ppf-l1: #6f6aa8; --ppf-l2: #8a80d4; --ppf-l3: #a892ff; --ppf-l4: #c3b6ff;
```

Bloc `@media (prefers-color-scheme: light)` (dans le `:root {...}`, ligne 96) — ajouter à la fin de la ligne, avant `}` :

```css
 --ppf-l1: #8b86c4; --ppf-l2: #7a6fd0; --ppf-l3: #6a58c8; --ppf-l4: #5741b0;
```

Bloc `@media print` (dans le `:root {...}`, ligne 100) — même ajout :

```css
 --ppf-l1: #8b86c4; --ppf-l2: #7a6fd0; --ppf-l3: #6a58c8; --ppf-l4: #5741b0;
```

Puis, toujours dans `CSS`, avant la fermeture `"#;` (ligne 104), ajouter les classes de la section :

```css
  .cov-elig { color: var(--muted); font-size: 13px; margin: 0 0 16px; }
  .cov-elig b { color: var(--fg); }
  .cov-row { display: grid; grid-template-columns: 200px 1fr 128px; gap: 12px;
    align-items: center; padding: 5px 0; font-size: 14px; }
  .cov-name { display: flex; align-items: center; gap: 8px; min-width: 0; }
  .cov-name .tag { color: var(--muted); font-size: 11.5px; }
  .cov-sw { width: 9px; height: 9px; border-radius: 3px; flex: none; }
  .cov-sub .cov-name { padding-left: 18px; color: var(--muted); }
  .cov-n { text-align: right; font-variant-numeric: tabular-nums; color: var(--muted); white-space: nowrap; }
  .cov-n b { color: var(--fg); }
  .cov-sub.last .cov-n b { color: var(--ppf-l4); }
  .cov-gh { color: var(--muted); font-size: 12px; text-transform: uppercase;
    letter-spacing: .06em; margin: 16px 0 4px; }
```

- [ ] **Step 5 : Ajouter la fonction `coverage_section` et l'appeler dans `render`**

Dans `report.rs`, ajouter cette fonction juste avant `fn kpi(` (ligne 267, après le `}` de `render`) :

```rust
// Section « Présence déclarative en annuaire » : barres (pas anneaux) pour la
// distinguer du réseau. Rendue seulement si au moins un annuaire est chargé.
// Dénominateur = éligibles 0225 ; « lignes » remplacé par le libellé record.
fn coverage_section(html: &mut String, c: &crate::coverage::Coverage, record_plural: &str) {
    if c.peppol.is_none() && c.ppf.is_none() {
        return;
    }
    let denom = c.eligible_0225;
    let pct = |n: usize| -> String {
        if denom == 0 { "—".to_string() } else { format!("{} %", (n * 100 + denom / 2) / denom) }
    };
    let width = |n: usize| -> f64 {
        if denom == 0 { 0.0 } else { n as f64 * 100.0 / denom as f64 }
    };
    let mut row = |html: &mut String, sub: bool, last: bool, color: &str, name: &str, tag: &str, n: usize, bold_name: bool| {
        let cls = if sub { if last { "cov-row cov-sub last" } else { "cov-row cov-sub" } } else { "cov-row" };
        let name_html = if bold_name { format!("<b>{name}</b>") } else { name.to_string() };
        let tag_html = if tag.is_empty() { String::new() } else { format!(" <span class=\"tag\">{tag}</span>") };
        html.push_str(&format!(
            "<div class=\"{cls}\"><span class=\"cov-name\">\
             <span class=\"cov-sw\" style=\"background:var(--{color})\"></span>{name_html}{tag_html}</span>\
             <span class=\"bar\"><i style=\"width:{:.0}%;background:var(--{color})\"></i></span>\
             <span class=\"cov-n\"><b>{}</b> / {} · {}</span></div>\n",
            width(n), fmt_int(n as u64), fmt_int(denom as u64), pct(n)
        ));
    };

    html.push_str("<h2>Présence déclarative en annuaire</h2>\n<div class=\"pa\">\n");
    html.push_str(&format!(
        "<p class=\"cov-elig\"><b>{}</b> éligibles 0225 / <b>{}</b> {} · <b>{}</b> non applicables — \
         présence déclarée dans les annuaires chargés, distincte du « Provisionnés Réseau Peppol » ci-dessus.</p>\n",
        fmt_int(denom as u64),
        fmt_int(c.total_lines as u64),
        record_plural,
        fmt_int(c.non_applicable as u64),
    ));
    if let Some(p) = c.peppol {
        row(html, false, false, "green", "Annuaire Peppol", "", p.present, false);
    }
    if let Some(p) = c.ppf {
        html.push_str("<div class=\"cov-gh\">Annuaire PPF — présent → utilisable</div>\n");
        row(html, false, false, "ppf-l1", "Annuaire PPF", "", p.present, true);
        row(html, true, false, "ppf-l2", "PPF actif", "motif C/P", p.active, false);
        row(html, true, false, "ppf-l3", "PDP définie", "réelle", p.pdp_definie, false);
        row(html, true, true, "ppf-l4", "PPF utilisable", "actif + PDP réelle", p.usable, false);
    }
    html.push_str("</div>\n");
}
```

Puis, dans `render`, appeler la section juste avant le pied de page. Insérer entre la fin de la section « Plateformes » (ligne 251, `html.push_str("</div>\n");`) et le commentaire `// Pied de page.` (ligne 253) :

```rust
    // Présence déclarative en annuaire (Peppol + PPF) — après le réseau, dont
    // elle est explicitement distincte.
    coverage_section(&mut html, d.coverage, d.record_plural);
```

Note : `name`/`tag` sont des littéraux internes (jamais de donnée non fiable), leur interpolation directe est sûre ; les valeurs numériques passent par `fmt_int`.

- [ ] **Step 6 : Lancer les tests du rapport**

Run : `cargo test --lib report -- --nocolor`
Expected : PASS — tous les tests existants + les 3 nouveaux (`couverture_section_rendue_et_distincte_du_reseau`, `couverture_absente_si_aucun_annuaire`, `couverture_variables_ppf_dans_les_trois_themes`).

- [ ] **Step 7 : Commit**

```bash
git add client/src-tauri/src/report.rs
git commit -m "feat(superpopaul): rapport — section « présence déclarative en annuaire » (TDD)"
```

---

## Task 5 : `export_report` async + couverture dans le rapport

**Files:**
- Modify: `client/src-tauri/src/commands.rs` (`export_report` ~473-500)

- [ ] **Step 1 : Rendre `export_report` async et y calculer la couverture**

Remplacer intégralement la fonction `export_report` (lignes 473-500) par :

```rust
#[tauri::command]
pub async fn export_report(state: State<'_, AppState>) -> Result<String, String> {
    let (snapshot, file_name) = {
        let last = state.last_run.lock().unwrap();
        let last = last
            .as_ref()
            .ok_or_else(|| String::from("Aucun run terminé à rapporter."))?;
        (last.snapshot.clone(), last.file_name.clone())
    };
    let cfg = state.current_config()?;
    let input = state.input_path()?;
    let store = state.store.clone();
    // Scan CSV + requêtes store : bloquants, hors executor tokio.
    tokio::task::spawn_blocking(move || {
        let (_, pids, line_counts) = scan_unique_pids(&input, &cfg.input.pid_column)?;
        let coverage = {
            let store_g = store.lock().unwrap();
            coverage_from_scan(&store_g, &pids, &line_counts)?
        };
        let now = chrono::Local::now();
        let html = report::render(&report::ReportData {
            file_name: &file_name,
            date_longue: &report::date_fr_longue(&now),
            date_heure: &now.format("%d/%m/%Y %H:%M").to_string(),
            today: now.date_naive(),
            version: env!("CARGO_PKG_VERSION"),
            snapshot: &snapshot,
            record_plural: cfg.input.record_label.plural(),
            coverage: &coverage,
        });
        let out = resolved_out_dir(&input, &cfg.output.dir).join(format!(
            "{}_rapport.html",
            input.file_stem().unwrap_or_default().to_string_lossy()
        ));
        std::fs::write(&out, html).map_err(|e| format!("écriture du rapport : {e}"))?;
        Ok(out.display().to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}
```

- [ ] **Step 2 : Vérifier la compilation + tous les tests Rust**

Run (depuis `client/src-tauri/`) : `cargo test -- --nocolor`
Expected : PASS — la suite complète. `export_report` reste enregistrée telle quelle dans `lib.rs` (le passage à `async` ne change pas `generate_handler!`).

- [ ] **Step 3 : Commit**

```bash
git add client/src-tauri/src/commands.rs
git commit -m "feat(superpopaul): export_report async — couverture annuaire dans le rapport"
```

---

## Task 6 : Panneau `#coverage` dans `index.html`

**Files:**
- Modify: `client/src/index.html` (étape 3, entre `#run-mode-hint` ~136 et `#cockpit` ~137)

- [ ] **Step 1 : Insérer le panneau, frère juste avant `#cockpit`**

Dans `client/src/index.html`, entre `<p id="run-mode-hint" class="field-hint"></p>` (ligne 136) et `<div id="cockpit" class="hidden">` (ligne 137), insérer :

```html
      <!-- Présence en annuaire (déclaratif, indépendant du run) : visible dès
           l'entrée de l'étape 3, AVANT le run. Hors #cockpit (qui est masqué
           tant qu'aucun run n'a démarré). Rempli par renderCoverage(). -->
      <div id="coverage" class="hidden">
        <div class="cov-head">
          <h3>
            <svg width="15" height="15" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.4" aria-hidden="true"><path d="M8 3.4 C6.4 2.4 3.9 2.4 2.3 3 V12.4 C3.9 11.8 6.4 11.8 8 12.8 M8 3.4 C9.6 2.4 12.1 2.4 13.7 3 V12.4 C12.1 11.8 9.6 11.8 8 12.8 M8 3.4 V12.8"/></svg>
            Présence en annuaire
          </h3>
          <span class="cov-elig" id="cov-elig"></span>
        </div>
        <p class="cov-caption">Présence déclarative dans les annuaires chargés — distincte du « Provisionnés Réseau Peppol » constaté pendant le run.</p>
        <div id="cov-body"></div>
      </div>
```

- [ ] **Step 2 : Commit**

```bash
git add client/src/index.html
git commit -m "feat(superpopaul): index — panneau #coverage (frère avant #cockpit)"
```

---

## Task 7 : Styles du panneau dans `styles.css`

**Files:**
- Modify: `client/src/styles.css` (après le bloc `#biz-band`/`.biz-metric`, ~ligne 240)

- [ ] **Step 1 : Ajouter les styles**

Dans `client/src/styles.css`, après la ligne `.provisional, .provisional b { font-style: italic; }` (ligne 240) et avant le commentaire `/* Télémétrie repliable ...`, insérer :

```css
/* Présence en annuaire (étape 3, avant le run) : couverture déclarative, en
   barres (pas anneaux) pour la distinguer des tuiles-run. Réutilise les tokens
   violets PPF (--ppf-l1..l4, entonnoir large → strict) et le vert « présence ». */
#coverage { padding: 6px 4px 16px; border-bottom: 1px solid var(--border); margin-bottom: 12px; }
.cov-head { display: flex; align-items: baseline; gap: 10px; flex-wrap: wrap; margin: 0 0 4px; }
.cov-head h3 { margin: 0; font-size: 14px; font-weight: 600; display: flex; align-items: center; gap: 7px; }
.cov-head h3 svg { color: var(--gold); flex: none; }
.cov-elig { color: var(--muted); font-size: 12.5px; }
.cov-elig b { color: var(--fg); }
.cov-caption { color: var(--muted); font-size: 12px; font-style: italic; margin: 0 0 14px; }
.cov-gh { font-size: 12px; color: var(--muted); text-transform: uppercase; letter-spacing: .06em; margin: 16px 0 4px; }
.cov-row { display: grid; grid-template-columns: 210px 1fr 132px; align-items: center; gap: 12px; margin: 8px 0; }
.cov-row.cov-sub .cov-name { padding-left: 20px; position: relative; color: var(--muted); }
.cov-row.cov-sub .cov-name::before { content: "├"; position: absolute; left: 4px; color: var(--border); }
.cov-row.cov-sub.last .cov-name::before { content: "└"; }
.cov-name { font-size: 13px; display: flex; align-items: center; gap: 8px; min-width: 0; }
.cov-tag { color: var(--muted); font-size: 11px; }
.cov-swatch { width: 9px; height: 9px; border-radius: 3px; flex: none; }
.cov-bar { height: 12px; border-radius: 6px; background: var(--track); overflow: hidden; }
.cov-bar i { display: block; height: 100%; border-radius: 6px; }
.cov-num { text-align: right; font-variant-numeric: tabular-nums; color: var(--muted); white-space: nowrap; font-size: 13px; }
.cov-num b { color: var(--fg); font-size: 15px; }
.cov-pct { display: inline-block; min-width: 4ch; }
.cov-row.cov-sub.last .cov-num b { color: var(--ppf-l4); }
```

- [ ] **Step 2 : Commit**

```bash
git add client/src/styles.css
git commit -m "style(superpopaul): styles du panneau présence en annuaire"
```

---

## Task 8 : `renderCoverage` dans `cockpit.js` + branchement

**Files:**
- Modify: `client/src/cockpit.js` (fonction d'entrée étape 3 ~123-145 ; nouvelle fonction `renderCoverage`)

- [ ] **Step 1 : Ajouter la fonction `renderCoverage`**

Dans `client/src/cockpit.js`, ajouter cette fonction juste avant `function suggestMode(s)` (ligne 148) :

```js
/** Panneau « Présence en annuaire » (couverture déclarative). Reçoit
 *  InputStats.coverage. Masque le panneau si aucun annuaire n'est chargé
 *  (peppol ET ppf null). Numéros uniquement dynamiques → sûr sans innerHTML. */
function renderCoverage(cov) {
  const panel = $("coverage");
  if (!cov || (!cov.peppol && !cov.ppf)) { panel.classList.add("hidden"); return; }
  const denom = cov.eligible_0225;
  const pctVal = (n) => (denom > 0 ? Math.round((n * 100) / denom) : 0);
  const pctLabel = (n) => (denom > 0 ? `${pctVal(n)} %` : "—");
  const swatch = (color) => h("span", { class: "cov-swatch", style: `background:var(--${color})` });
  const bar = (n, color) => h("span", { class: "cov-bar" },
    h("i", { style: `width:${pctVal(n)}%;background:var(--${color})` }));
  const num = (n) => h("span", { class: "cov-num" },
    h("b", {}, fmt(n)), ` / ${fmt(denom)} `, h("span", { class: "cov-pct" }, pctLabel(n)));

  $("cov-elig").replaceChildren(
    h("b", {}, fmt(denom)), ` éligibles 0225 / ${fmt(cov.total_lines)} lignes · `,
    h("b", {}, fmt(cov.non_applicable)), " non applicables");

  const rows = [];
  if (cov.peppol) {
    rows.push(h("div", { class: "cov-row" },
      h("span", { class: "cov-name" }, swatch("green"), " Annuaire Peppol"),
      bar(cov.peppol.present, "green"), num(cov.peppol.present)));
  }
  if (cov.ppf) {
    const p = cov.ppf;
    rows.push(h("div", { class: "cov-gh" }, "Annuaire PPF — présent → utilisable"));
    rows.push(h("div", { class: "cov-row" },
      h("span", { class: "cov-name" }, swatch("ppf-l1"), " ", h("b", {}, "Annuaire PPF")),
      bar(p.present, "ppf-l1"), num(p.present)));
    const sub = (label, tag, n, color, last) =>
      h("div", { class: "cov-row cov-sub" + (last ? " last" : "") },
        h("span", { class: "cov-name" }, swatch(color), ` ${label} `, h("span", { class: "cov-tag" }, tag)),
        bar(n, color), num(n));
    rows.push(sub("PPF actif", "motif C/P", p.active, "ppf-l2", false));
    rows.push(sub("PDP définie", "réelle", p.pdp_definie, "ppf-l3", false));
    rows.push(sub("PPF utilisable", "actif + PDP réelle", p.usable, "ppf-l4", true));
  }
  $("cov-body").replaceChildren(...rows);
  panel.classList.remove("hidden");
}
```

- [ ] **Step 2 : Masquer le panneau au nettoyage d'écran et le rendre après `analyze_input`**

Dans la fonction d'entrée de l'étape 3 (lignes 128-144) :

a) Dans le bloc de nettoyage (après `$("run-stats").classList.add("hidden");`, ligne 134), ajouter :

```js
  $("coverage").classList.add("hidden");
```

b) Juste après `renderRunStats(s);` (ligne 140), ajouter :

```js
    renderCoverage(s.coverage);
```

- [ ] **Step 3 : Commit**

```bash
git add client/src/cockpit.js
git commit -m "feat(superpopaul): cockpit — renderCoverage (panneau présence en annuaire)"
```

---

## Task 9 : Vérification finale (tests Rust + GUI manuelle)

**Files:** aucun (vérification).

- [ ] **Step 1 : Suite Rust complète**

Run (depuis `client/src-tauri/`) : `cargo test -- --nocolor`
Expected : PASS, aucun test ignoré. Noter le total (doit augmenter de 8 + 3 = 11 tests vs base).

- [ ] **Step 2 : Clippy (le projet suit clippy — cf. commit `b3bb4bd`)**

Run : `cargo clippy --all-targets -- -D warnings`
Expected : aucun warning.

- [ ] **Step 3 : Validation GUI manuelle (maquette validée comme référence)**

Lancer l'app (`cargo tauri dev` depuis `client/`, ou le workflow habituel). Vérifier :

1. **Aucun annuaire chargé** → à l'étape 3, panneau « Présence en annuaire » **absent**.
2. **Annuaire Peppol seul** (onglet Fichiers) puis étape 3 → panneau visible **avant** le run, bloc Peppol (vert) seul, dénominateur « N éligibles 0225 / M lignes · K non applicables » en clair. Bloc PPF absent.
3. **Annuaire PPF seul** → bloc PPF (entonnoir violet l1→l4) seul, `PPF utilisable` (l4) en avant ; bloc Peppol absent.
4. **Les deux chargés** → les deux blocs, conformes à la maquette validée.
5. **Rapport HTML** (après un run, bouton « Rapport HTML ») → section « Présence déclarative en annuaire » présente, distincte de « Provisionnés Réseau Peppol », entonnoir PPF lisible ; **basculer l'OS en thème clair** et rouvrir → violets PPF lisibles sur fond clair.
6. **Chiffres cohérents** : le total de lignes et les éligibles 0225 correspondent au fichier d'entrée ; croiser 1-2 valeurs avec les colonnes CSV `in_directory`/`annuaire_ppf`/`ppf_usable` d'un export du même fichier.

- [ ] **Step 4 : Nettoyer le fichier de maquette (scratchpad) — rien à committer.**

---

## Self-Review (rempli à l'écriture du plan)

**Couverture spec :**
- Modèle temporel (visible avant run) → Task 3 (analyze_input) + Task 6/8 (panneau frère hors #cockpit). ✅
- Dénominateur éligibles 0225 par ligne, non-applicables en clair → Task 1 (`compute`) + Task 3 (`coverage_from_scan` pondération `line_counts`) + Task 8 (affichage). ✅
- 5 notions (Peppol + 4 PPF) → structs Task 1, rendu Task 4 (rapport) + Task 8 (cockpit). ✅
- Gates indépendants → Task 3 (`coverage_from_scan`) + `Option` Task 1. ✅
- Distinction annuaire ≠ réseau → libellés Task 4 (« présence déclarative », « distincte du Provisionnés Réseau ») + test `couverture_section_rendue_et_distincte_du_reseau`. ✅
- Fonction pure testable + 8 cas (dont entonnoir discriminant + round-trip serde) → Task 1. ✅
- Rapport : section + `--ppf-l*` thème clair → Task 4. ✅
- Pas de parité CLI → aucune tâche CLI. ✅
- Sécurité UI (pas d'innerHTML dynamique) → Task 8 (h()/replaceChildren, valeurs numériques uniquement). ✅

**Placeholders :** aucun — code complet à chaque étape.

**Cohérence des types :** `Coverage`/`PeppolCoverage`/`PpfCoverage` (champs `present`/`active`/`pdp_definie`/`usable`) identiques Task 1 → 3 → 4 → 8. `PpfFlags.in_ppf` (et non `annuaire_ppf`) utilisé côté Rust (Task 1/3). Champs JSON consommés par JS (`eligible_0225`, `total_lines`, `non_applicable`, `peppol.present`, `ppf.{present,active,pdp_definie,usable}`) = noms serde par défaut (snake_case), vérifiés par `round_trip_serde` (Task 1) et consommés tels quels en Task 8.
