# Timeline calendaire du plan de charge — plan d'implémentation

> **Pour les agents :** SOUS-SKILL REQUIS — utiliser `superpowers:subagent-driven-development`
> (recommandé) ou `superpowers:executing-plans` pour exécuter ce plan tâche par
> tâche. Les étapes sont en cases à cocher (`- [ ]`).

**But :** remplacer la table des Runs de Facturation de l'onglet *Paramétrage*
par une timeline par jour civil, qui montre les runs écartés avec leur motif,
les jalons du calendrier, les jours chômés, et la distribution du pool par jour
de cycle.

**Architecture :** un module Rust neuf, `timeline.rs`, assemble la timeline à
partir du calendrier et des détails d'allocation ; `calendrier.rs` gagne le
calcul des fériés, `plan.rs` la distribution par jour de cycle. `PlanApercu`
troque son champ `details` contre `timeline` + `stock_jj`. Le JS ne fait que
rendre — aucune décision métier côté UI.

**Pile :** Rust (chrono 0.4, serde), Tauri, JS vanilla sans bundler, CSS maison.

**Spec :** `docs/superpowers/specs/2026-07-25-plan-timeline-calendaire-design.md`

**Commandes de référence** (depuis la racine du dépôt, sans `cd`) :

```bash
cargo test  --manifest-path client/src-tauri/Cargo.toml <filtre>
cargo check --manifest-path client/src-tauri/Cargo.toml --bins
cargo clippy --manifest-path client/src-tauri/Cargo.toml --all-targets
```

Rappel : `direct.rs`, `resolver.rs`, `directory.rs` et `commands.rs:88` portent
**5 warnings clippy préexistants**. Ne pas croire les avoir introduits, ne pas
les corriger dans ce lot.

---

## Structure des fichiers

| Fichier | Responsabilité |
|---|---|
| `client/src-tauri/src/timeline.rs` | **neuf** — assemble les jours civils : jalons, jours chômés, runs et leur motif d'écart. Pur : aucune DB, aucune E/S. |
| `client/src-tauri/src/calendrier.rs` | ajout de `feries()` et du computus de Meeus. Reste un module de calendrier pur. |
| `client/src-tauri/src/plan.rs` | ajout de `StockJJ` et `stock_par_jj()` — porte sur le pool, pas sur le calendrier. |
| `client/src-tauri/src/lib.rs` | déclaration du module `timeline`. |
| `client/src-tauri/src/commands.rs` | `PlanApercu` : `details` → `timeline`, `+ stock_jj`. |
| `client/src/app.js` | `renderPlanParam` : rendu de la timeline, case « exclure », graphe des jours de cycle. |
| `client/src/styles.css` | styles de la timeline et des barres. |

---

## Tâche 1 : les fériés français

**Fichiers :**
- Modifier : `client/src-tauri/src/calendrier.rs`

Le module a déjà un helper de test `d(iso: &str) -> NaiveDate` dans son
`mod tests`. Les deux années figées ci-dessous ont été recoupées à la main :
Pâques 2026 tombe le **5 avril**, Pâques 2024 le **31 mars** ; l'Ascension 2024
est le jeudi **9 mai** et le lundi de Pentecôte 2024 le **20 mai** — dates
fériées françaises publiques. Si l'implémenteur a un doute, il les recoupe sur
une source externe : **jamais depuis l'implémentation qu'elles vérifient**.

- [ ] **Étape 1 : écrire les tests qui échouent**

À placer dans le `mod tests` existant de `calendrier.rs` :

```rust
    #[test]
    fn feries_2026_les_onze_dates() {
        let f = feries(2026);
        let dates: Vec<NaiveDate> = f.iter().map(|(d, _)| *d).collect();
        assert_eq!(dates.len(), 11, "onze fériés nationaux, pas un de plus");
        assert_eq!(
            dates,
            vec![
                d("2026-01-01"),
                d("2026-04-06"), // lundi de Pâques (Pâques le 5 avril)
                d("2026-05-01"),
                d("2026-05-08"),
                d("2026-05-14"), // Ascension
                d("2026-05-25"), // lundi de Pentecôte
                d("2026-07-14"),
                d("2026-08-15"),
                d("2026-11-01"),
                d("2026-11-11"),
                d("2026-12-25"),
            ],
            "les fériés sortent triés par date"
        );
        let noms: Vec<&str> = f.iter().map(|(_, n)| *n).collect();
        assert_eq!(
            noms,
            vec![
                "Jour de l'an",
                "Lundi de Pâques",
                "Fête du Travail",
                "Victoire 1945",
                "Ascension",
                "Lundi de Pentecôte",
                "Fête nationale",
                "Assomption",
                "Toussaint",
                "Armistice",
                "Noël",
            ],
            "les onze fériés sont nommés, dans l'ordre des dates triées — aucun sous un « férié » générique"
        );
    }

    #[test]
    fn feries_2024_mobiles_ancres_sur_paques() {
        // Deuxième année indépendante de 2026 : un décalage 1/39/50 mal
        // transcrit, ou un computus juste par coïncidence sur la seule année
        // 2026, se verrait ici (Pâques 2024 tombe le 31 mars).
        let dates: Vec<NaiveDate> = feries(2024).iter().map(|(d, _)| *d).collect();
        assert_eq!(
            dates,
            vec![
                d("2024-01-01"),
                d("2024-04-01"), // lundi de Pâques (Pâques le 31 mars)
                d("2024-05-01"),
                d("2024-05-08"),
                d("2024-05-09"), // Ascension
                d("2024-05-20"), // lundi de Pentecôte
                d("2024-07-14"),
                d("2024-08-15"),
                d("2024-11-01"),
                d("2024-11-11"),
                d("2024-12-25"),
            ]
        );
    }

    #[test]
    fn feries_2049_couvre_la_correction_m_du_computus() {
        // Le terme m = (a + 11h + 22l) / 451 vaut 0 en 2024 comme en 2026 :
        // aucun des deux tests précédents ne l'exerce. Sur 2000-2100 il ne
        // vaut 1 qu'en 2049 et 2076. Référence établie par l'algorithme de
        // Gauss, indépendant du computus testé ici : Pâques 2049 le 18 avril
        // — si m restait à 0 par erreur de transcription, le calcul donnerait
        // le 25 avril.
        let dates: Vec<NaiveDate> = feries(2049).iter().map(|(d, _)| *d).collect();
        assert!(dates.contains(&d("2049-04-19")), "lundi de Pâques (Pâques le 18 avril)");
        assert!(dates.contains(&d("2049-05-27")), "Ascension");
        assert!(dates.contains(&d("2049-06-07")), "lundi de Pentecôte");
    }
```

⚠️ Le nom d'un test doit décrire ce que le code peut réellement casser.
`paques` ne consulte jamais la longueur de février — un test nommé
« propagent le décalage bissextile » annoncerait donc un mécanisme absent.
C'est ce qu'une revue a corrigé ici ; la leçon vaut pour tout le plan.

- [ ] **Étape 2 : lancer les tests pour les voir échouer**

```bash
cargo test --manifest-path client/src-tauri/Cargo.toml calendrier::tests::feries
```

Attendu : ÉCHEC de compilation — `cannot find function 'feries' in this scope`.

- [ ] **Étape 3 : écrire l'implémentation minimale**

À ajouter dans `calendrier.rs`, après `runs_utilisables` :

```rust
/// Dimanche de Pâques, computus de Meeus (forme grégorienne anonyme).
fn paques(annee: i32) -> NaiveDate {
    let a = annee % 19;
    let b = annee / 100;
    let c = annee % 100;
    let d = b / 4;
    let e = b % 4;
    let f = (b + 8) / 25;
    let g = (b - f + 1) / 3;
    let h = (19 * a + b - d - g + 15) % 30;
    let i = c / 4;
    let k = c % 4;
    let l = (32 + 2 * e + 2 * i - h - k) % 7;
    let m = (a + 11 * h + 22 * l) / 451;
    let n = h + l - 7 * m + 114;
    NaiveDate::from_ymd_opt(annee, (n / 31) as u32, (n % 31 + 1) as u32)
        .expect("le computus ne produit que des dates de mars ou d'avril")
}

fn date_fixe(annee: i32, mois: u32, jour: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(annee, mois, jour)
        .expect("aucun férié fixe français ne tombe un 29 février")
}

/// Les onze jours fériés nationaux français de l'année, triés par date, avec
/// leur nom. Pas de particularisme d'Alsace-Moselle : parité avec peppolstat.
///
/// Purement décoratifs : aucun calcul du plan ne les lit. Ils servent à
/// comprendre un calendrier de runs, pas à le corriger.
pub fn feries(annee: i32) -> Vec<(NaiveDate, &'static str)> {
    let p = paques(annee);
    let mut out = vec![
        (date_fixe(annee, 1, 1), "Jour de l'an"),
        (date_fixe(annee, 5, 1), "Fête du Travail"),
        (date_fixe(annee, 5, 8), "Victoire 1945"),
        (date_fixe(annee, 7, 14), "Fête nationale"),
        (date_fixe(annee, 8, 15), "Assomption"),
        (date_fixe(annee, 11, 1), "Toussaint"),
        (date_fixe(annee, 11, 11), "Armistice"),
        (date_fixe(annee, 12, 25), "Noël"),
        (p + chrono::Duration::days(1), "Lundi de Pâques"),
        (p + chrono::Duration::days(39), "Ascension"),
        (p + chrono::Duration::days(50), "Lundi de Pentecôte"),
    ];
    out.sort_by_key(|(d, _)| *d);
    out
}
```

- [ ] **Étape 4 : lancer les tests pour les voir passer**

```bash
cargo test --manifest-path client/src-tauri/Cargo.toml calendrier::tests::feries
```

Attendu : `test result: ok. 3 passed`.

- [ ] **Étape 5 : commit**

```bash
git add client/src-tauri/src/calendrier.rs
git commit -m "feat(superpopaul): fériés français calculés (computus de Meeus)"
```

---

## Tâche 2 : le module `timeline` — étendue et jours

**Fichiers :**
- Créer : `client/src-tauri/src/timeline.rs`
- Modifier : `client/src-tauri/src/lib.rs`

- [ ] **Étape 1 : déclarer le module**

Dans `lib.rs`, en respectant l'ordre alphabétique existant, entre
`pub mod store;` et `pub mod telemetry;` :

```rust
pub mod telemetry;
pub mod timeline;
```

(`timeline` vient après `telemetry` en alphabétique — placer la ligne juste
après.)

- [ ] **Étape 2 : écrire le test qui échoue**

Créer `client/src-tauri/src/timeline.rs` avec, pour l'instant, seulement les
types, une fonction vide et le test :

```rust
//! Assemblage de la timeline calendaire de l'écran Plan de charge.
//!
//! Module PUR : aucune DB, aucune UI, aucun accès disque. Il ne décide rien —
//! il met bout à bout ce que `calendrier` et `plan` ont déjà établi, pour que
//! l'UI n'ait qu'à rendre des lignes.

use crate::calendrier::RunFacturation;
use crate::plan::DetailRun;
use chrono::{Datelike, NaiveDate, Weekday};
use std::collections::HashMap;

/// Ce qui coupe le calendrier. Plusieurs peuvent tomber le même jour.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "sorte", rename_all = "snake_case")]
pub enum Jalon {
    DebutFenetre,
    FinFenetre,
    Mep { rang: usize },
}

/// Pourquoi un run ne compte pas. Miroir exact des trois filtres de
/// `calendrier::runs_utilisables`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Ecart {
    Exclu,
    HorsFenetre,
    MepNonPassee,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct RunJour {
    pub num: String,
    pub jjs: Vec<u8>,
    pub exclu: bool,
    pub ecart: Option<Ecart>,
    /// Présent si et seulement si `ecart` est `None`.
    pub detail: Option<DetailRun>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct JourTimeline {
    /// ISO, comme le reste de la charge utile envoyée au JS.
    pub date: String,
    /// « lun » … « dim ».
    pub jour_semaine: &'static str,
    pub weekend: bool,
    pub ferie: Option<&'static str>,
    pub jalons: Vec<Jalon>,
    /// Une liste, pas un `Option` : rien n'interdit deux runs à la même date,
    /// et un run perdu en silence est ce que ce lot corrige.
    pub runs: Vec<RunJour>,
}

pub fn timeline(
    _runs: &[RunFacturation],
    _debut: NaiveDate,
    _fin: NaiveDate,
    _meps: &[NaiveDate],
    _details: &[DetailRun],
) -> Vec<JourTimeline> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(iso: &str) -> NaiveDate {
        NaiveDate::parse_from_str(iso, "%Y-%m-%d").unwrap()
    }

    fn run(num: &str, date: &str, jjs: &[u8]) -> RunFacturation {
        RunFacturation { num: num.into(), date: d(date), jjs: jjs.to_vec(), exclu: false }
    }

    #[test]
    fn couvre_tous_les_jours_sans_trou() {
        let t = timeline(&[], d("2026-07-01"), d("2026-07-05"), &[], &[]);
        assert_eq!(
            t.iter().map(|j| j.date.as_str()).collect::<Vec<_>>(),
            vec!["2026-07-01", "2026-07-02", "2026-07-03", "2026-07-04", "2026-07-05"],
            "un jour manquant serait un trou dans le calendrier affiché"
        );
        assert!(
            t.iter().all(|j| j.runs.is_empty()),
            "sans calendrier chargé, l'étendue se réduit à la fenêtre et aucun jour ne porte de run"
        );
    }

    #[test]
    fn etendue_deborde_la_fenetre_pour_montrer_les_runs_hors_fenetre() {
        // Le run du 20 est hors fenêtre. S'il sortait de l'étendue, l'écran ne
        // pourrait pas expliquer pourquoi il ne compte pas — le défaut même
        // que ce lot corrige.
        let t = timeline(
            &[run("3326", "2026-07-20", &[17])],
            d("2026-07-10"),
            d("2026-07-15"),
            &[],
            &[],
        );
        assert_eq!(t.first().unwrap().date, "2026-07-10");
        assert_eq!(t.last().unwrap().date, "2026-07-20");
    }

    #[test]
    fn jours_de_week_end_marques() {
        // 4 et 5 juillet 2026 : samedi et dimanche.
        let t = timeline(&[], d("2026-07-01"), d("2026-07-06"), &[], &[]);
        let we: Vec<&str> =
            t.iter().filter(|j| j.weekend).map(|j| j.date.as_str()).collect();
        assert_eq!(we, vec!["2026-07-04", "2026-07-05"]);
        assert_eq!(t[0].jour_semaine, "mer", "1er juillet 2026 est un mercredi");
    }

    #[test]
    fn feries_portes_par_le_jour() {
        let t = timeline(&[], d("2026-07-13"), d("2026-07-15"), &[], &[]);
        assert_eq!(t[1].ferie, Some("Fête nationale"), "le 14 juillet");
        assert_eq!(t[0].ferie, None);
    }
}
```

- [ ] **Étape 3 : lancer les tests pour les voir échouer**

```bash
cargo test --manifest-path client/src-tauri/Cargo.toml timeline::
```

Attendu : ÉCHEC — les quatre tests échouent sur une timeline vide
(`assertion 'left == right' failed`, `left: []`).

`Datelike`, `Weekday` et `HashMap` sont importés dès maintenant pour l'étape 4 :
la compilation émet donc trois `unused_imports` à ce stade, qui disparaissent
avec l'implémentation. Ne pas les « corriger » en retirant les `use`.

- [ ] **Étape 4 : écrire l'implémentation**

Remplacer le corps de `timeline` :

```rust
const JOURS: [&str; 7] = ["lun", "mar", "mer", "jeu", "ven", "sam", "dim"];

pub fn timeline(
    runs: &[RunFacturation],
    debut: NaiveDate,
    fin: NaiveDate,
    _meps: &[NaiveDate],
    _details: &[DetailRun],
) -> Vec<JourTimeline> {
    let lo = runs.iter().map(|r| r.date).min().unwrap_or(debut).min(debut);
    let hi = runs.iter().map(|r| r.date).max().unwrap_or(fin).max(fin);

    let mut feries: HashMap<NaiveDate, &'static str> = HashMap::new();
    for annee in lo.year()..=hi.year() {
        feries.extend(crate::calendrier::feries(annee));
    }

    let mut out = Vec::new();
    let mut jour = lo;
    while jour <= hi {
        out.push(JourTimeline {
            date: jour.to_string(),
            jour_semaine: JOURS[jour.weekday().num_days_from_monday() as usize],
            weekend: matches!(jour.weekday(), Weekday::Sat | Weekday::Sun),
            ferie: feries.get(&jour).copied(),
            jalons: Vec::new(),
            runs: Vec::new(),
        });
        jour += chrono::Duration::days(1);
    }
    out
}
```

- [ ] **Étape 5 : lancer les tests pour les voir passer**

```bash
cargo test --manifest-path client/src-tauri/Cargo.toml timeline::
```

Attendu : `test result: ok. 4 passed`.

- [ ] **Étape 6 : commit**

```bash
git add client/src-tauri/src/timeline.rs client/src-tauri/src/lib.rs
git commit -m "feat(superpopaul): module timeline — jours civils, week-ends, fériés"
```

---

## Tâche 3 : les motifs d'écart des runs

**Fichiers :**
- Modifier : `client/src-tauri/src/timeline.rs`

- [ ] **Étape 1 : écrire les tests qui échouent**

À ajouter dans le `mod tests` de `timeline.rs` :

```rust
    fn detail(num: &str, vise: usize, place: usize) -> DetailRun {
        DetailRun {
            run_num: num.into(),
            run_date: "2026-07-09".into(),
            jjs: vec![8],
            mep_id: 1,
            mep_date: "2026-07-08".into(),
            vise,
            report_entrant: 0,
            stock: 240,
            place,
            reliquat: 0,
        }
    }

    #[test]
    fn run_hors_fenetre_reste_visible_avec_son_motif() {
        // Sans motif affiché, une cible non atteinte reste inexplicable :
        // c'est le défaut de la v1 que ce lot corrige.
        let t = timeline(
            &[run("3327", "2026-07-22", &[19])],
            d("2026-07-10"),
            d("2026-07-20"),
            &[d("2026-07-11")],
            &[],
        );
        let j = t.iter().find(|j| j.date == "2026-07-22").unwrap();
        assert_eq!(j.runs[0].ecart, Some(Ecart::HorsFenetre));
        assert_eq!(j.runs[0].detail, None, "un run écarté n'a pas de chiffres");
    }

    #[test]
    fn run_le_jour_meme_de_la_premiere_mep_est_ecarte() {
        // Le filtre de runs_utilisables est STRICT (`r.date > premiere`) : un
        // run tombant le jour de la MEP est écarté lui aussi. C'est ce cas qui
        // interdit le libellé « avant la première MEP ».
        let t = timeline(
            &[run("3319", "2026-07-08", &[6])],
            d("2026-07-01"),
            d("2026-07-20"),
            &[d("2026-07-08")],
            &[],
        );
        let j = t.iter().find(|j| j.date == "2026-07-08").unwrap();
        assert_eq!(j.runs[0].ecart, Some(Ecart::MepNonPassee));
    }

    #[test]
    fn exclusion_manuelle_prime_sur_les_autres_motifs() {
        // L'exclusion est le seul motif que l'utilisateur pilote depuis
        // l'écran : elle doit rester lisible même sur un run par ailleurs
        // hors fenêtre, sinon décocher la case n'a aucun effet visible.
        let mut r = run("3321", "2026-07-30", &[9]);
        r.exclu = true;
        let t = timeline(&[r], d("2026-07-01"), d("2026-07-20"), &[d("2026-07-05")], &[]);
        let j = t.iter().find(|j| j.date == "2026-07-30").unwrap();
        assert_eq!(j.runs[0].ecart, Some(Ecart::Exclu));
        assert!(j.runs[0].exclu);
    }

    #[test]
    fn run_retenu_porte_ses_chiffres() {
        let t = timeline(
            &[run("3320", "2026-07-09", &[8])],
            d("2026-07-01"),
            d("2026-07-20"),
            &[d("2026-07-08")],
            &[detail("3320", 143, 143)],
        );
        let j = t.iter().find(|j| j.date == "2026-07-09").unwrap();
        assert_eq!(j.runs[0].ecart, None);
        assert_eq!(j.runs[0].detail.as_ref().unwrap().vise, 143);
    }

    #[test]
    fn deux_runs_le_meme_jour_sont_tous_deux_rendus() {
        // Rien dans le contrat de runs.csv n'interdit deux runs à la même
        // date. En perdre un en silence serait la faute que ce lot corrige.
        let t = timeline(
            &[run("3320", "2026-07-09", &[8]), run("3321", "2026-07-09", &[9])],
            d("2026-07-01"),
            d("2026-07-20"),
            &[d("2026-07-08")],
            &[],
        );
        let j = t.iter().find(|j| j.date == "2026-07-09").unwrap();
        assert_eq!(j.runs.len(), 2);
        assert_eq!(j.runs[0].num, "3320");
        assert_eq!(j.runs[1].num, "3321");
    }

    #[test]
    fn sans_aucune_mep_tout_run_est_ecarte() {
        // `runs_utilisables` ne rend rien sans MEP : il n'y a rien à facturer.
        let t = timeline(
            &[run("3320", "2026-07-09", &[8])],
            d("2026-07-01"),
            d("2026-07-20"),
            &[],
            &[],
        );
        let j = t.iter().find(|j| j.date == "2026-07-09").unwrap();
        assert_eq!(j.runs[0].ecart, Some(Ecart::MepNonPassee));
    }
```

- [ ] **Étape 2 : lancer les tests pour les voir échouer**

```bash
cargo test --manifest-path client/src-tauri/Cargo.toml timeline::
```

Attendu : ÉCHEC — `index out of bounds: the len is 0` sur `j.runs[0]`, les six
nouveaux tests en échec.

- [ ] **Étape 3 : écrire l'implémentation**

Remplacer la signature et la boucle de `timeline` :

```rust
pub fn timeline(
    runs: &[RunFacturation],
    debut: NaiveDate,
    fin: NaiveDate,
    meps: &[NaiveDate],
    details: &[DetailRun],
) -> Vec<JourTimeline> {
    let lo = runs.iter().map(|r| r.date).min().unwrap_or(debut).min(debut);
    let hi = runs.iter().map(|r| r.date).max().unwrap_or(fin).max(fin);

    let mut feries: HashMap<NaiveDate, &'static str> = HashMap::new();
    for annee in lo.year()..=hi.year() {
        feries.extend(crate::calendrier::feries(annee));
    }

    let premiere_mep = meps.iter().min().copied();
    let mut par_date: HashMap<NaiveDate, Vec<RunJour>> = HashMap::new();
    for r in runs {
        // Ordre délibéré : l'exclusion prime, c'est le seul motif que
        // l'utilisateur pilote depuis l'écran.
        let ecart = if r.exclu {
            Some(Ecart::Exclu)
        } else if r.date < debut || r.date > fin {
            Some(Ecart::HorsFenetre)
        } else if !premiere_mep.is_some_and(|p| r.date > p) {
            Some(Ecart::MepNonPassee)
        } else {
            None
        };
        let detail = match ecart {
            None => details.iter().find(|d| d.run_num == r.num).cloned(),
            Some(_) => None,
        };
        par_date.entry(r.date).or_default().push(RunJour {
            num: r.num.clone(),
            jjs: r.jjs.clone(),
            exclu: r.exclu,
            ecart,
            detail,
        });
    }
    for v in par_date.values_mut() {
        v.sort_by(|a, b| a.num.cmp(&b.num));
    }

    let mut out = Vec::new();
    let mut jour = lo;
    while jour <= hi {
        out.push(JourTimeline {
            date: jour.to_string(),
            jour_semaine: JOURS[jour.weekday().num_days_from_monday() as usize],
            weekend: matches!(jour.weekday(), Weekday::Sat | Weekday::Sun),
            ferie: feries.get(&jour).copied(),
            jalons: Vec::new(),
            runs: par_date.remove(&jour).unwrap_or_default(),
        });
        jour += chrono::Duration::days(1);
    }
    out
}
```

- [ ] **Étape 4 : lancer les tests pour les voir passer**

```bash
cargo test --manifest-path client/src-tauri/Cargo.toml timeline::
```

Attendu : `test result: ok. 10 passed`.

- [ ] **Étape 5 : commit**

```bash
git add client/src-tauri/src/timeline.rs
git commit -m "feat(superpopaul): motif d'écart des runs dans la timeline"
```

---

## Tâche 4 : les jalons (MEP et bornes de fenêtre)

**Fichiers :**
- Modifier : `client/src-tauri/src/timeline.rs`

- [ ] **Étape 1 : écrire les tests qui échouent**

À ajouter dans le `mod tests` de `timeline.rs` :

```rust
    #[test]
    fn bornes_de_fenetre_posees_sur_leurs_jours() {
        let t = timeline(&[], d("2026-07-10"), d("2026-07-12"), &[], &[]);
        assert_eq!(t[0].jalons, vec![Jalon::DebutFenetre]);
        assert_eq!(t[1].jalons, vec![]);
        assert_eq!(t[2].jalons, vec![Jalon::FinFenetre]);
    }

    #[test]
    fn meps_numerotees_dans_l_ordre_chronologique() {
        // Le rang affiché doit suivre les dates, pas l'ordre de saisie :
        // « MEP 2 » avant « MEP 1 » sur le calendrier serait un contresens.
        let t = timeline(
            &[],
            d("2026-07-01"),
            d("2026-07-20"),
            &[d("2026-07-15"), d("2026-07-05")],
            &[],
        );
        let j5 = t.iter().find(|j| j.date == "2026-07-05").unwrap();
        let j15 = t.iter().find(|j| j.date == "2026-07-15").unwrap();
        assert_eq!(j5.jalons, vec![Jalon::Mep { rang: 1 }]);
        assert_eq!(j15.jalons, vec![Jalon::Mep { rang: 2 }]);
    }

    #[test]
    fn plusieurs_jalons_le_meme_jour_sont_tous_rendus() {
        // Une MEP posée le dernier jour de la fenêtre : en perdre un des deux
        // rendrait le calendrier faux.
        let t = timeline(&[], d("2026-07-01"), d("2026-07-10"), &[d("2026-07-10")], &[]);
        let dernier = t.last().unwrap();
        assert_eq!(dernier.jalons, vec![Jalon::Mep { rang: 1 }, Jalon::FinFenetre]);
    }
```

- [ ] **Étape 2 : lancer les tests pour les voir échouer**

```bash
cargo test --manifest-path client/src-tauri/Cargo.toml timeline::
```

Attendu : ÉCHEC — `left: []`, les trois nouveaux tests en échec.

- [ ] **Étape 3 : écrire l'implémentation**

Dans `timeline`, avant la boucle, ajouter la table des jalons :

```rust
    let mut jalons: HashMap<NaiveDate, Vec<Jalon>> = HashMap::new();
    let mut dates_mep: Vec<NaiveDate> = meps.to_vec();
    dates_mep.sort_unstable();
    dates_mep.dedup();
    for (i, m) in dates_mep.iter().enumerate() {
        jalons.entry(*m).or_default().push(Jalon::Mep { rang: i + 1 });
    }
    jalons.entry(debut).or_default().push(Jalon::DebutFenetre);
    jalons.entry(fin).or_default().push(Jalon::FinFenetre);
```

puis, dans la construction de `JourTimeline`, remplacer
`jalons: Vec::new(),` par :

```rust
            jalons: jalons.remove(&jour).unwrap_or_default(),
```

L'ordre d'insertion place les MEP avant les bornes le même jour, ce que fige
`plusieurs_jalons_le_meme_jour_sont_tous_rendus`.

- [ ] **Étape 4 : lancer les tests pour les voir passer**

```bash
cargo test --manifest-path client/src-tauri/Cargo.toml timeline::
```

Attendu : `test result: ok. 13 passed`.

- [ ] **Étape 5 : commit**

```bash
git add client/src-tauri/src/timeline.rs
git commit -m "feat(superpopaul): jalons MEP et bornes de fenêtre dans la timeline"
```

---

## Tâche 5 : le stock par jour de cycle

**Fichiers :**
- Modifier : `client/src-tauri/src/plan.rs`

- [ ] **Étape 1 : écrire les tests qui échouent**

À ajouter dans le `mod tests` de `plan.rs`. **Ne définir aucun helper neuf** :
le module en a déjà deux, à réutiliser tels quels —
`cand(cf, jj, pa) -> CfCandidat` (`plan.rs:1480`) et `d(iso) -> NaiveDate`. Les
`RunFacturation` s'écrivent en littéral, comme partout ailleurs dans ce
`mod tests` (voir `plan.rs:1556`). `RunFacturation` est déjà importé en tête de
fichier (`plan.rs:11`) — ne pas rajouter de `use`.

```rust
    #[test]
    fn stock_par_jj_rend_les_trente_et_un_jours() {
        let s = stock_par_jj(&[cand("A", 8, "PA1")], &[]);
        assert_eq!(s.len(), 31, "les jours de cycle vides comptent aussi");
        assert_eq!(s[0].jj, 1);
        assert_eq!(s[30].jj, 31);
        assert_eq!(s[7].comptes, 1, "le jour de cycle 8 porte un compte");
    }

    #[test]
    fn stock_par_jj_signale_un_jour_sans_run() {
        // Sans ce signal, les comptes hors d'atteinte restent invisibles :
        // l'écran ne sait dire que « stock insuffisant », jamais où.
        let pool = vec![cand("A", 8, "PA1"), cand("B", 19, "PA1")];
        let retenus = [RunFacturation {
            num: "3320".into(),
            date: d("2026-07-09"),
            jjs: vec![8],
            exclu: false,
        }];
        let s = stock_par_jj(&pool, &retenus);
        assert!(s[7].couvert, "le jour de cycle 8 est couvert par le run");
        assert!(!s[18].couvert, "aucun run ne couvre le jour de cycle 19");
        assert_eq!(s[18].comptes, 1, "et pourtant un compte y est bloqué");
    }

    #[test]
    fn stock_par_jj_ne_compte_pas_les_runs_ecartes() {
        // On passe ici les runs RETENUS : un run exclu ne doit pas faire
        // croire que son jour de cycle est servi, sinon l'exclusion se ferait
        // à l'aveugle.
        let s = stock_par_jj(&[cand("A", 9, "PA1")], &[]);
        assert!(!s[8].couvert);
    }
```

- [ ] **Étape 2 : lancer les tests pour les voir échouer**

```bash
cargo test --manifest-path client/src-tauri/Cargo.toml plan::tests::stock_par_jj
```

Attendu : ÉCHEC de compilation — `cannot find function 'stock_par_jj'`.

- [ ] **Étape 3 : écrire l'implémentation**

Dans `plan.rs`, à la suite de `quotas_par_pa` :

```rust
/// Distribution du pool sur les jours de cycle, et couverture par les runs
/// **retenus**. Toujours 31 entrées : un jour de cycle vide est une
/// information, pas une absence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct StockJJ {
    pub jj: u8,
    pub comptes: usize,
    pub couvert: bool,
}

pub fn stock_par_jj(pool: &[CfCandidat], retenus: &[RunFacturation]) -> Vec<StockJJ> {
    let mut comptes = [0usize; 32];
    for c in pool {
        if (1..=31).contains(&c.jj) {
            comptes[c.jj as usize] += 1;
        }
    }
    (1..=31u8)
        .map(|jj| StockJJ {
            jj,
            comptes: comptes[jj as usize],
            couvert: retenus.iter().any(|r| r.couvre(jj)),
        })
        .collect()
}
```

Aucun `use` à ajouter : `RunFacturation` est importé ligne 11 de `plan.rs`.

- [ ] **Étape 4 : lancer les tests pour les voir passer**

```bash
cargo test --manifest-path client/src-tauri/Cargo.toml plan::tests::stock_par_jj
```

Attendu : `test result: ok. 3 passed`.

- [ ] **Étape 5 : commit**

```bash
git add client/src-tauri/src/plan.rs
git commit -m "feat(superpopaul): distribution du pool par jour de cycle"
```

---

## Tâche 6 : câbler la charge utile de l'aperçu

**Fichiers :**
- Modifier : `client/src-tauri/src/commands.rs:865-877` (struct `PlanApercu`)
- Modifier : `client/src-tauri/src/commands.rs:905-975` (`calculer_plan`)

Aucun test neuf : `commands.rs` est la couche d'assemblage, la logique est
testée en amont. La garantie est que la suite complète reste verte et que le
binaire compile.

- [ ] **Étape 1 : modifier la struct**

Dans `PlanApercu`, remplacer la ligne `pub details: …` par :

```rust
    pub timeline: Vec<crate::timeline::JourTimeline>,
    pub stock_jj: Vec<crate::plan::StockJJ>,
```

- [ ] **Étape 2 : alimenter les deux champs**

Dans `calculer_plan`, la variable `a` (l'`Allocation`) porte `a.details`. Juste
avant la construction de `apercu`, ajouter :

```rust
    let timeline =
        crate::timeline::timeline(&runs, debut, fin, &meps, &a.details);
    let stock_jj = crate::plan::stock_par_jj(&pool, &utilisables);
```

puis, dans le littéral `PlanApercu { … }`, remplacer `details: a.details,` par :

```rust
        timeline,
        stock_jj,
```

`runs` est bien la liste **complète** des runs importés (issue de
`params.calendrier()`), pas `utilisables` : c'est ce qui permet d'afficher les
runs écartés. `utilisables` va en revanche à `stock_par_jj`, dont la couverture
ne doit compter que les runs retenus.

- [ ] **Étape 3 : vérifier que tout compile et que la suite est verte**

```bash
cargo check --manifest-path client/src-tauri/Cargo.toml --bins
cargo test  --manifest-path client/src-tauri/Cargo.toml
```

Attendu : compilation sans erreur ; `test result: ok`, avec **431
tests** (412 avant ce lot, + 19 sur les tâches 1 à 5). Si un test préexistant casse,
s'arrêter et comprendre avant de continuer.

- [ ] **Étape 4 : commit**

```bash
git add client/src-tauri/src/commands.rs
git commit -m "feat(superpopaul): l'aperçu du plan porte la timeline et le stock par JJ"
```

---

## Tâche 7 : rendu de la timeline dans l'onglet Paramétrage

**Fichiers :**
- Modifier : `client/src/app.js:1378-1403` (bloc « Runs de Facturation » de
  `renderPlanParam`)

⚠️ **Le projet n'a aucune infrastructure de test JS.** Les deux seuls essais
GUI de la v1 ont révélé deux défauts de câblage JS↔Rust. Cette tâche et les
deux suivantes ne sont vérifiables qu'en lançant l'application : ne pas les
déclarer terminées sur la seule foi d'une relecture.

- [ ] **Étape 1 : remplacer le bloc de la table**

Supprimer les lignes 1378 à 1403 (de `noeuds.push(h("h2", {}, "Runs de
Facturation"));` jusqu'à `noeuds.push(tbl);` inclus) et écrire à la place :

```js
  noeuds.push(h("h2", {}, "Runs de Facturation"));
  const retenus = a.timeline.reduce(
    (n, j) => n + j.runs.filter((r) => !r.ecart).length, 0);
  const totalRuns = a.timeline.reduce((n, j) => n + j.runs.length, 0);
  noeuds.push(h("p", { class: "field-hint" },
    `${fmtN(retenus)} run(s) retenu(s) sur ${fmtN(totalRuns)} affiché(s) · rattachement à la dernière MEP strictement antérieure. Décocher un run le retire du plan.`));

  const tbl = h("table", { class: "plan-tl" },
    h("tr", {}, ...[["Jour", ""], ["Run", ""], ["Jours facturés", ""],
                    ["Visé", "n"], ["Report", "n"], ["Stock", "n"],
                    ["Placé", "n"], ["Reliquat", "n"], ["", ""]]
      .map(([t, c]) => h("th", { class: c }, t))));

  let moisCourant = "";
  for (const j of a.timeline) {
    const mois = j.date.slice(0, 7);
    if (mois !== moisCourant) {
      moisCourant = mois;
      tbl.append(h("tr", { class: "tl-mois" },
        h("td", { colspan: "9" }, libelleMois(j.date))));
    }
    for (const jl of j.jalons) tbl.append(ligneJalon(j, jl));
    if (!j.runs.length) { tbl.append(ligneVide(j)); continue; }
    for (const r of j.runs) tbl.append(ligneRun(j, r));
  }
  noeuds.push(h("div", { class: "tl-scroll" }, tbl));
```

- [ ] **Étape 2 : ajouter les fonctions de rendu**

Juste avant `function renderPlanParam()` (donc après le helper `marche`) :

```js
const TL_MOIS = ["janvier", "février", "mars", "avril", "mai", "juin", "juillet",
  "août", "septembre", "octobre", "novembre", "décembre"];

const TL_ECARTS = {
  exclu: "exclu à la main",
  hors_fenetre: "hors fenêtre",
  mep_non_passee: "la première MEP n'est pas encore passée",
};

/** « Juillet 2026 » depuis une date ISO, sans passer par Date (fuseaux). */
function libelleMois(iso) {
  const m = TL_MOIS[+iso.slice(5, 7) - 1];
  return `${m[0].toUpperCase()}${m.slice(1)} ${iso.slice(0, 4)}`;
}

function celluleJour(j) {
  return h("td", { class: "tl-jour" }, `${j.jour_semaine} ${j.date.slice(8)}`);
}

function ligneJalon(j, jl) {
  const texte = jl.sorte === "mep" ? `MEP ${jl.rang}`
    : jl.sorte === "debut_fenetre" ? "Début de la fenêtre FUT"
    : "Fin de la fenêtre FUT";
  const tr = h("tr", { class: jl.sorte === "mep" ? "tl-mep" : "tl-borne" },
    celluleJour(j),
    h("td", { colspan: "8" }, h("span", { class: "flag" }, texte)));
  return tr;
}

function ligneVide(j) {
  const notes = [];
  if (j.ferie) notes.push(`férié — ${j.ferie}`);
  else if (j.weekend) notes.push("week-end");
  return h("tr", { class: j.weekend || j.ferie ? "tl-off" : "" },
    celluleJour(j),
    h("td", { colspan: "8", class: "tl-note" }, notes.join("")));
}

function ligneRun(j, r) {
  const cb = h("input", { type: "checkbox", onchange: () => {
    const cible = plan.runs.find((x) => x.num === r.num);
    if (cible) cible.exclu = !cible.exclu;
    planRecalc();
  } });
  cb.checked = r.exclu;
  const boite = h("td", {}, h("label", { class: "tl-chk" }, cb, " exclure"));

  if (r.ecart) {
    return h("tr", { class: "tl-ecarte" },
      celluleJour(j),
      h("td", {}, r.num),
      h("td", { class: "jj" }, r.jjs.join(" · ")),
      h("td", { colspan: "5", class: "tl-why" }, `écarté — ${TL_ECARTS[r.ecart] ?? r.ecart}`),
      boite);
  }
  const d = r.detail;
  return h("tr", { class: "tl-run" },
    celluleJour(j),
    h("td", {}, r.num),
    h("td", { class: "jj" }, r.jjs.join(" · ")),
    h("td", { class: "n" }, fmtN(d.vise)),
    h("td", { class: d.report_entrant ? "n carry" : "n zero" },
      d.report_entrant ? `+${fmtN(d.report_entrant)}` : "—"),
    h("td", { class: "n" }, fmtN(d.stock)),
    h("td", { class: "n" }, fmtN(d.place)),
    h("td", { class: d.reliquat ? "n carry" : "n zero" },
      d.reliquat ? `+${fmtN(d.reliquat)}` : "0"),
    boite);
}
```

Note : la case bascule `plan.runs[i].exclu`, le tableau que `planParams()`
envoie déjà au backend — aucun code Rust à toucher, le champ `exclu` de
`RunParam` est honoré depuis la v1 (`calendrier.rs:181`).

- [ ] **Étape 3 : vérifier qu'aucune référence à `a.details` ne subsiste**

```bash
grep -n "a\.details\|\.details" client/src/app.js
```

Attendu : **aucune sortie**. Une occurrence restante planterait l'écran, le
champ n'existant plus dans la charge utile.

- [ ] **Étape 4 : commit**

```bash
git add client/src/app.js
git commit -m "feat(superpopaul): timeline calendaire à la place de la table des runs"
```

---

## Tâche 8 : le graphe du stock par jour de cycle

**Fichiers :**
- Modifier : `client/src/app.js` (`renderPlanParam`, après le bloc timeline)

- [ ] **Étape 1 : ajouter le bloc**

Juste après `noeuds.push(h("div", { class: "tl-scroll" }, tbl));` :

```js
  const totalPool = a.stock_jj.reduce((n, s) => n + s.comptes, 0);
  const atteignables = a.stock_jj.reduce((n, s) => n + (s.couvert ? s.comptes : 0), 0);
  const maxJJ = Math.max(1, ...a.stock_jj.map((s) => s.comptes));
  noeuds.push(h("h2", {}, "Stock par jour de cycle"));
  noeuds.push(h("p", { class: "field-hint" },
    "Comptes du pool éligible, par jour de cycle de facturation. En rouge, les jours qu'aucun run retenu ne couvre : ces comptes sont hors d'atteinte tant que le calendrier ou la fenêtre ne change pas."));
  const barres = h("div", { class: "jj-bars" });
  for (const s of a.stock_jj) {
    const titre = s.couvert
      ? `Jour de cycle ${s.jj} — ${fmtN(s.comptes)} comptes — couvert`
      : `Jour de cycle ${s.jj} — ${fmtN(s.comptes)} comptes — aucun run retenu ne le couvre`;
    barres.append(h("div", { class: s.couvert ? "jj-bar" : "jj-bar no", title: titre },
      h("i", { style: `height:${((s.comptes / maxJJ) * 100).toFixed(1)}%` }),
      h("span", {}, String(s.jj))));
  }
  noeuds.push(barres);
  noeuds.push(h("p", { class: "jj-legend" },
    h("b", {}, fmtN(totalPool)), " comptes éligibles · ",
    h("b", {}, fmtN(atteignables)), " atteignables par les runs retenus · ",
    h("b", {}, fmtN(totalPool - atteignables)), " hors d'atteinte."));
```

- [ ] **Étape 2 : commit**

```bash
git add client/src/app.js
git commit -m "feat(superpopaul): graphe du stock par jour de cycle"
```

---

## Tâche 9 : les styles

**Fichiers :**
- Modifier : `client/src/styles.css` (à la suite du bloc `table.plan-data`,
  vers la ligne 541)

- [ ] **Étape 1 : ajouter les styles**

```css
/* --- Timeline calendaire du plan de charge -------------------------------- */
.tl-scroll { overflow-x: auto; }
table.plan-tl { width: 100%; border-collapse: collapse; font-size: 13px;
  font-variant-numeric: tabular-nums; }
table.plan-tl th { font-size: 11.5px; color: var(--muted); font-weight: 600;
  text-align: left; padding: 0 8px 6px; white-space: nowrap; }
table.plan-tl th.n, table.plan-tl td.n { text-align: right; }
table.plan-tl td { padding: 3px 8px; border-top: 1px solid rgba(43, 55, 82, .5);
  white-space: nowrap; }
table.plan-tl td.jj { color: var(--muted); font-size: 12px; }

tr.tl-mois td { padding: 16px 8px 5px; border-top: 0; font-size: 11px;
  letter-spacing: .1em; text-transform: uppercase; color: var(--muted); }
table.plan-tl tr.tl-mois:first-child td { padding-top: 2px; }

td.tl-jour { color: var(--muted); font-size: 12.5px; }
tr.tl-run { background: rgba(43, 55, 82, .28); }
tr.tl-run td.tl-jour { color: var(--fg); font-weight: 600; }
tr.tl-off td { opacity: .62; }
td.tl-note { color: var(--muted); font-size: 12px; font-style: italic; }
tr.tl-ecarte td.tl-why { color: var(--muted); font-size: 12px; }

/* Un jalon coupe le calendrier : bande pleine largeur, pas une couleur de
   jour. Vert éteint — ni or (action) ni orange (avertissement). */
tr.tl-mep td { border-top: 1px solid var(--border); padding-top: 7px;
  padding-bottom: 7px;
  background: linear-gradient(90deg, rgba(47, 128, 80, .16), rgba(47, 128, 80, 0) 62%); }
tr.tl-mep .flag { font-weight: 600; }
tr.tl-mep .flag::before { content: "▶"; color: var(--green-later);
  margin-right: 7px; font-size: 11px; }
/* La fenêtre est un cadre, la MEP un événement : plus faible, en pointillés. */
tr.tl-borne td { border-top: 1px dashed var(--border); padding-top: 6px;
  padding-bottom: 6px; }
tr.tl-borne .flag { color: var(--muted); font-size: 11.5px; letter-spacing: .06em;
  text-transform: uppercase; }

.tl-chk { display: inline-flex; align-items: center; gap: 5px;
  color: var(--muted); font-size: 12px; cursor: pointer; }
.tl-chk input { accent-color: var(--gold); margin: 0; }

/* --- Stock par jour de cycle ---------------------------------------------- */
.jj-bars { display: flex; align-items: flex-end; gap: 3px; height: 104px;
  margin: 12px 0 0; }
.jj-bar { flex: 1; display: flex; flex-direction: column; justify-content: flex-end;
  align-items: center; gap: 4px; height: 100%; }
.jj-bar i { display: block; width: 100%; border-radius: 2px 2px 0 0;
  background: var(--green); }
.jj-bar.no i { background: var(--red); }
.jj-bar span { font-size: 10px; color: var(--muted); font-variant-numeric: tabular-nums; }
.jj-bar.no span { color: var(--red); }
.jj-legend { font-size: 12px; color: var(--muted); margin-top: 10px; }
.jj-legend b { color: var(--fg); font-variant-numeric: tabular-nums; }
```

- [ ] **Étape 2 : commit**

```bash
git add client/src/styles.css
git commit -m "feat(superpopaul): styles de la timeline et des barres de jour de cycle"
```

---

## Tâche 10 : vérification

- [ ] **Étape 1 : suite complète et compilation**

```bash
cargo test  --manifest-path client/src-tauri/Cargo.toml
cargo check --manifest-path client/src-tauri/Cargo.toml --bins
cargo clippy --manifest-path client/src-tauri/Cargo.toml --all-targets 2>&1 | grep -c warning
```

Attendu : suite verte (431 tests), compilation propre, et **5 warnings
clippy** — les préexistants, pas un de plus. Un sixième signifie que ce lot en
a introduit un.

- [ ] **Étape 2 : parcours en application**

Lancer l'application, aller à l'étape 3, ouvrir « Plan de charge → », charger
un `runs.csv` et un CSV portant des colonnes CF et JJ, puis vérifier **chacun**
de ces points :

- la timeline s'affiche, un jour par ligne, avec les en-têtes de mois ;
- les week-ends et les fériés sont atténués et libellés ;
- les bornes de la fenêtre et chaque MEP portent leur bande ;
- un run hors fenêtre affiche « écarté — hors fenêtre » ;
- décocher « exclure » sur un run retenu le fait passer à « écarté — exclu à
  la main », **et** les chiffres des autres runs bougent (le plan est
  recalculé) ;
- le graphe des jours de cycle s'affiche, avec des barres rouges sur les jours
  non couverts, et la légende chiffrée est cohérente avec l'entonnoir ;
- recocher « exclure » rétablit l'état précédent.

Tout écart constaté est un défaut à corriger avant de clore le lot — c'est le
seul filet, il n'y a pas de test JS.

- [ ] **Étape 3 : commit final s'il y a eu des correctifs**

```bash
git add -A
git commit -m "fix(superpopaul): <ce qui a été corrigé au parcours GUI>"
```
