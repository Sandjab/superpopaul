# Répartition des CF par plateforme — plan d'implémentation

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ajouter au rapport HTML une section « Répartition des {record_plural} par plateforme » : un ring (top 5 PA + « autres » + « sans plateforme », base = total du fichier) et la liste détaillée de toutes les plateformes, triée décroissant, % à 2 décimales.

**Architecture:** Module pur `repartition.rs` (agrégation testable sans UI) + glue `repartition_from_scan` dans `commands.rs` (calcul à l'export via le scan existant, miroir de `coverage_from_scan`/`securisation_from_scan`) + rendu dans `report.rs`. Regroupement par nom de PA (repli code), poids = lignes/CF (`line_counts`).

**Tech Stack:** Rust (client Tauri), `cargo test` dans `client/src-tauri/`. Aucune dépendance nouvelle. Rendu HTML/SVG statique (zéro JS), style « Bleu nuit & or ».

**Spec:** `docs/superpowers/specs/2026-07-24-repartition-cf-par-plateforme-design.md`

---

## File Structure

- **Create** `client/src-tauri/src/repartition.rs` — logique pure : `PaCount`, `Repartition`, `pa_key`, `compute` + tests.
- **Modify** `client/src-tauri/src/lib.rs` — déclarer `pub mod repartition;`.
- **Modify** `client/src-tauri/src/commands.rs` — `repartition_from_scan` (glue) + branchement dans `export_report`.
- **Modify** `client/src-tauri/src/report.rs` — champ `ReportData.repartition_pa`, helper `fmt_pct2`, `repartition_section`, appel dans `render`, tokens CSS, mise à jour du helper de test `data()`.

**Note de convention (assumée) :** comme `coverage_from_scan` et `securisation_from_scan`, la fonction glue `repartition_from_scan` n'est pas testée unitairement (le codebase ne teste pas ces `*_from_scan`) — toute la logique testable vit dans le module pur `repartition.rs` (`pa_key`, `compute`), et le bout-en-bout est couvert par le test de rendu de la Tâche 3.

---

## Task 1: Module pur `repartition.rs`

**Files:**
- Create: `client/src-tauri/src/repartition.rs`
- Modify: `client/src-tauri/src/lib.rs` (ajouter `pub mod repartition;` en ordre alphabétique, après `pub mod output;` / avant `pub mod resolver;` selon l'ordre existant)

- [ ] **Step 1: Écrire le fichier avec les types, les fonctions et les tests (test-first : le corps des fonctions est un stub qui échoue)**

Create `client/src-tauri/src/repartition.rs` :

```rust
//! Répartition des lignes (CF) du fichier par plateforme de dématérialisation
//! (Point d'Accès Peppol). Agrégat PUR : aucune DB, aucune UI — la partie
//! impure (load_map) vit dans `commands.rs::repartition_from_scan`. Comptage
//! PAR LIGNE : chaque adressage unique est pondéré par son nombre de lignes.

use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaCount {
    pub nom: String,
    pub lignes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repartition {
    /// Dénominateur des % : total des lignes du fichier (plateformes + sans).
    pub total_lignes: u64,
    /// Toutes les plateformes, triées par lignes décroissant (départage par nom).
    pub plateformes: Vec<PaCount>,
    /// Lignes sans plateforme : adressage non résolu, ou résolu sans PA.
    pub sans_plateforme: u64,
}

/// Clé de regroupement d'une résolution : nom de PA, repli sur le code si le
/// nom est absent/vide, `None` si ni l'un ni l'autre (→ « sans plateforme »).
pub fn pa_key(pa_name: Option<&str>, pa_code: Option<&str>) -> Option<String> {
    pa_name
        .filter(|s| !s.is_empty())
        .or(pa_code.filter(|s| !s.is_empty()))
        .map(str::to_string)
}

/// Agrège les entrées `(clé PA ou None, nombre de lignes)` : somme par PA,
/// `None` → `sans_plateforme`, tri décroissant (départage alphabétique stable).
pub fn compute(entrees: &[(Option<String>, u64)]) -> Repartition {
    let mut par_pa: BTreeMap<String, u64> = BTreeMap::new();
    let mut sans_plateforme = 0u64;
    let mut total_lignes = 0u64;
    for (cle, n) in entrees {
        total_lignes += n;
        match cle {
            Some(nom) => *par_pa.entry(nom.clone()).or_insert(0) += n,
            None => sans_plateforme += n,
        }
    }
    // BTreeMap → ordre alphabétique ; sort_by stable préserve ce départage à
    // égalité d'effectif (même discipline que telemetry::ranked).
    let mut plateformes: Vec<PaCount> = par_pa
        .into_iter()
        .map(|(nom, lignes)| PaCount { nom, lignes })
        .collect();
    plateformes.sort_by(|a, b| b.lignes.cmp(&a.lignes));
    Repartition { total_lignes, plateformes, sans_plateforme }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pa_key_prefere_le_nom() {
        assert_eq!(pa_key(Some("Cegedim"), Some("PA01")), Some("Cegedim".into()));
    }

    #[test]
    fn pa_key_replie_sur_le_code_si_nom_vide_ou_absent() {
        assert_eq!(pa_key(Some(""), Some("PA01")), Some("PA01".into()));
        assert_eq!(pa_key(None, Some("PA01")), Some("PA01".into()));
    }

    #[test]
    fn pa_key_none_si_ni_nom_ni_code() {
        assert_eq!(pa_key(None, None), None);
        assert_eq!(pa_key(Some(""), Some("")), None);
    }

    #[test]
    fn compute_agrege_les_lignes_par_pa() {
        let e = vec![
            (Some("Cegedim".to_string()), 10u64),
            (Some("Docaposte".to_string()), 4),
            (Some("Cegedim".to_string()), 6),
        ];
        let r = compute(&e);
        assert_eq!(r.total_lignes, 20);
        assert_eq!(r.sans_plateforme, 0);
        assert_eq!(
            r.plateformes,
            vec![
                PaCount { nom: "Cegedim".into(), lignes: 16 },
                PaCount { nom: "Docaposte".into(), lignes: 4 },
            ]
        );
    }

    #[test]
    fn compute_none_compte_en_sans_plateforme() {
        let e = vec![(Some("Cegedim".to_string()), 5u64), (None, 3)];
        let r = compute(&e);
        assert_eq!(r.sans_plateforme, 3);
        assert_eq!(r.total_lignes, 8);
        assert_eq!(r.plateformes, vec![PaCount { nom: "Cegedim".into(), lignes: 5 }]);
    }

    #[test]
    fn compute_trie_decroissant_puis_alphabetique() {
        // À effectif égal (A et B = 5), l'ordre alphabétique départage.
        let e = vec![
            (Some("B".to_string()), 5u64),
            (Some("A".to_string()), 5),
            (Some("C".to_string()), 9),
        ];
        let r = compute(&e);
        let noms: Vec<&str> = r.plateformes.iter().map(|p| p.nom.as_str()).collect();
        assert_eq!(noms, vec!["C", "A", "B"]);
    }
}
```

Add to `client/src-tauri/src/lib.rs` (respect the existing alphabetical order of `pub mod` lines):

```rust
pub mod repartition;
```

- [ ] **Step 2: Lancer les tests → ils passent (la logique ci-dessus est déjà complète)**

Run: `cd client/src-tauri && cargo test --lib repartition::tests`
Expected: PASS (7 tests). Si un test échoue, corriger l'implémentation, pas le test.

> TDD note : ici le module est écrit d'un bloc car chaque fonction est triviale et entièrement couverte. Si tu préfères le cycle strict, commente le corps de `compute`/`pa_key` (`todo!()`), lance pour voir l'échec, puis rétablis.

- [ ] **Step 3: Vérifier la compilation globale et l'absence de warning**

Run: `cargo build 2>&1 | grep -iE "warning|error"`
Expected: aucune sortie.

- [ ] **Step 4: Commit**

```bash
git add client/src-tauri/src/repartition.rs client/src-tauri/src/lib.rs
git commit -m "feat(superpopaul): module repartition (lignes par plateforme, agrégat pur)" \
  -m "Claude-Session: https://claude.ai/code/session_01WiCbUuyS9beUGgMePtptyt"
```

---

## Task 2: Câbler la donnée jusqu'au rapport (sans rendu visible)

Ajoute le champ `repartition_pa` à `ReportData`, la fonction glue `repartition_from_scan`, et branche le calcul dans `export_report`. Aucune section n'est encore rendue → le rendu ne change pas, les tests existants restent verts.

**Files:**
- Modify: `client/src-tauri/src/report.rs` (struct `ReportData`, ~ligne 10-35 ; helper de test `data()` dans `mod tests`)
- Modify: `client/src-tauri/src/commands.rs` (`repartition_from_scan` près de `securisation_from_scan` ~ligne 139 ; `export_report` ~ligne 562-604)

- [ ] **Step 1: Ajouter le champ à `ReportData`**

In `client/src-tauri/src/report.rs`, dans `pub struct ReportData<'a>`, après le champ `securisation` :

```rust
    /// Répartition des lignes par plateforme (PA). `None` = pas de résolution
    /// exploitable → section non rendue.
    pub repartition_pa: Option<&'a crate::repartition::Repartition>,
```

- [ ] **Step 2: Mettre à jour le helper de test `data()` pour qu'il compile**

In `client/src-tauri/src/report.rs`, dans `mod tests`, repérer la fonction `data()` (celle qui construit un `ReportData` de base, source des `..data(s)`). Ajouter le champ dans son littéral :

```rust
        repartition_pa: None,
```

(Les tests utilisant `..data(s)` / `..data_with(...)` restent inchangés grâce au struct-update.)

- [ ] **Step 3: Écrire la fonction glue `repartition_from_scan` dans `commands.rs`**

In `client/src-tauri/src/commands.rs`, juste après `securisation_from_scan` (après sa `}` de fin, ~ligne 176) :

```rust
/// Répartition des lignes par plateforme (PA) à partir d'un scan déjà fait.
/// Population : lignes du fichier courant (`line_counts`), PA du dernier état
/// de résolution connu en base (`load_map`). Regroupement par nom de PA (repli
/// code). Miroir de `securisation_from_scan` ; logique testée dans `repartition`.
fn repartition_from_scan(
    store: &Store,
    pids: &[String],
    line_counts: &HashMap<String, u64>,
) -> Result<crate::repartition::Repartition, String> {
    let resolutions = store.load_map(pids)?;
    let mut entrees: Vec<(Option<String>, u64)> = Vec::with_capacity(pids.len());
    for p in pids {
        let n = *line_counts.get(p).unwrap_or(&0);
        let cle = resolutions
            .get(p)
            .and_then(|r| crate::repartition::pa_key(r.pa_name.as_deref(), r.pa_code.as_deref()));
        entrees.push((cle, n));
    }
    Ok(crate::repartition::compute(&entrees))
}
```

- [ ] **Step 4: Calculer et passer la répartition dans `export_report`**

In `client/src-tauri/src/commands.rs`, dans `export_report`, à l'intérieur du `spawn_blocking`, le bloc `match scan_unique_pids(...)` produit `(coverage, securisation)`. Le remplacer pour produire aussi la répartition, en réutilisant le même scan :

Remplacer :

```rust
        let (coverage, securisation) = match scan_unique_pids(&input, &cfg.input.pid_column) {
            Ok((_, pids, line_counts)) => {
                let now_utc = chrono::Utc::now();
                let store_g = store.lock().unwrap();
                let cov = coverage_from_scan(&store_g, &pids, &line_counts, &cfg.ppf.motifs())
                    .unwrap_or(crate::coverage::Coverage::EMPTY);
                let secu =
                    securisation_from_scan(&store_g, &pids, &line_counts, now_utc, &cfg.ppf.motifs())
                        .ok()
                        .flatten();
                (cov, secu)
            }
            Err(_) => (crate::coverage::Coverage::EMPTY, None),
        };
```

par :

```rust
        let (coverage, securisation, repartition) = match scan_unique_pids(&input, &cfg.input.pid_column) {
            Ok((_, pids, line_counts)) => {
                let now_utc = chrono::Utc::now();
                let store_g = store.lock().unwrap();
                let cov = coverage_from_scan(&store_g, &pids, &line_counts, &cfg.ppf.motifs())
                    .unwrap_or(crate::coverage::Coverage::EMPTY);
                let secu =
                    securisation_from_scan(&store_g, &pids, &line_counts, now_utc, &cfg.ppf.motifs())
                        .ok()
                        .flatten();
                let rep = repartition_from_scan(&store_g, &pids, &line_counts).ok();
                (cov, secu, rep)
            }
            Err(_) => (crate::coverage::Coverage::EMPTY, None, None),
        };
```

Puis, dans le littéral `report::ReportData { ... }` juste en dessous, ajouter après `securisation: securisation.as_ref(),` :

```rust
            repartition_pa: repartition.as_ref(),
```

- [ ] **Step 5: Compiler et lancer toute la suite**

Run: `cargo test --lib 2>&1 | tail -5`
Expected: tous les tests passent (aucune régression ; le rendu est inchangé car la section n'est pas encore émise).

Run: `cargo build 2>&1 | grep -iE "warning|error"`
Expected: aucune sortie.

- [ ] **Step 6: Commit**

```bash
git add client/src-tauri/src/report.rs client/src-tauri/src/commands.rs
git commit -m "feat(superpopaul): calcule la répartition par plateforme à l'export (câblage)" \
  -m "Claude-Session: https://claude.ai/code/session_01WiCbUuyS9beUGgMePtptyt"
```

---

## Task 3: Rendu de la section (ring + liste)

**Files:**
- Modify: `client/src-tauri/src/report.rs` (helper `fmt_pct2` près de `fmt_pct` ~ligne 545 ; `repartition_section` ; appel dans `render` après la section PA « en adressages uniques » ~ligne 291 ; tokens CSS dans `const CSS` ~ligne 40-49 ; test dans `mod tests`)

- [ ] **Step 1: Écrire le test de rendu (échouera : la section n'est pas émise)**

In `client/src-tauri/src/report.rs`, dans `mod tests`, ajouter :

```rust
    fn repartition_full() -> crate::repartition::Repartition {
        use crate::repartition::PaCount;
        crate::repartition::Repartition {
            total_lignes: 12_000,
            plateformes: vec![
                PaCount { nom: "Cegedim e-Business".into(), lignes: 3_900 },
                PaCount { nom: "Docaposte".into(), lignes: 2_640 },
                PaCount { nom: "Generix".into(), lignes: 1_800 },
                PaCount { nom: "Esker".into(), lignes: 1_200 },
                PaCount { nom: "Tenor".into(), lignes: 720 },
                PaCount { nom: "Edicom".into(), lignes: 300 },
            ],
            sans_plateforme: 1_440,
        }
    }

    #[test]
    fn repartition_section_rendue() {
        let s = snap();
        let rep = repartition_full();
        let d = ReportData { repartition_pa: Some(&rep), ..data(&s) };
        let html = render(&d);
        assert!(html.contains("par plateforme"), "titre de section");
        assert!(html.contains("Cegedim e-Business"), "nom de PA dans la liste");
        assert!(html.contains("Sans plateforme"), "segment sans plateforme dans le ring");
        // % à 2 décimales, virgule française : 3 900 / 12 000 = 32,50 %.
        assert!(html.contains("32,50\u{202F}%"), "pourcentage à 2 décimales");
        // La section « en adressages uniques » existe toujours (non remplacée).
        assert!(html.contains("en adressages uniques"), "section existante préservée");
    }

    #[test]
    fn repartition_section_absente_si_none() {
        let s = snap();
        let d = data(&s); // repartition_pa: None
        assert!(!render(&d).contains("par plateforme"), "pas de section sans données");
    }
```

- [ ] **Step 2: Lancer le test → il échoue**

Run: `cargo test --lib report::tests::repartition_section_rendue`
Expected: FAIL (le HTML ne contient pas « par plateforme » : la section n'est pas encore émise).

- [ ] **Step 3: Ajouter le helper `fmt_pct2`**

In `client/src-tauri/src/report.rs`, juste après `fmt_pct` (~ligne 552) :

```rust
/// « 32,50 % » — comme `fmt_pct` mais à 2 décimales (demande explicite pour la
/// répartition par plateforme).
fn fmt_pct2(part: u64, total: u64) -> String {
    let p = if total == 0 {
        0.0
    } else {
        part as f64 * 100.0 / total as f64
    };
    format!("{p:.2}\u{202F}%").replace('.', ",")
}
```

- [ ] **Step 4: Écrire `repartition_section`**

In `client/src-tauri/src/report.rs`, ajouter (par ex. après `securisation_section`, avant les helpers `fmt_*`) :

```rust
// Répartition des lignes par plateforme (PA) : ring top 5 + « autres » + « sans
// plateforme » (base = total du fichier), puis la liste de TOUTES les
// plateformes triée décroissant. % à 2 décimales sur le total. Rendue seulement
// s'il y a un total et au moins une plateforme. Noms de PA échappés (SMP =
// entrée non fiable).
fn repartition_section(html: &mut String, rep: &crate::repartition::Repartition, record_plural: &str) {
    if rep.total_lignes == 0 || rep.plateformes.is_empty() {
        return;
    }
    // Couleurs catégorielles du top 5 (tokens CSS), puis neutres.
    const TOP_COLORS: [&str; 5] = ["pa-1", "pa-2", "pa-3", "pa-4", "pa-5"];
    let n_top = rep.plateformes.len().min(5);
    let reste: u64 = rep.plateformes[n_top..].iter().map(|p| p.lignes).sum();
    let reste_nb = rep.plateformes.len().saturating_sub(n_top);

    // Segments du ring : (classe couleur, libellé, lignes), vides omis.
    let mut segments: Vec<(&str, String, u64)> = Vec::new();
    for (i, p) in rep.plateformes[..n_top].iter().enumerate() {
        segments.push((TOP_COLORS[i], p.nom.clone(), p.lignes));
    }
    if reste > 0 {
        segments.push(("pa-autres", format!("Autres ({reste_nb} plateformes)"), reste));
    }
    if rep.sans_plateforme > 0 {
        segments.push(("pa-sans", "Sans plateforme".to_string(), rep.sans_plateforme));
    }

    html.push_str(&format!(
        "<h2>Répartition des {record_plural} par plateforme \
         <span class=\"unit\">· en {record_plural}</span></h2>\n<div class=\"ring-row\">\n"
    ));

    // Anneau SVG (même géométrie que le ring principal : r=80, RING_C).
    html.push_str(
        "<svg width=\"210\" height=\"210\" viewBox=\"0 0 210 210\" role=\"img\" \
         aria-label=\"Répartition par plateforme\">\n<g transform=\"rotate(-90 105 105)\">\n\
         <circle cx=\"105\" cy=\"105\" r=\"80\" fill=\"none\" stroke=\"var(--track)\" stroke-width=\"26\"/>\n",
    );
    let mut cum = 0.0;
    for (color, _, lignes) in &segments {
        let len = *lignes as f64 / rep.total_lignes as f64 * RING_C;
        html.push_str(&format!(
            "<circle cx=\"105\" cy=\"105\" r=\"80\" fill=\"none\" stroke=\"var(--{color})\" \
             stroke-width=\"26\" stroke-dasharray=\"{len:.1} {RING_C:.2}\" \
             stroke-dashoffset=\"{:.1}\"/>\n",
            -cum
        ));
        cum += len;
    }
    html.push_str(&format!(
        "</g>\n<text x=\"105\" y=\"110\" text-anchor=\"middle\" class=\"ring-center\">{}</text>\n\
         <text x=\"105\" y=\"128\" text-anchor=\"middle\" class=\"ring-sub\">{record_plural} au total</text>\n</svg>\n",
        fmt_int(rep.total_lignes)
    ));

    // Légende du ring (mêmes segments, effectif + % 2 décimales).
    html.push_str("<div class=\"legend\">\n");
    for (color, label, lignes) in &segments {
        html.push_str(&format!(
            "<span class=\"dot\" style=\"background:var(--{color})\"></span><span>{}</span>\
             <span class=\"n\"><b>{}</b> · {}</span>\n",
            esc(label),
            fmt_int(*lignes),
            fmt_pct2(*lignes, rep.total_lignes)
        ));
    }
    html.push_str("</div>\n</div>\n");

    // Liste détaillée : TOUTES les plateformes, barres colorées (top 5 = couleur
    // de segment, reste neutre), largeur relative au max.
    let max = rep.plateformes.iter().map(|p| p.lignes).max().unwrap_or(1).max(1);
    html.push_str("<div class=\"pa\">\n");
    for (i, p) in rep.plateformes.iter().enumerate() {
        let color = TOP_COLORS.get(i).copied().unwrap_or("pa-autres");
        html.push_str(&format!(
            "<div class=\"pa-row\"><span class=\"pa-name\">{}</span>\
             <span class=\"bar\"><i style=\"width:{:.0}%;background:var(--{color})\"></i></span>\
             <span class=\"pa-n\"><b>{}</b> · {}</span></div>\n",
            esc(&p.nom),
            p.lignes as f64 * 100.0 / max as f64,
            fmt_int(p.lignes),
            fmt_pct2(p.lignes, rep.total_lignes)
        ));
    }
    html.push_str("</div>\n");
}
```

- [ ] **Step 5: Appeler la section dans `render`, après la section « en adressages uniques »**

In `client/src-tauri/src/report.rs`, dans `render`, juste après le bloc de la section « Plateformes … · en adressages uniques » (après son `html.push_str("</div>\n");` de fin, ~ligne 291) et avant `coverage_section(...)` :

```rust
    // Répartition en lignes/CF (jumelle « en {record_plural} » de la section
    // ci-dessus, en adressages uniques).
    if let Some(rep) = d.repartition_pa {
        repartition_section(&mut html, rep, d.record_plural);
    }
```

- [ ] **Step 6: Ajouter les tokens CSS de couleur**

In `client/src-tauri/src/report.rs`, dans `const CSS`, à la fin du bloc `:root { ... }` (avant sa `}` fermante, ~ligne 49) :

```rust
    --pa-1: #d9a83f; --pa-2: #4cc268; --pa-3: #a892ff; --pa-4: #e0873a; --pa-5: #5aa9e6;
    --pa-autres: #6b7794; --pa-sans: #3a4460;
```

> Si un bloc thème clair (`@media (prefers-color-scheme: light)` ou une classe) redéfinit la palette dans `const CSS`, y ajouter les mêmes sept tokens à l'identique (couleurs catégorielles inchangées entre thèmes). Vérifier par : `grep -n "prefers-color-scheme\|--track" src/report.rs`.

- [ ] **Step 7: Lancer le test de rendu → il passe**

Run: `cargo test --lib report::tests::repartition_section_rendue report::tests::repartition_section_absente_si_none`
Expected: PASS (2 tests).

- [ ] **Step 8: Lancer toute la suite + build sans warning**

Run: `cargo test --lib 2>&1 | tail -5`
Expected: tous verts.
Run: `cargo build 2>&1 | grep -iE "warning|error"`
Expected: aucune sortie.

- [ ] **Step 9: Commit**

```bash
git add client/src-tauri/src/report.rs
git commit -m "feat(superpopaul): section « répartition des CF par plateforme » dans le rapport" \
  -m "Ring top 5 + autres + sans plateforme (base = total fichier), liste de toutes les plateformes triée décroissant, % à 2 décimales. Placée après la section « en adressages uniques »." \
  -m "Claude-Session: https://claude.ai/code/session_01WiCbUuyS9beUGgMePtptyt"
```

---

## Task 4: Vérification finale et revue

**Files:** aucun changement de code sauf correctifs éventuels.

- [ ] **Step 1: Suite complète + clippy**

Run: `cargo test --lib 2>&1 | tail -5`
Expected: tous verts (≈ +12 tests par rapport à la baseline 272).
Run: `cargo clippy --lib 2>&1 | grep -iE "warning|error"`
Expected: aucun nouveau warning imputable aux fichiers touchés (`repartition.rs`, `report.rs`, `commands.rs`). Les warnings préexistants d'autres fichiers sont hors périmètre.

- [ ] **Step 2: Vérifier le rendu réel (validation manuelle non automatisable)**

Dans l'app (ou via un run existant), exporter un rapport et ouvrir le HTML :
- la nouvelle section apparaît après « Plateformes … · en adressages uniques » ;
- le ring totalise 100 % du fichier (top 5 + autres + sans plateforme) ;
- la liste montre toutes les plateformes, triée décroissant, sans « sans plateforme », % à 2 décimales ;
- un fichier sans aucune résolution → section absente (ou vide) sans planter.

Signaler ce qui relève de la validation humaine (le transfert visuel exact) plutôt que de l'affirmer.

- [ ] **Step 3: Revue de code (optionnelle mais recommandée)**

Envisager `superpowers:requesting-code-review` sur le diff des 3 commits.

- [ ] **Step 4: Push**

```bash
git push origin main
```

---

## Self-review (rédaction du plan)

- **Couverture du spec :** module pur (T1) ✓ ; calcul à l'export via scan (T2) ✓ ; regroupement nom→code (`pa_key`, T1) ✓ ; sans-plateforme (T1/T3) ✓ ; ring base A + liste sans sans-plateforme (T3) ✓ ; % 2 décimales base total fichier (`fmt_pct2`, T3) ✓ ; barres colorées (T3) ✓ ; emplacement après section existante (T3) ✓ ; section existante inchangée (test T3) ✓ ; hors-scope (télémétrie/CSV/CLI) respecté.
- **Placeholders :** aucun — code complet à chaque étape.
- **Cohérence des types :** `Repartition { total_lignes, plateformes: Vec<PaCount{nom,lignes}>, sans_plateforme }`, `pa_key(Option<&str>,Option<&str>)->Option<String>`, `compute(&[(Option<String>,u64)])->Repartition`, `repartition_from_scan(&Store,&[String],&HashMap<String,u64>)->Result<Repartition,String>`, `ReportData.repartition_pa: Option<&Repartition>`, `fmt_pct2(u64,u64)->String` — noms alignés entre toutes les tâches.
- **Écart assumé au spec :** ring « top 5 + autres » dès > 5 plateformes (demande littérale de l'utilisateur), là où la section existante n'agrège qu'au-delà de 6. Signalé ; réglage trivial si l'on veut aligner.
