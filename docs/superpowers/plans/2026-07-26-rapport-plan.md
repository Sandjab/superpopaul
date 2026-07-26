# Rapport de plan de charge — courbes et mise en page — plan d'implémentation

> **Pour les agents :** SOUS-SKILL REQUIS — utiliser `superpowers:subagent-driven-development`
> (recommandé) ou `superpowers:executing-plans` pour exécuter ce plan tâche par
> tâche. Les étapes sont en cases à cocher (`- [ ]`).

**But :** doter le rapport de plan de charge de ses deux courbes (parc facturant
cumulé, charge par run) et de la mise en page qui lui manque depuis toujours.

**Architecture :** deux modules Rust purs neufs — `charge.rs` (combien de
factures à chaque run) et `charts.rs` (SVG aire et barres empilées) — consommés
par `plan_report.rs`, qui reste seul responsable de la mise en page. Le CSS
partagé de `report.rs` reçoit des ajouts, aucune règle existante n'est modifiée.

**Pile :** Rust 2021, `chrono`, aucune dépendance graphique. SVG inline sans JS.
Tests : `cargo test` depuis `client/src-tauri/`.

**Spec :** `docs/superpowers/specs/2026-07-26-rapport-plan-design.md`
**Maquette validée :** `docs/superpowers/maquettes/2026-07-26-rapport-plan.html`

---

## Contexte indispensable avant de commencer

Lire la spec. Les points qu'on oublie sinon :

- **Toute donnée d'origine CSV ou SMP passe par `esc`** (`report.rs:617`,
  `pub(crate)`). Les numéros de run, les noms de plateforme et les raisons
  sociales viennent de fichiers fournis par des tiers : ce sont des entrées non
  fiables. Un `format!` direct dans du SVG ou du HTML est un défaut de sécurité,
  pas un raccourci.
- **`fmt_int`** (`report.rs:633`, `pub(crate)`) formate les entiers avec
  séparateur de milliers. L'utiliser partout plutôt que `to_string()`.
- **Modules purs** : `charge.rs` et `charts.rs` n'ont ni DB, ni UI, ni accès
  disque, comme `timeline.rs`. Ils ne décident rien — l'appelant leur donne les
  runs **retenus** et les lignes **actives**.
- **Texte en français**, y compris les noms de tests et les commentaires.
- **TDD** : le test d'abord, on le voit échouer, puis le code minimal.

Commande de test, depuis `client/src-tauri/` :

```bash
cargo test
```

État de départ attendu : **457 tests Rust verts**, 5 warnings clippy
préexistants. Le vérifier avant de commencer — si le compte diffère, s'arrêter
et le signaler.

---

## Structure des fichiers

| Fichier | Responsabilité |
|---------|----------------|
| `client/src-tauri/src/charge.rs` | **neuf** — premières factures et récurrences par run. Aucun rendu. |
| `client/src-tauri/src/charts.rs` | **neuf** — SVG aire cumulée et barres empilées. Ne connaît ni plan ni run. |
| `client/src-tauri/src/plan_report.rs` | mise en page du rapport, avertissements dérivés, indicateurs |
| `client/src-tauri/src/report.rs` | ajouts au `CSS` uniquement — le rapport de run ne doit pas bouger |
| `client/src-tauri/src/lib.rs` | déclaration des deux modules |
| `client/src-tauri/src/commands.rs` | `plan_rapport` fournit runs retenus et pool par jour de cycle |

---

## Tâche 1 : `charge.rs` — squelette et cas dégénérés

**Fichiers :**
- Créer : `client/src-tauri/src/charge.rs`
- Modifier : `client/src-tauri/src/lib.rs`

- [ ] **Étape 1 : déclarer le module**

Dans `client/src-tauri/src/lib.rs`, ajouter la ligne en respectant l'ordre
alphabétique (entre `calendrier` et `commands`) :

```rust
pub mod charge;
```

- [ ] **Étape 2 : écrire le test qui échoue**

Créer `client/src-tauri/src/charge.rs` avec **uniquement** l'en-tête, les types
et le module de tests :

```rust
//! Charge de facturation par run : premières factures et récurrences.
//!
//! Module PUR — aucune DB, aucune UI, aucun accès disque, dans la lignée de
//! `timeline`. Il ne décide pas quels runs comptent : l'appelant fournit les
//! runs **retenus** (`calendrier::runs_utilisables`) et les lignes **actives**
//! (les retirées écartées). Un run exclu du plan est donc absent de la série,
//! et les comptes qui y étaient placés ne sont comptés nulle part — c'est la
//! décision 5 de la spec, assumée.

use crate::calendrier::RunFacturation;
use crate::plan::LignePlan;
use chrono::{Datelike, NaiveDate};
use std::collections::{HashMap, HashSet};

/// Ce que facture un run : les comptes qui démarrent, et ceux qui reviennent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChargeRun {
    pub num: String,
    pub date: NaiveDate,
    /// Comptes dont la **première** facture tombe à ce run.
    pub premieres: usize,
    /// Comptes déjà en production qui refacturent à ce run.
    pub recurrences: usize,
}

impl ChargeRun {
    pub fn total(&self) -> usize {
        self.premieres + self.recurrences
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::Origine;

    fn jour(iso: &str) -> NaiveDate {
        NaiveDate::parse_from_str(iso, "%Y-%m-%d").unwrap()
    }

    fn run(num: &str, date: &str, jjs: &[u8]) -> RunFacturation {
        RunFacturation {
            num: num.into(),
            date: jour(date),
            jjs: jjs.to_vec(),
            exclu: false,
        }
    }

    /// Une ligne de plan : seuls `jj` et `run_num` comptent pour ce module.
    fn ligne(cf: &str, jj: u8, run_num: &str) -> LignePlan {
        LignePlan {
            cf: cf.into(),
            participant: "0225:1".into(),
            jj,
            raison_sociale: "ACME".into(),
            pa: "Cegedim".into(),
            mep_id: 1,
            mep_date: jour("2026-08-01"),
            run_num: run_num.into(),
            run_date: jour("2026-08-11"),
            origine: Origine::Auto,
            in_directory: true,
            resolved_at: 0,
            planned_at: 0,
            retire: None,
        }
    }

    #[test]
    fn serie_vide_sans_run_ni_ligne() {
        assert!(charge(&[], &[]).is_empty());
    }
}
```

- [ ] **Étape 3 : lancer le test et vérifier qu'il échoue**

```bash
cd client/src-tauri && cargo test charge::
```

Attendu : **échec de compilation**, `cannot find function 'charge' in this scope`.

- [ ] **Étape 4 : écrire l'implémentation minimale**

Ajouter dans `charge.rs`, avant `#[cfg(test)]` :

```rust
/// Factures émises à chaque run.
///
/// `lignes` : lignes **actives** du plan. `runs` : runs **retenus**, triés par
/// date croissante (contrat de `calendrier::runs_utilisables`).
///
/// Règle : un compte facture **une fois par mois civil**, au premier run du
/// mois dont les jours de cycle couvrent le sien.
pub fn charge(lignes: &[LignePlan], runs: &[RunFacturation]) -> Vec<ChargeRun> {
    let _ = lignes;
    runs.iter()
        .map(|r| ChargeRun {
            num: r.num.clone(),
            date: r.date,
            premieres: 0,
            recurrences: 0,
        })
        .collect()
}
```

- [ ] **Étape 5 : lancer le test et vérifier qu'il passe**

```bash
cd client/src-tauri && cargo test charge::
```

Attendu : `test charge::tests::serie_vide_sans_run_ni_ligne ... ok`

- [ ] **Étape 6 : commit**

```bash
git add client/src-tauri/src/charge.rs client/src-tauri/src/lib.rs
git commit -m "feat(superpopaul): squelette du calcul de charge par run"
```

---

## Tâche 2 : `charge.rs` — premières factures

**Fichiers :**
- Modifier : `client/src-tauri/src/charge.rs`

- [ ] **Étape 1 : écrire le test qui échoue**

Ajouter dans `mod tests` :

```rust
#[test]
fn pas_de_recurrence_avant_le_demarrage() {
    // Un seul run : le compte y démarre. Il ne peut pas y « revenir ».
    let runs = vec![run("R1", "2026-08-11", &[5])];
    let lignes = vec![ligne("CF1", 5, "R1"), ligne("CF2", 5, "R1")];
    let c = charge(&lignes, &runs);
    assert_eq!(c.len(), 1);
    assert_eq!(c[0].premieres, 2);
    assert_eq!(c[0].recurrences, 0, "une première facture n'est pas une récurrence");
}

#[test]
fn un_compte_place_sur_un_run_absent_nest_compte_nulle_part() {
    // Run exclu du plan : la ligne le désigne encore, mais il n'est pas fourni.
    // Décision 5 de la spec — la charge sous-estime alors la réalité, et c'est su.
    let runs = vec![run("R2", "2026-08-25", &[5])];
    let lignes = vec![ligne("CF1", 5, "R1")];
    let c = charge(&lignes, &runs);
    assert_eq!(c[0].premieres, 0);
    assert_eq!(c[0].recurrences, 0);
}
```

- [ ] **Étape 2 : lancer les tests et vérifier qu'ils échouent**

```bash
cd client/src-tauri && cargo test charge::
```

Attendu : `pas_de_recurrence_avant_le_demarrage` échoue —
`assertion failed: left == right, left: 0, right: 2`.

- [ ] **Étape 3 : implémenter les premières factures**

Remplacer le corps de `charge` :

```rust
pub fn charge(lignes: &[LignePlan], runs: &[RunFacturation]) -> Vec<ChargeRun> {
    let index_par_num: HashMap<&str, usize> = runs
        .iter()
        .enumerate()
        .map(|(i, r)| (r.num.as_str(), i))
        .collect();

    let mut out: Vec<ChargeRun> = runs
        .iter()
        .map(|r| ChargeRun {
            num: r.num.clone(),
            date: r.date,
            premieres: 0,
            recurrences: 0,
        })
        .collect();

    for l in lignes {
        // Un compte placé sur un run non retenu est ignoré, pas replié sur un
        // autre run : le replier inventerait une facture.
        if let Some(&depart) = index_par_num.get(l.run_num.as_str()) {
            out[depart].premieres += 1;
        }
    }
    out
}
```

Retirer la ligne `let _ = lignes;`.

- [ ] **Étape 4 : lancer les tests et vérifier qu'ils passent**

```bash
cd client/src-tauri && cargo test charge::
```

Attendu : 3 tests `ok`.

- [ ] **Étape 5 : commit**

```bash
git add client/src-tauri/src/charge.rs
git commit -m "feat(superpopaul): premières factures par run"
```

---

## Tâche 3 : `charge.rs` — récurrences, une par mois civil

C'est le cœur du lot. Le test 1 ci-dessous est celui qui distingue la règle
retenue (une facture par mois) de la lecture littérale écartée (une facture par
run couvrant le jour de cycle).

**Fichiers :**
- Modifier : `client/src-tauri/src/charge.rs`

- [ ] **Étape 1 : écrire les tests qui échouent**

Ajouter dans `mod tests` :

```rust
#[test]
fn un_compte_ne_facture_quune_fois_par_mois() {
    // Deux runs de SEPTEMBRE couvrent le jour de cycle 5. Le compte, démarré
    // en août, ne doit facturer qu'UNE fois en septembre — au premier des deux.
    let runs = vec![
        run("R1", "2026-08-11", &[5]),
        run("R2", "2026-09-08", &[5]),
        run("R3", "2026-09-22", &[5]),
    ];
    let lignes = vec![ligne("CF1", 5, "R1")];
    let c = charge(&lignes, &runs);
    assert_eq!(c[0].premieres, 1);
    assert_eq!(c[1].recurrences, 1, "le premier run de septembre porte la facture");
    assert_eq!(c[2].recurrences, 0, "le second run du mois ne refacture pas");
}

#[test]
fn mois_sans_run_couvrant_le_jj_ne_facture_pas() {
    // Septembre ne couvre que le jour 15 : le compte de jour 5 saute ce mois.
    // Pas de report silencieux sur octobre : le trou est le fait du calendrier.
    let runs = vec![
        run("R1", "2026-08-11", &[5]),
        run("R2", "2026-09-08", &[15]),
        run("R3", "2026-10-06", &[5]),
    ];
    let lignes = vec![ligne("CF1", 5, "R1")];
    let c = charge(&lignes, &runs);
    assert_eq!(c[1].recurrences, 0, "aucun run de septembre ne couvre le jour 5");
    assert_eq!(c[2].recurrences, 1, "octobre reprend, sans rattraper septembre");
}

#[test]
fn les_runs_sans_premiere_facture_portent_les_recurrences() {
    // Régime de croisière : après la dernière MEP, les runs ne démarrent plus
    // personne mais facturent tout le parc. Ils ne doivent pas disparaître.
    let runs = vec![
        run("R1", "2026-08-11", &[5]),
        run("R2", "2026-09-08", &[5]),
    ];
    let lignes = vec![ligne("CF1", 5, "R1"), ligne("CF2", 5, "R1")];
    let c = charge(&lignes, &runs);
    assert_eq!(c[1].premieres, 0);
    assert_eq!(c[1].recurrences, 2);
    assert_eq!(c[1].total(), 2);
}

#[test]
fn un_compte_place_hors_porteur_recurre_quand_meme() {
    // Le compte démarre au SECOND run de septembre (le porteur du mois est le
    // premier). Il ne doit pas être perdu pour les mois suivants.
    let runs = vec![
        run("R1", "2026-09-08", &[5]),
        run("R2", "2026-09-22", &[5]),
        run("R3", "2026-10-06", &[5]),
    ];
    let lignes = vec![ligne("CF1", 5, "R2")];
    let c = charge(&lignes, &runs);
    assert_eq!(c[1].premieres, 1, "il démarre bien à son run");
    assert_eq!(c[0].recurrences, 0, "le porteur de septembre lui est antérieur");
    assert_eq!(c[2].recurrences, 1, "octobre le reprend");
}
```

- [ ] **Étape 2 : lancer les tests et vérifier qu'ils échouent**

```bash
cd client/src-tauri && cargo test charge::
```

Attendu : les 4 nouveaux échouent, tous sur `recurrences` valant `0`.

- [ ] **Étape 3 : implémenter les récurrences**

Dans `charge`, insérer après la construction de `index_par_num` :

```rust
    // Porteur du mois : pour chaque jour de cycle, les index des runs qui
    // portent sa facture mensuelle — le PREMIER run de chaque mois civil qui
    // couvre ce jour. Deux runs du même mois couvrant le même jour ne
    // facturent donc pas deux fois.
    let mut vu: HashSet<(i32, u32, u8)> = HashSet::new();
    let mut porteurs_par_jj: HashMap<u8, Vec<usize>> = HashMap::new();
    for (i, r) in runs.iter().enumerate() {
        for &jj in &r.jjs {
            if vu.insert((r.date.year(), r.date.month(), jj)) {
                porteurs_par_jj.entry(jj).or_default().push(i);
            }
        }
    }
```

Puis remplacer la boucle de comptage :

```rust
    for l in lignes {
        let Some(&depart) = index_par_num.get(l.run_num.as_str()) else {
            continue;
        };
        out[depart].premieres += 1;
        // Strictement APRÈS le départ : le mois du démarrage, la facture est
        // déjà comptée comme première.
        if let Some(porteurs) = porteurs_par_jj.get(&l.jj) {
            for &i in porteurs.iter().filter(|&&i| i > depart) {
                out[i].recurrences += 1;
            }
        }
    }
```

- [ ] **Étape 4 : lancer les tests et vérifier qu'ils passent**

```bash
cd client/src-tauri && cargo test charge::
```

Attendu : 7 tests `ok`.

- [ ] **Étape 5 : vérifier la suite complète**

```bash
cd client/src-tauri && cargo test 2>&1 | tail -5
```

Attendu : `test result: ok. 464 passed` (457 + 7).

- [ ] **Étape 6 : commit**

```bash
git add client/src-tauri/src/charge.rs
git commit -m "feat(superpopaul): récurrences mensuelles dans la charge par run"
```

---

## Tâche 4 : `charts.rs` — échelle des graduations

**Fichiers :**
- Créer : `client/src-tauri/src/charts.rs`
- Modifier : `client/src-tauri/src/lib.rs`

- [ ] **Étape 1 : déclarer le module**

Dans `lib.rs`, après `pub mod charge;` :

```rust
pub mod charts;
```

- [ ] **Étape 2 : écrire les tests qui échouent**

Créer `client/src-tauri/src/charts.rs` :

```rust
//! Graphes SVG du rapport — aire cumulée et barres empilées.
//!
//! Module PUR : il reçoit des nombres et des étiquettes, il rend une chaîne
//! SVG. Il ne connaît ni plan, ni run, ni facture. Aucun JS, aucune dépendance
//! graphique : les couleurs viennent des variables CSS déjà définies dans la
//! constante `CSS` de `report`, qui s'appliquent au SVG inline.
//!
//! Les étiquettes proviennent de fichiers fournis par des tiers (numéros de run
//! du `runs.csv`) : elles passent toutes par `esc`.

use crate::report::{esc, fmt_int};
use chrono::NaiveDate;

/// Borne haute « ronde » d'un axe et pas de ses graduations, visant au plus
/// quatre intervalles. Un maximum nul rend `(1, 1)` : jamais de division par
/// zéro, jamais d'axe sans graduation.
fn echelle(max: u64) -> (u64, u64) {
    if max == 0 {
        return (1, 1);
    }
    let mut p = 1u64;
    loop {
        for mult in [1u64, 2, 5] {
            let pas = mult * p;
            if pas.saturating_mul(4) >= max {
                return (((max + pas - 1) / pas) * pas, pas);
            }
        }
        match p.checked_mul(10) {
            Some(n) => p = n,
            None => return (max, max),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn echelle_arrondie_au_palier_superieur() {
        assert_eq!(echelle(1800), (2000, 500), "0 500 1000 1500 2000");
        assert_eq!(echelle(3340), (4000, 1000), "0 1000 2000 3000 4000");
        assert_eq!(echelle(7), (8, 2));
    }

    #[test]
    fn echelle_dun_maximum_nul_ne_divise_pas_par_zero() {
        assert_eq!(echelle(0), (1, 1));
    }
}
```

- [ ] **Étape 3 : lancer les tests et vérifier qu'ils passent**

```bash
cd client/src-tauri && cargo test charts::
```

Attendu : 2 tests `ok`. (Ici le code accompagne le test dès l'écriture :
`echelle` est une fonction arithmétique pure dont les cas sont entièrement
décrits par les assertions. Si l'un échoue, corriger `echelle`, pas le test.)

- [ ] **Étape 4 : commit**

```bash
git add client/src-tauri/src/charts.rs client/src-tauri/src/lib.rs
git commit -m "feat(superpopaul): échelle des graduations des graphes"
```

---

## Tâche 5 : `charts.rs` — barres empilées

**Fichiers :**
- Modifier : `client/src-tauri/src/charts.rs`

- [ ] **Étape 1 : écrire les tests qui échouent**

Ajouter dans `mod tests` :

```rust
fn barre(label: &str, sous: &str, bas: u64, haut: u64) -> Barre {
    Barre { label: label.into(), sous_label: sous.into(), bas, haut }
}

#[test]
fn barres_empilees_rend_une_barre_par_serie() {
    let b = vec![barre("R1", "11/08", 420, 0), barre("R2", "08/09", 610, 420)];
    let svg = barres_empilees(&b);
    assert!(svg.starts_with("<svg"), "{svg}");
    assert!(svg.ends_with("</svg>"));
    assert_eq!(svg.matches("class=\"b-first\"").count(), 2);
    // La barre sans récurrence ne doit pas produire de rectangle de hauteur nulle.
    assert_eq!(svg.matches("class=\"b-rec\"").count(), 1);
    assert!(svg.contains(">R1<") && svg.contains(">11/08<"));
}

#[test]
fn barres_empilees_marque_le_pic() {
    // Le repère relie l'indicateur de tête à la figure (décision 12).
    let b = vec![barre("R1", "11/08", 420, 0), barre("R2", "08/09", 610, 420)];
    let svg = barres_empilees(&b);
    assert!(svg.contains("class=\"b-peak\""), "{svg}");
    assert!(svg.contains("pic 1 030"), "le pic vaut 610 + 420, séparateur de milliers compris");
}

#[test]
fn barres_empilees_serie_vide_rend_un_svg_valide() {
    let svg = barres_empilees(&[]);
    assert!(svg.starts_with("<svg") && svg.ends_with("</svg>"), "{svg}");
    assert!(svg.contains("Aucun run"), "l'absence doit se dire, pas se taire");
}

#[test]
fn barres_empilees_valeurs_toutes_nulles_ne_divisent_pas_par_zero() {
    let svg = barres_empilees(&[barre("R1", "11/08", 0, 0)]);
    assert!(svg.starts_with("<svg"));
}

#[test]
fn les_etiquettes_de_run_sont_echappees() {
    // Le numéro de run vient du runs.csv : entrée non fiable.
    let svg = barres_empilees(&[barre("<script>alert(1)</script>", "11/08", 1, 0)]);
    assert!(!svg.contains("<script>alert"), "injection non échappée : {svg}");
    assert!(svg.contains("&lt;script&gt;"));
}
```

- [ ] **Étape 2 : lancer les tests et vérifier qu'ils échouent**

```bash
cd client/src-tauri && cargo test charts::
```

Attendu : **échec de compilation**, `cannot find type 'Barre'`.

- [ ] **Étape 3 : implémenter**

Ajouter dans `charts.rs`, avant `#[cfg(test)]` :

```rust
/// Une barre du graphe de charge : `bas` empilé sous `haut`.
pub struct Barre {
    pub label: String,
    pub sous_label: String,
    pub bas: u64,
    pub haut: u64,
}

const W: f64 = 760.0;
const X0: f64 = 52.0;
const X1: f64 = 748.0;
const Y0: f64 = 16.0;
const Y1: f64 = 194.0;

/// Barres empilées : `bas` (premières factures) sous `haut` (récurrences).
pub fn barres_empilees(barres: &[Barre]) -> String {
    let mut s = String::with_capacity(4 * 1024);
    s.push_str(&format!(
        "<svg viewBox=\"0 0 {W} 240\" role=\"img\" aria-label=\"Charge par run\">"
    ));
    if barres.is_empty() {
        s.push_str(&format!(
            "<text class=\"tick mid\" x=\"{}\" y=\"120\">Aucun run retenu</text></svg>",
            W / 2.0
        ));
        return s;
    }

    let max = barres.iter().map(|b| b.bas + b.haut).max().unwrap_or(0);
    let (haut, pas) = echelle(max);
    let y = |v: u64| Y1 - (v as f64 / haut as f64) * (Y1 - Y0);

    let mut v = 0u64;
    while v <= haut {
        s.push_str(&format!(
            "<line class=\"grid\" x1=\"{X0}\" y1=\"{0:.1}\" x2=\"{X1}\" y2=\"{0:.1}\"></line>\
             <text class=\"tick end\" x=\"44\" y=\"{1:.1}\">{2}</text>",
            y(v),
            y(v) + 4.0,
            fmt_int(v)
        ));
        v += pas;
    }

    let bande = (X1 - X0) / barres.len() as f64;
    let largeur = (bande * 0.54).min(46.0);
    for (i, b) in barres.iter().enumerate() {
        let centre = X0 + bande * (i as f64 + 0.5);
        let x = centre - largeur / 2.0;
        if b.bas > 0 {
            s.push_str(&format!(
                "<rect class=\"b-first\" x=\"{x:.1}\" y=\"{:.1}\" width=\"{largeur:.1}\" height=\"{:.1}\"></rect>",
                y(b.bas),
                Y1 - y(b.bas)
            ));
        }
        if b.haut > 0 {
            s.push_str(&format!(
                "<rect class=\"b-rec\" x=\"{x:.1}\" y=\"{:.1}\" width=\"{largeur:.1}\" height=\"{:.1}\"></rect>",
                y(b.bas + b.haut),
                y(b.bas) - y(b.bas + b.haut)
            ));
        }
        s.push_str(&format!(
            "<text class=\"tick mid\" x=\"{centre:.1}\" y=\"212\">{}</text>\
             <text class=\"tick mid\" x=\"{centre:.1}\" y=\"226\">{}</text>",
            esc(&b.label),
            esc(&b.sous_label)
        ));
    }

    // Repère du pic : la barre la plus haute est celle qui dimensionne.
    if max > 0 {
        s.push_str(&format!(
            "<line class=\"b-peak\" x1=\"{X0}\" y1=\"{0:.1}\" x2=\"{X1}\" y2=\"{0:.1}\"></line>\
             <text class=\"tick end\" x=\"{X1}\" y=\"{1:.1}\">pic {2}</text>",
            y(max),
            y(max) - 5.0,
            fmt_int(max)
        ));
    }

    s.push_str(&format!(
        "<line class=\"axis\" x1=\"{X0}\" y1=\"{Y1}\" x2=\"{X1}\" y2=\"{Y1}\"></line></svg>"
    ));
    s
}
```

- [ ] **Étape 4 : lancer les tests et vérifier qu'ils passent**

```bash
cd client/src-tauri && cargo test charts::
```

Attendu : 7 tests `ok`.

- [ ] **Étape 5 : commit**

```bash
git add client/src-tauri/src/charts.rs
git commit -m "feat(superpopaul): graphe en barres empilées de la charge par run"
```

---

## Tâche 6 : `charts.rs` — aire cumulée en escalier

**Fichiers :**
- Modifier : `client/src-tauri/src/charts.rs`

- [ ] **Étape 1 : écrire les tests qui échouent**

Ajouter dans `mod tests` :

```rust
fn j(iso: &str) -> NaiveDate {
    NaiveDate::parse_from_str(iso, "%Y-%m-%d").unwrap()
}

#[test]
fn aire_cumulee_trace_un_escalier_et_ses_jalons() {
    let pts = vec![
        Point { date: j("2026-08-11"), valeur: 420 },
        Point { date: j("2026-09-08"), valeur: 1030 },
    ];
    let jalons = vec![JalonChart { date: j("2026-08-01"), label: "MEP 1".into() }];
    let svg = aire_cumulee(&pts, &jalons, j("2026-08-01"), j("2026-09-30"));
    assert!(svg.starts_with("<svg") && svg.ends_with("</svg>"));
    assert!(svg.contains("class=\"area\"") && svg.contains("class=\"line\""));
    // Escalier : le tracé n'utilise que des segments horizontaux et verticaux.
    assert!(svg.contains(" H ") && svg.contains(" V "), "{svg}");
    assert!(!svg.contains(" C "), "aucune courbe de Bézier : le parc saute, il ne glisse pas");
    assert!(svg.contains("MEP 1"));
}

#[test]
fn aire_cumulee_sans_point_rend_un_svg_valide() {
    let svg = aire_cumulee(&[], &[], j("2026-08-01"), j("2026-09-30"));
    assert!(svg.starts_with("<svg") && svg.ends_with("</svg>"), "{svg}");
    assert!(svg.contains("Aucune"));
}

#[test]
fn aire_cumulee_fenetre_dun_seul_jour_ne_divise_pas_par_zero() {
    let pts = vec![Point { date: j("2026-08-01"), valeur: 5 }];
    let svg = aire_cumulee(&pts, &[], j("2026-08-01"), j("2026-08-01"));
    assert!(svg.starts_with("<svg"));
}

#[test]
fn les_libelles_de_jalon_sont_echappes() {
    let jalons = vec![JalonChart { date: j("2026-08-01"), label: "<script>x</script>".into() }];
    let pts = vec![Point { date: j("2026-08-11"), valeur: 1 }];
    let svg = aire_cumulee(&pts, &jalons, j("2026-08-01"), j("2026-09-30"));
    assert!(!svg.contains("<script>x"), "{svg}");
    assert!(svg.contains("&lt;script&gt;"));
}
```

- [ ] **Étape 2 : lancer les tests et vérifier qu'ils échouent**

```bash
cd client/src-tauri && cargo test charts::
```

Attendu : **échec de compilation**, `cannot find type 'Point'`.

- [ ] **Étape 3 : implémenter**

Ajouter dans `charts.rs`, avant `#[cfg(test)]` :

```rust
/// Un palier de la courbe cumulée : à cette date, le cumul vaut cette valeur.
pub struct Point {
    pub date: NaiveDate,
    pub valeur: u64,
}

/// Un repère vertical daté (mise en production).
pub struct JalonChart {
    pub date: NaiveDate,
    pub label: String,
}

/// Aire cumulée **en escalier** : le parc facturant saute à chaque run, il ne
/// croît pas continûment. Une courbe lissée suggérerait une progression
/// quotidienne qui n'existe pas.
pub fn aire_cumulee(
    points: &[Point],
    jalons: &[JalonChart],
    debut: NaiveDate,
    fin: NaiveDate,
) -> String {
    let mut s = String::with_capacity(4 * 1024);
    s.push_str(&format!(
        "<svg viewBox=\"0 0 {W} 240\" role=\"img\" aria-label=\"Parc facturant cumulé\">"
    ));
    if points.is_empty() {
        s.push_str(&format!(
            "<text class=\"tick mid\" x=\"{}\" y=\"120\">Aucune première facture planifiée</text></svg>",
            W / 2.0
        ));
        return s;
    }

    let jours = (fin - debut).num_days().max(1) as f64;
    let x = |d: NaiveDate| X0 + ((d - debut).num_days() as f64 / jours) * (X1 - X0);
    let max = points.iter().map(|p| p.valeur).max().unwrap_or(0);
    let (haut, pas) = echelle(max);
    let y = |v: u64| Y1 + 6.0 - (v as f64 / haut as f64) * (Y1 + 6.0 - Y0);

    let mut v = 0u64;
    while v <= haut {
        s.push_str(&format!(
            "<line class=\"grid\" x1=\"{X0}\" y1=\"{0:.1}\" x2=\"{X1}\" y2=\"{0:.1}\"></line>\
             <text class=\"tick end\" x=\"44\" y=\"{1:.1}\">{2}</text>",
            y(v),
            y(v) + 4.0,
            fmt_int(v)
        ));
        v += pas;
    }

    for jl in jalons {
        let jx = x(jl.date);
        s.push_str(&format!(
            "<line class=\"mep\" x1=\"{jx:.1}\" y1=\"{Y0}\" x2=\"{jx:.1}\" y2=\"{:.1}\"></line>\
             <text class=\"mep-lbl\" x=\"{:.1}\" y=\"27\">{}</text>",
            y(0),
            jx + 22.0,
            esc(&jl.label)
        ));
    }

    // Escalier : on avance à l'horizontale jusqu'à la date du run, puis on
    // monte d'un coup (H puis V, jamais de segment oblique).
    let base = y(0);
    let mut d = format!("M {X0:.1},{base:.1}");
    for p in points {
        d.push_str(&format!(" H {:.1} V {:.1}", x(p.date), y(p.valeur)));
    }
    d.push_str(&format!(" H {X1:.1}"));
    s.push_str(&format!("<path class=\"area\" d=\"{d} V {base:.1} Z\"></path>"));
    s.push_str(&format!("<path class=\"line\" d=\"{d}\"></path>"));

    s.push_str(&format!(
        "<line class=\"axis\" x1=\"{X0}\" y1=\"{base:.1}\" x2=\"{X1}\" y2=\"{base:.1}\"></line></svg>"
    ));
    s
}
```

- [ ] **Étape 4 : lancer les tests et vérifier qu'ils passent**

```bash
cd client/src-tauri && cargo test charts::
```

Attendu : 10 tests `ok`.

- [ ] **Étape 5 : vérifier la suite complète et clippy**

```bash
cd client/src-tauri && cargo test 2>&1 | tail -5 && cargo clippy --all-targets 2>&1 | grep -c "^warning"
```

Attendu : `475 passed` (457 + 7 `charge` + 11 `charts`), et **5** warnings clippy — les
préexistants. Tout warning supplémentaire est à corriger avant de continuer.

- [ ] **Étape 6 : commit**

```bash
git add client/src-tauri/src/charts.rs
git commit -m "feat(superpopaul): courbe en escalier du parc facturant"
```

---

## Tâche 7 : CSS — brancher la feuille de style manquante

Aucune règle existante n'est modifiée : le rapport de run ne doit pas bouger
d'un pixel. On ajoute uniquement.

**Fichiers :**
- Modifier : `client/src-tauri/src/report.rs` (constante `CSS`, ligne 47)

- [ ] **Étape 1 : écrire le test qui échoue**

Dans `report.rs`, `mod tests`, ajouter :

```rust
#[test]
fn le_css_style_les_classes_du_rapport_de_plan() {
    // Régression : ces classes étaient émises par plan_report sans qu'aucune
    // règle ne les définisse — le rapport s'affichait en HTML brut.
    for sel in [
        ".kpis.sub", ".warn", ".chart", ".b-first", ".b-rec",
        "thead th", "tbody td", ".tbl", ".dist-row", ".dist-gap",
    ] {
        assert!(CSS.contains(sel), "règle manquante pour « {sel} »");
    }
}
```

- [ ] **Étape 2 : lancer le test et vérifier qu'il échoue**

```bash
cd client/src-tauri && cargo test le_css_style_les_classes
```

Attendu : `règle manquante pour « .kpis.sub »`.

- [ ] **Étape 3 : ajouter les règles**

Copier le bloc CSS de la maquette validée. Il est délimité dans
`docs/superpowers/maquettes/2026-07-26-rapport-plan.html` par le commentaire
`AJOUTS DU CHANTIER « rapport de plan »` et court jusqu'à la fin de la balise
`<style>`. L'insérer **à la fin** de la constante `CSS` de `report.rs`, juste
avant le `"#;` de fermeture, en conservant l'indentation à deux espaces des
règles voisines.

Vérifier après collage qu'aucune accolade n'est déséquilibrée :

```bash
cd client/src-tauri && cargo test le_css_style_les_classes
```

- [ ] **Étape 4 : vérifier que le rapport de run n'a pas bougé**

```bash
cd client/src-tauri && cargo test report::
```

Attendu : tous verts, aucun test du rapport de run modifié.

- [ ] **Étape 5 : commit**

```bash
git add client/src-tauri/src/report.rs
git commit -m "feat(superpopaul): styles du rapport de plan dans le CSS partagé"
```

---

## Tâche 8 : `plan_report.rs` — avertissements dérivés

Le site d'appel passe aujourd'hui `avertissements: &[]` : la section est morte.
On la fait vivre à partir de ce que le rapport a déjà sous la main.

**Fichiers :**
- Modifier : `client/src-tauri/src/plan_report.rs`

- [ ] **Étape 1 : écrire les tests qui échouent**

Ajouter dans `mod tests` de `plan_report.rs` :

```rust
fn runs_test() -> Vec<crate::calendrier::RunFacturation> {
    vec![crate::calendrier::RunFacturation {
        num: "R1".into(),
        date: jour("2026-08-11"),
        jjs: vec![5],
        exclu: false,
    }]
}

#[test]
fn avertit_sur_une_plateforme_du_pool_sans_compte_planifie() {
    let lignes = vec![ligne("CF1", "Cegedim", "2026-08-01", Origine::Auto)];
    let pool = BTreeMap::from([("Cegedim".to_string(), 10usize), ("Freedz".to_string(), 4)]);
    let jj = BTreeMap::from([(5u8, 14usize)]);
    let html = render(&data(&lignes, &pool, &jj, &runs_test()));
    assert!(html.contains("Freedz"), "la plateforme non servie doit être nommée");
    assert!(!html.contains(">Cegedim</b> : aucun compte"), "{html}");
}

#[test]
fn avertit_sur_un_jour_de_cycle_hors_datteinte() {
    let lignes = vec![ligne("CF1", "Cegedim", "2026-08-01", Origine::Auto)];
    let pool = BTreeMap::from([("Cegedim".to_string(), 10usize)]);
    // Le jour 12 pèse 30 comptes mais aucun run retenu ne le couvre.
    let jj = BTreeMap::from([(5u8, 14usize), (12u8, 30usize)]);
    let html = render(&data(&lignes, &pool, &jj, &runs_test()));
    assert!(html.contains("12"), "le jour de cycle orphelin doit être nommé");
    assert!(html.contains("30"), "son effectif aussi : c'est ce qui rend l'alerte actionnable");
}

#[test]
fn aucun_avertissement_quand_le_plan_couvre_tout() {
    let lignes = vec![ligne("CF1", "Cegedim", "2026-08-01", Origine::Auto)];
    let pool = BTreeMap::from([("Cegedim".to_string(), 10usize)]);
    let jj = BTreeMap::from([(5u8, 14usize)]);
    let html = render(&data(&lignes, &pool, &jj, &runs_test()));
    assert!(!html.contains("Avertissements"), "pas de section vide : {html}");
}

#[test]
fn les_avertissements_derives_sont_echappes() {
    let lignes = vec![ligne("CF1", "Cegedim", "2026-08-01", Origine::Auto)];
    let pool = BTreeMap::from([
        ("Cegedim".to_string(), 10usize),
        ("<script>alert(1)</script>".to_string(), 4),
    ]);
    let jj = BTreeMap::from([(5u8, 14usize)]);
    let html = render(&data(&lignes, &pool, &jj, &runs_test()));
    assert!(!html.contains("<script>alert"), "injection non échappée");
    assert!(html.contains("&lt;script&gt;"));
}
```

- [ ] **Étape 2 : adapter la fabrique `data` des tests**

Remplacer la fonction `data` existante (elle prend `warns`, qui disparaît) :

```rust
fn data<'a>(
    lignes: &'a [LignePlan],
    pool: &'a BTreeMap<String, usize>,
    pool_par_jj: &'a BTreeMap<u8, usize>,
    runs: &'a [crate::calendrier::RunFacturation],
) -> PlanReportData<'a> {
    PlanReportData {
        fichier: "clients.csv",
        date_longue: "25 juillet 2026",
        version: "1.1.0",
        lignes,
        aujourdhui: jour("2026-07-25"),
        pool_par_pa: pool,
        pool_par_jj,
        runs,
    }
}
```

Adapter les appels existants de `data(...)` dans les tests déjà présents : leur
passer `&BTreeMap::new()` et `&runs_test()` en plus. Supprimer le test
`rapport_echappe_aussi_les_avertissements`, remplacé par
`les_avertissements_derives_sont_echappes`.

- [ ] **Étape 3 : lancer les tests et vérifier qu'ils échouent**

```bash
cd client/src-tauri && cargo test plan_report::
```

Attendu : **échec de compilation**, `struct PlanReportData has no field named pool_par_jj`.

- [ ] **Étape 4 : implémenter**

Dans `plan_report.rs`, modifier `PlanReportData` — retirer `avertissements`,
ajouter les deux champs :

```rust
pub struct PlanReportData<'a> {
    pub fichier: &'a str,
    pub date_longue: &'a str,
    pub version: &'a str,
    pub lignes: &'a [LignePlan],
    pub aujourdhui: chrono::NaiveDate,
    /// Pool éligible par plateforme, pour la comparaison plan vs pool.
    pub pool_par_pa: &'a BTreeMap<String, usize>,
    /// Pool éligible par jour de cycle, pour détecter les comptes hors d'atteinte.
    pub pool_par_jj: &'a BTreeMap<u8, usize>,
    /// Runs **retenus** du calendrier.
    pub runs: &'a [crate::calendrier::RunFacturation],
}
```

Ajouter la fonction, avant `render` :

```rust
/// Avertissements que le rapport déduit de son propre contenu.
///
/// Les avertissements de l'allocation ne sont pas persistés dans `PlanMeta` :
/// on ne peut pas les restituer. Ceux-ci portent la même information utile et
/// se recalculent sur des données fraîches, ce que le rapport fait déjà pour
/// le pool.
fn avertissements_derives(
    actifs: &[&LignePlan],
    pool_par_pa: &BTreeMap<String, usize>,
    pool_par_jj: &BTreeMap<u8, usize>,
    runs: &[crate::calendrier::RunFacturation],
) -> Vec<String> {
    let mut out = Vec::new();

    let servies: std::collections::HashSet<&str> =
        actifs.iter().map(|l| l.pa.as_str()).collect();
    for (pa, n) in pool_par_pa {
        if *n > 0 && !servies.contains(pa.as_str()) {
            out.push(format!(
                "plateforme « {pa} » : aucun compte planifié, alors que {n} comptes \
                 du pool lui appartiennent"
            ));
        }
    }

    let couverts: std::collections::HashSet<u8> =
        runs.iter().flat_map(|r| r.jjs.iter().copied()).collect();
    for (jj, n) in pool_par_jj {
        if *n > 0 && !couverts.contains(jj) {
            out.push(format!(
                "jour de cycle {jj} : {n} comptes hors d'atteinte — aucun run retenu \
                 ne le couvre"
            ));
        }
    }
    out
}
```

Dans `render`, remplacer le bloc `if !d.avertissements.is_empty()` par :

```rust
    let avertissements = avertissements_derives(&actifs, d.pool_par_pa, d.pool_par_jj, d.runs);
    if !avertissements.is_empty() {
        html.push_str("<section class=\"warn\">\n<h2>Avertissements</h2>\n<ul>\n");
        for a in &avertissements {
            html.push_str(&format!("<li>{}</li>\n", esc(a)));
        }
        html.push_str("</ul>\n</section>\n");
    }
```

- [ ] **Étape 5 : lancer les tests et vérifier qu'ils passent**

```bash
cd client/src-tauri && cargo test plan_report::
```

Attendu : tous verts. La compilation de `commands.rs` échoue encore (champ
`avertissements` disparu) — c'est attendu, la tâche 10 la répare. Pour isoler,
utiliser `cargo test --lib plan_report::` si nécessaire.

- [ ] **Étape 6 : commit**

```bash
git add client/src-tauri/src/plan_report.rs
git commit -m "feat(superpopaul): avertissements dérivés du rapport de plan"
```

---

## Tâche 9 : `plan_report.rs` — indicateurs, graphes et mise en page

**Fichiers :**
- Modifier : `client/src-tauri/src/plan_report.rs`
- Référence visuelle : `docs/superpowers/maquettes/2026-07-26-rapport-plan.html`

- [ ] **Étape 1 : écrire les tests qui échouent**

Ajouter dans `mod tests` :

```rust
#[test]
fn les_indicateurs_de_trajectoire_sont_calcules() {
    let mut l2 = ligne("CF2", "Cegedim", "2026-08-01", Origine::Auto);
    l2.jj = 5;
    let lignes = vec![ligne("CF1", "Cegedim", "2026-08-01", Origine::Auto), l2];
    let pool = BTreeMap::from([("Cegedim".to_string(), 8usize)]);
    let jj = BTreeMap::from([(5u8, 8usize)]);
    let html = render(&data(&lignes, &pool, &jj, &runs_test()));
    assert!(html.contains("comptes planifiés"));
    assert!(html.contains("sur <b>8</b> éligibles"), "l'échelle du pool : {html}");
    assert!(html.contains("fin de montée en charge"));
    assert!(html.contains("pic de charge"));
    assert!(html.contains("plateformes couvertes"));
}

#[test]
fn le_rapport_contient_les_deux_graphes() {
    let lignes = vec![ligne("CF1", "Cegedim", "2026-08-01", Origine::Auto)];
    let pool = BTreeMap::from([("Cegedim".to_string(), 8usize)]);
    let jj = BTreeMap::from([(5u8, 8usize)]);
    let html = render(&data(&lignes, &pool, &jj, &runs_test()));
    assert_eq!(html.matches("<svg").count(), 2, "aire cumulée + barres");
    assert!(html.contains("Parc facturant"));
    assert!(html.contains("Charge par run"));
}

#[test]
fn le_rapport_nemet_plus_de_classes_orphelines() {
    // Régression du constat d'ouverture : ces classes n'existaient dans aucune
    // feuille de style, le rapport s'affichait en HTML brut.
    let lignes = vec![ligne("CF1", "Cegedim", "2026-08-01", Origine::Auto)];
    let pool = BTreeMap::from([("Cegedim".to_string(), 8usize)]);
    let jj = BTreeMap::from([(5u8, 8usize)]);
    let html = render(&data(&lignes, &pool, &jj, &runs_test()));
    for orpheline in ["class=\"cards\"", "class=\"card\"", "class=\"big\"", "class=\"lbl\""] {
        assert!(!html.contains(orpheline), "classe sans style : {orpheline}");
    }
    assert!(html.contains("class=\"kpis\""), "les cartes du rapport de run");
    assert!(html.contains("class=\"tbl\""), "la table doit être encadrée");
}
```

- [ ] **Étape 2 : lancer les tests et vérifier qu'ils échouent**

```bash
cd client/src-tauri && cargo test --lib plan_report::
```

Attendu : `l'échelle du pool` échoue, puis `assertion failed: 0 == 2` sur les SVG.

- [ ] **Étape 3 : implémenter les indicateurs**

Dans `render`, après le calcul de `actifs`, ajouter :

```rust
    let charge = crate::charge::charge(
        &actifs.iter().map(|l| (*l).clone()).collect::<Vec<_>>(),
        d.runs,
    );
    let pool_total: usize = d.pool_par_pa.values().sum();
    let servies = actifs.iter().map(|l| l.pa.as_str()).collect::<std::collections::HashSet<_>>();
    let pa_du_pool = d.pool_par_pa.iter().filter(|(_, n)| **n > 0).count();
    let pic = charge.iter().max_by_key(|c| c.total());
    // Fin de montée en charge : dernier run portant une PREMIÈRE facture, non
    // dernier run de la série — sinon l'indicateur mesurerait la longueur du
    // runs.csv, pas celle du déploiement.
    let fin = charge.iter().rev().find(|c| c.premieres > 0);
```

Remplacer le bloc `<section class="cards">` par la bande de trajectoire, en
suivant la maquette (`.kpis` / `.kpi` / `.v` / `.l` / `.abs` / `.unit`) :

```rust
    html.push_str("<section class=\"kpis\">\n");
    html.push_str(&format!(
        "<div class=\"kpi gold\"><div class=\"v\">{}</div>\
         <div class=\"l\">comptes planifiés</div>\
         <div class=\"abs\">sur <b>{}</b> éligibles · {}</div></div>\n",
        fmt_int(total),
        fmt_int(pool_total as u64),
        pourcent(total, pool_total as u64)
    ));
    if let Some(c) = fin {
        html.push_str(&format!(
            "<div class=\"kpi\"><div class=\"v\">{}</div>\
             <div class=\"l\">fin de montée en charge</div>\
             <div class=\"abs\">dernier run portant un démarrage</div></div>\n",
            c.date.format("%d/%m/%Y")
        ));
    }
    if let Some(p) = pic {
        html.push_str(&format!(
            "<div class=\"kpi amber\"><div class=\"v\">{}</div>\
             <div class=\"l\">pic de charge (un run)</div>\
             <div class=\"abs\">run <b>{}</b> le <b>{}</b></div></div>\n",
            fmt_int(p.total() as u64),
            esc(&p.num),
            p.date.format("%d/%m/%Y")
        ));
    }
    html.push_str(&format!(
        "<div class=\"kpi\"><div class=\"v\">{} <span class=\"unit\">/ {}</span></div>\
         <div class=\"l\">plateformes couvertes</div></div>\n",
        servies.len(),
        pa_du_pool
    ));
    html.push_str("</section>\n");
```

- [ ] **Étape 4 : implémenter les deux graphes**

Après la section des avertissements, insérer :

```rust
    // ③ Parc facturant — cumul en escalier.
    let mut cumul = 0u64;
    let points: Vec<crate::charts::Point> = charge
        .iter()
        .filter(|c| c.premieres > 0)
        .map(|c| {
            cumul += c.premieres as u64;
            crate::charts::Point { date: c.date, valeur: cumul }
        })
        .collect();
    // `mep_id` est déjà 1-basé : le réindexer ferait diverger le libellé du
    // graphe de celui de la table, qui affiche `mep_id` tel quel.
    let jalons: Vec<crate::charts::JalonChart> = meps
        .iter()
        .map(|(id, date)| crate::charts::JalonChart {
            date: chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").unwrap_or(d.aujourdhui),
            label: format!("MEP {id}"),
        })
        .collect();
    let debut = jalons.iter().map(|j| j.date).min()
        .into_iter()
        .chain(points.first().map(|p| p.date))
        .min()
        .unwrap_or(d.aujourdhui);
    let fin_axe = charge.last().map(|c| c.date).unwrap_or(debut);
    html.push_str("<h2>Parc facturant</h2>\n<p class=\"h2sub\">Nombre de comptes ayant émis \
        leur première facture, cumulé au fil des runs. Les jalons marquent les mises en \
        production.</p>\n<div class=\"chart\">\n");
    html.push_str(&crate::charts::aire_cumulee(&points, &jalons, debut, fin_axe));
    html.push_str("\n</div>\n");

    // ④ Charge par run — barres empilées.
    let barres: Vec<crate::charts::Barre> = charge
        .iter()
        .map(|c| crate::charts::Barre {
            label: c.num.clone(),
            sous_label: c.date.format("%d/%m").to_string(),
            bas: c.premieres as u64,
            haut: c.recurrences as u64,
        })
        .collect();
    html.push_str("<h2>Charge par run</h2>\n<p class=\"h2sub\">Factures émises à chaque run : \
        premières factures des comptes qui démarrent, et récurrences des comptes déjà en \
        production. Un compte facture une fois par mois civil, au premier run du mois \
        couvrant son jour de cycle.</p>\n<div class=\"chart\">\n");
    html.push_str(&crate::charts::barres_empilees(&barres));
    html.push_str(
        "\n<div class=\"chart-legend\">\
         <span><i style=\"background:var(--gold)\"></i>premières factures</span>\
         <span><i style=\"background:var(--green-later)\"></i>récurrences</span>\
         </div>\n</div>\n",
    );
```

- [ ] **Étape 5 : habiller la table**

Ajouter d'abord deux helpers en fin de fichier, à côté de `pourcent` :

```rust
/// Part en pourcentage pour une largeur de barre. Total nul → 0, pas de NaN.
fn part(x: u64, total: u64) -> f64 {
    if total == 0 {
        0.0
    } else {
        100.0 * x as f64 / total as f64
    }
}

/// Écart en points, signe toujours explicite, virgule française et vrai moins
/// typographique — c'est ce que montre la maquette validée.
fn fmt_ecart(points: f64) -> String {
    format!("{points:+.1} pt").replace('.', ",").replace('-', "−")
}
```

Remplacer l'ouverture de la section MEP/Runs :

```rust
    html.push_str(
        "<h2>Mises en production et Runs de Facturation</h2>\n\
         <p class=\"h2sub\">Le fichier de chaque MEP est <b>cumulatif</b> : il contient \
         aussi les comptes des MEP précédentes.</p>\n<div class=\"tbl\">\n<table>\n\
         <thead><tr><th>MEP</th><th>Date</th><th>Run</th><th>Date du run</th>\
         <th>Jours de cycle</th><th class=\"num\">Comptes</th>\
         <th class=\"num\">Cumul</th></tr></thead>\n<tbody>\n",
    );
```

Puis remplacer la boucle de lignes par :

```rust
    let mut cumul_table = 0u64;
    let mut mep_precedente = usize::MAX;
    for ((mep_id, mep_date, run_date, run_num), lignes) in &par_run {
        cumul_table += lignes.len() as u64;
        let mut jjs: Vec<u8> = lignes.iter().map(|l| l.jj).collect();
        jjs.sort_unstable();
        jjs.dedup();
        let debut_mep = *mep_id != mep_precedente;
        mep_precedente = *mep_id;
        // Le gel est une propriété de la MEP : toutes les lignes du run la
        // partagent, `all` le dit sans supposer que le groupe est homogène.
        let gelee = lignes.iter().all(|l| l.gelee(d.aujourdhui));
        html.push_str(&format!(
            "<tr{}><td class=\"mep-cell\">{mep_id}{}</td><td>{}</td><td>{}</td>\
             <td>{}</td><td class=\"jj\">{}</td><td class=\"num\"><b>{}</b></td>\
             <td class=\"num\">{}</td></tr>\n",
            if debut_mep { " class=\"mep-start\"" } else { "" },
            if gelee { "<span class=\"frozen\">gelée</span>" } else { "" },
            esc(mep_date),
            esc(run_num),
            esc(run_date),
            esc(&jjs.iter().map(u8::to_string).collect::<Vec<_>>().join(", ")),
            fmt_int(lignes.len() as u64),
            fmt_int(cumul_table),
        ));
    }
    html.push_str("</tbody>\n</table>\n</div>\n");
```

- [ ] **Étape 6 : habiller la répartition par plateforme**

Remplacer l'ouverture de la section et sa boucle par :

```rust
    html.push_str(
        "<h2>Répartition par plateforme</h2>\n\
         <p class=\"h2sub\">Part de chaque plateforme dans le plan, comparée à sa part du \
         pool éligible. L'écart en points signale une plateforme sur- ou sous-servie.</p>\n\
         <div class=\"dist\">\n<div class=\"chart-legend\" style=\"margin-bottom:10px\">\
         <span><i style=\"background:var(--gold)\"></i>part du plan</span>\
         <span><i style=\"background:var(--pa-autres)\"></i>part du pool éligible</span>\
         </div>\n",
    );
    for nom in noms {
        let p = plan_par_pa.get(nom).copied().unwrap_or(0);
        let e = d.pool_par_pa.get(nom).copied().unwrap_or(0) as u64;
        let part_plan = part(p, total);
        let part_pool = part(e, pool_total as u64);
        let ecart = part_plan - part_pool;
        html.push_str(&format!(
            "<div class=\"dist-row{}\"><div class=\"dist-name\">{}</div>\
             <div class=\"dist-bars\">\
             <div class=\"bar\"><i style=\"width:{part_plan:.1}%\"></i></div>\
             <div class=\"bar pool\"><i style=\"width:{part_pool:.1}%\"></i></div></div>\
             <div class=\"dist-n\"><b>{}</b> · {}<br>{} · {}</div>\
             <div class=\"dist-gap {}\">{}</div></div>\n",
            if p == 0 { " absent" } else { "" },
            esc(nom),
            fmt_int(p),
            pourcent(p, total),
            fmt_int(e),
            pourcent(e, pool_total as u64),
            if ecart >= 0.0 { "over" } else { "under" },
            fmt_ecart(ecart),
        ));
    }
    html.push_str("</div>\n");
```

- [ ] **Étape 7 : ajouter la bande de contrôle**

Juste avant le `<footer>` :

```rust
    html.push_str(
        "<h2>Contrôle du plan</h2>\n<p class=\"h2sub\">Ce qui a été figé ou repris à la \
         main. Les comptes retirés sont exclus de tous les chiffres ci-dessus.</p>\n\
         <section class=\"kpis sub\">\n",
    );
    for (valeur, libelle) in [
        (geles, "gelés (MEP passée)"),
        (manuels, "retouches manuelles"),
        (couverture, "placés pour couverture"),
        (retires, "retirés"),
    ] {
        html.push_str(&format!(
            "<div class=\"kpi{}\"><div class=\"v\">{}</div>\
             <div class=\"l\">{libelle}</div></div>\n",
            if valeur > 0 { " on" } else { "" },
            fmt_int(valeur)
        ));
    }
    html.push_str("</section>\n");
```

- [ ] **Étape 8 : écrire le test de la table et de l'écart**

```rust
#[test]
fn la_table_marque_le_debut_de_mep_et_le_gel() {
    let mut l2 = ligne("CF2", "Cegedim", "2026-10-01", Origine::Auto);
    l2.mep_id = 2;
    l2.run_num = "R2".into();
    // MEP 1 au 01/07, soit avant le 25/07 que `data` donne pour aujourd'hui :
    // elle est donc gelée, et c'est ce badge que le test cherche.
    let lignes = vec![ligne("CF1", "Cegedim", "2026-07-01", Origine::Auto), l2];
    let pool = BTreeMap::from([("Cegedim".to_string(), 8usize)]);
    let jj = BTreeMap::from([(5u8, 8usize)]);
    let html = render(&data(&lignes, &pool, &jj, &runs_test()));
    assert!(html.contains("class=\"mep-start\""), "chaque MEP ouvre un groupe");
    assert!(html.contains("class=\"frozen\">gelée"), "la MEP passée est signalée : {html}");
    assert!(html.contains("<tbody>"), "table structurée, pas un empilement de <tr>");
}

#[test]
fn lecart_de_repartition_est_signe_et_en_francais() {
    assert_eq!(fmt_ecart(4.4), "+4,4 pt");
    assert_eq!(fmt_ecart(-1.6), "−1,6 pt");
    assert_eq!(part(0, 0), 0.0, "aucune largeur de barre ne doit valoir NaN");
}
```

- [ ] **Étape 9 : lancer les tests et vérifier qu'ils passent**

```bash
cd client/src-tauri && cargo test --lib plan_report::
```

Attendu : tous verts.

- [ ] **Étape 10 : commit**

```bash
git add client/src-tauri/src/plan_report.rs
git commit -m "feat(superpopaul): courbes et mise en page du rapport de plan"
```

---

## Tâche 10 : `commands.rs` — câblage

**Fichiers :**
- Modifier : `client/src-tauri/src/commands.rs` (`plan_rapport`, ligne 1435)

- [ ] **Étape 1 : fournir runs retenus et pool par jour de cycle**

Dans `plan_rapport`, après la construction de `pool_par_pa`, ajouter :

```rust
        let mut pool_par_jj: std::collections::BTreeMap<u8, usize> = Default::default();
        for c in &pool {
            *pool_par_jj.entry(c.jj).or_insert(0) += 1;
        }
        // Runs RETENUS : `calendrier_du_plan` applique déjà les trois filtres
        // (exclusion, fenêtre, MEP passée), comme pour `plan_ajouter`.
        let (runs, _meps) = calendrier_du_plan(&meta)?;
```

Puis dans l'appel à `plan_report::render`, remplacer
`avertissements: &[],` par :

```rust
            pool_par_jj: &pool_par_jj,
            runs: &runs,
```

- [ ] **Étape 2 : compiler**

```bash
cd client/src-tauri && cargo build 2>&1 | tail -5
```

Attendu : compilation sans erreur.

- [ ] **Étape 3 : lancer la suite complète et clippy**

```bash
cd client/src-tauri && cargo test 2>&1 | tail -5 && cargo clippy --all-targets 2>&1 | grep -c "^warning"
```

Attendu : tous les tests verts, **5** warnings clippy (les préexistants).

- [ ] **Étape 4 : commit**

```bash
git add client/src-tauri/src/commands.rs
git commit -m "feat(superpopaul): le rapport de plan reçoit runs et pool par jour de cycle"
```

---

## Tâche 11 : passe de mutation

Obligatoire. La session du 25–26/07 a trouvé un test incapable d'échouer à
**chacune** de ses cinq tâches Rust — jamais des tests manquants, toujours des
tests qui passaient quoi qu'il arrive.

**Fichiers :** aucun modifié si tout va bien.

- [ ] **Étape 1 : muter le cœur du calcul**

Appliquer une à une ces mutations dans `charge.rs`, relancer `cargo test charge::`
après chacune, **remettre le code en état** ensuite :

| # | Mutation | Test qui DOIT échouer |
|---|----------|-----------------------|
| 1 | `if vu.insert(...)` → `vu.insert(...); if true` | `un_compte_ne_facture_quune_fois_par_mois` |
| 2 | `.filter(\|&&i\| i > depart)` → `.filter(\|&&i\| i >= depart)` | `pas_de_recurrence_avant_le_demarrage` |
| 3 | `out[depart].premieres += 1;` → supprimer | plusieurs |
| 4 | clé du porteur `(year, month, jj)` → `(year, jj)` | `un_compte_ne_facture_quune_fois_par_mois` |

Si une mutation ne fait échouer **aucun** test, le test correspondant est creux :
le corriger avant d'aller plus loin, et le signaler dans le compte rendu.

- [ ] **Étape 2 : muter les graphes**

| # | Mutation | Test qui DOIT échouer |
|---|----------|-----------------------|
| 5 | dans `barres_empilees`, `esc(&b.label)` → `b.label.clone()` | `les_etiquettes_de_run_sont_echappees` |
| 6 | dans `echelle`, `if max == 0` → `if false` | `echelle_dun_maximum_nul_ne_divise_pas_par_zero` |
| 7 | dans `aire_cumulee`, `H`/`V` → un segment `L` oblique | `aire_cumulee_trace_un_escalier_et_ses_jalons` |

- [ ] **Étape 3 : muter les avertissements**

| # | Mutation | Test qui DOIT échouer |
|---|----------|-----------------------|
| 8 | `!servies.contains(...)` → `servies.contains(...)` | `avertit_sur_une_plateforme_du_pool_sans_compte_planifie` |
| 9 | `!couverts.contains(jj)` → `false` | `avertit_sur_un_jour_de_cycle_hors_datteinte` |

- [ ] **Étape 4 : vérifier le retour à l'état initial**

```bash
cd client/src-tauri && git diff --stat && cargo test 2>&1 | tail -3
```

Attendu : **aucun diff** (toutes les mutations annulées) et suite verte.

---

## Tâche 12 : parcours GUI

Les trois défauts de la session précédente étaient invisibles hors application.
Aucun test ne remplace cette étape.

- [ ] **Étape 1 : lancer l'application**

```bash
cd client && npm run tauri dev
```

- [ ] **Étape 2 : produire un rapport**

Charger un CSV, aller jusqu'à l'écran Plan de charge, établir un plan, puis
générer le rapport. Ouvrir le `.html` produit.

- [ ] **Étape 3 : vérifier**

- les deux graphes s'affichent, avec des barres et une courbe non dégénérées ;
- le pic annoncé en carte correspond bien à la barre la plus haute ;
- la table est encadrée et lisible (elle ne doit plus ressembler à du HTML brut) ;
- la répartition affiche deux barres par plateforme et un écart en points ;
- basculer le thème du système en **clair** : le rapport suit ;
- aperçu d'impression (`Cmd+P`) : fond blanc, graphes lisibles, rien de tronqué ;
- un plan avec une plateforme non servie affiche bien la section Avertissements.

- [ ] **Étape 4 : rendre compte**

Signaler tout écart avec la maquette validée. Ne pas déclarer le lot terminé
avant que ce parcours soit passé.

---

## Définition de terminé

- [ ] `cargo test` vert, effectif attendu **457 + 27 neufs − 1 remplacé = 483**
      (7 `charge`, 11 `charts`, 1 `report`, 4 avertissements, 5 mise en page ;
      `rapport_echappe_aussi_les_avertissements` disparaît au profit de
      `les_avertissements_derives_sont_echappes`)
- [ ] `cargo clippy --all-targets` : 5 warnings, les préexistants, aucun neuf
- [ ] Passe de mutation faite, aucune mutation survivante
- [ ] Parcours GUI validé par l'utilisateur, thèmes sombre et clair, impression
- [ ] `git status` propre, commits en `feat(superpopaul): …`
