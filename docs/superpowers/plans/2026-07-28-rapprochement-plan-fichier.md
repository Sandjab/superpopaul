# Rapprochement du plan avec un nouveau fichier — plan d'implémentation

> **Pour les agents :** SOUS-COMPÉTENCE REQUISE — utiliser
> `superpowers:subagent-driven-development` (recommandé) ou
> `superpowers:executing-plans` pour exécuter ce plan tâche par tâche. Les
> étapes utilisent la syntaxe à cases (`- [ ]`) pour le suivi.

**But :** mettre à jour un plan de charge établi sur un fichier F1 quand un
fichier F2 arrive, en retirant les comptes devenus inéligibles ou disparus et
en déplaçant ceux qui ont changé de jour de cycle, sans toucher au reste.

**Architecture :** un module `rapprochement.rs` pur (aucune I/O) calcule une
liste d'écarts et l'applique par mutation en place. Deux commandes Tauri sur
le modèle `plan_preview` / `plan_generate` : l'une calcule sans écrire, l'autre
recalcule et persiste. `plan::regenerer` n'est jamais appelé — c'est ce qui
garantit zéro re-tirage.

**Pile :** Rust (Tauri 2, chrono, rusqlite), tests `cargo test` dans
`client/src-tauri/` ; JS vanilla + faux DOM, `node --test "tests/*.test.js"`
depuis `client/`.

**Spec :** `docs/superpowers/specs/2026-07-28-rapprochement-plan-fichier-design.md`

---

## Structure des fichiers

| Fichier | Rôle | Nature |
|---|---|---|
| `client/src-tauri/src/rapprochement.rs` | types `Nature`/`Action`/`Ecart`/`Rapprochement`, `calculer`, `appliquer` | **créé** |
| `client/src-tauri/src/plan.rs` | extraction de `dedoublonner`, consommée par `construire_pool` | modifié |
| `client/src-tauri/src/store.rs` | colonne `plan_meta.rapproche_le` + migration | modifié |
| `client/src-tauri/src/commands.rs` | `plan_rapprocher`, `plan_rapprocher_appliquer`, `RapprochementVue` | modifié |
| `client/src-tauri/src/lib.rs` | déclaration du module + enregistrement des commandes | modifié |
| `client/src/app.js` | écran de revue et application | modifié (après maquette) |
| `client/src/styles.css` | styles des groupes d'écarts | modifié (après maquette) |
| `client/tests/rapprochement.test.js` | câblage IHM | **créé** |

Les tâches 1 à 7 sont livrables sans IHM : le cœur est prouvé par `cargo test`
avant qu'une ligne de JS soit écrite.

---

## Tâche 1 : extraire `plan::dedoublonner`

Le rapprochement doit hériter du refus de `construire_pool` sur un compte
présent deux fois avec deux jours de cycle différents, mais ne peut pas
l'appeler : `construire_pool` ne rend que les **éligibles**, or le
rapprochement cherche les inéligibles. Extraction à comportement constant.

**Fichiers :**
- Modifier : `client/src-tauri/src/plan.rs:84-122`

- [ ] **Étape 1 : écrire le test d'usage direct**

À ajouter dans `mod tests` de `plan.rs`, après `pool_nominal` :

```rust
    #[test]
    fn dedoublonner_garde_la_premiere_occurrence_dans_l_ordre() {
        let e = vec![
            ligne("CF1", "5", "Cegedim"),
            ligne("CF2", "12", "Esker"),
            ligne("CF1", "5", "Cegedim"),
        ];
        let out = dedoublonner(&e).unwrap();
        assert_eq!(out.len(), 2, "CF1 n'est retenu qu'une fois");
        assert_eq!(out[0].cf, "CF1", "l'ordre d'entrée est conservé");
        assert_eq!(out[1].cf, "CF2");
    }

    /// Le rapprochement rend les INÉLIGIBLES : il doit pouvoir dédoublonner
    /// sans que l'entonnoir écarte ce qu'il vient chercher.
    #[test]
    fn dedoublonner_rend_aussi_les_comptes_ineligibles() {
        let e = vec![
            avec_ctc(ligne("CF1", "5", "Cegedim"), "later"),
            {
                let mut l = ligne("CF2", "12", "Esker");
                l.ppf_usable = false;
                l
            },
        ];
        let out = dedoublonner(&e).unwrap();
        assert_eq!(out.len(), 2, "aucun filtre d'éligibilité ici");
    }
```

- [ ] **Étape 2 : lancer le test, vérifier qu'il échoue**

```bash
cd client/src-tauri && cargo test dedoublonner 2>&1 | tail -20
```

Attendu : ÉCHEC de compilation, `cannot find function 'dedoublonner' in this scope`.

- [ ] **Étape 3 : extraire la fonction**

Dans `plan.rs`, **avant** `construire_pool`, insérer :

```rust
/// Dédoublonne les entrées par compte de facturation. Première occurrence
/// retenue, ordre d'entrée conservé ; toute divergence sur le jour de cycle ou
/// l'adressage est une incohérence de données → refus fort.
///
/// Extraite de `construire_pool` pour que le rapprochement hérite du même
/// refus : lui cherche les comptes INÉLIGIBLES, que l'entonnoir écarte.
pub fn dedoublonner(entrees: &[LigneEntree]) -> Result<Vec<&LigneEntree>, String> {
    let mut vus: HashMap<&str, &LigneEntree> = HashMap::new();
    let mut ordre: Vec<&LigneEntree> = Vec::new();
    for l in entrees {
        match vus.get(l.cf.as_str()) {
            None => {
                vus.insert(&l.cf, l);
                ordre.push(l);
            }
            Some(prem) => {
                if prem.jj_brut.trim() != l.jj_brut.trim() {
                    return Err(format!(
                        "compte de facturation « {} » : deux jours de cycle différents \
                         dans le fichier ({} et {}) — corrige la donnée avant de planifier",
                        l.cf,
                        prem.jj_brut.trim(),
                        l.jj_brut.trim()
                    ));
                }
                if prem.participant != l.participant {
                    return Err(format!(
                        "compte de facturation « {} » : deux adressages différents \
                         dans le fichier ({} et {}) — corrige la donnée avant de planifier",
                        l.cf, prem.participant, l.participant
                    ));
                }
            }
        }
    }
    Ok(ordre)
}
```

- [ ] **Étape 4 : faire appeler `construire_pool`**

Dans `construire_pool`, remplacer tout le bloc `// 1) Dédoublonnage par CF …`
(les lignes de `let mut vus` jusqu'à la fermeture de la boucle `for l in entrees`)
par :

```rust
    // 1) Dédoublonnage par CF, partagé avec le rapprochement.
    let ordre = dedoublonner(entrees)?;
```

Le reste de la fonction est inchangé : `f.cf_distincts = ordre.len() as u64;`
puis `for l in ordre { … }`.

- [ ] **Étape 5 : vérifier que tout passe, tests existants compris**

```bash
cd client/src-tauri && cargo test 2>&1 | tail -20
```

Attendu : `test result: ok`, et **aucun test existant modifié**. Si un test de
`construire_pool` a dû changer, l'extraction a changé le comportement : revenir
en arrière.

⚠️ `grep "test result: ok"` masque une suite rouge — lire la sortie complète.

- [ ] **Étape 6 : commit**

```bash
git add client/src-tauri/src/plan.rs
git commit -m "refactor(superpopaul): extraire plan::dedoublonner de construire_pool"
```

---

## Tâche 2 : le module et les retraits

**Fichiers :**
- Créer : `client/src-tauri/src/rapprochement.rs`
- Modifier : `client/src-tauri/src/lib.rs`

- [ ] **Étape 1 : déclarer le module**

Dans `lib.rs`, à sa place alphabétique dans le bloc existant (entre `ppf` et
`repartition`). Tous les modules du crate sont `pub mod` sans exception :

```rust
pub mod rapprochement;
```

- [ ] **Étape 2 : écrire le fichier avec les types et un `calculer` qui ne fait rien**

Créer `client/src-tauri/src/rapprochement.rs` :

```rust
//! Rapprochement d'un plan persisté avec un fichier de comptes plus récent.
//!
//! Module **pur** : aucune I/O, aucune horloge. Ce qui dépend du disque ou de
//! la base (empreinte du fichier, état de l'annuaire PPF) vit dans
//! `commands.rs`, comme pour `securisation` et `modes`.

use crate::calendrier::RunFacturation;
use crate::plan::{LigneEntree, LignePlan};
use std::collections::HashMap;

/// Ce qui a changé pour un compte, entre le plan et le fichier courant.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Nature {
    /// Le compte est au plan mais n'est plus éligible. `avant`/`apres` portent
    /// le libellé lisible, pas le drapeau : « CTC prêt » → « CTC non prêt » se
    /// lit, `true` → `false` non.
    EligibilitePerdue { avant: String, apres: String },
    DisparuDuFichier,
    JourChange { avant: u8, apres: u8 },
    PlateformeChangee { avant: String, apres: String },
}

/// Ce qu'on fait de l'écart. Séparé de `Nature` : la même nature ne donne pas
/// la même action selon que la ligne est gelée.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Action {
    Retirer { motif: String },
    Deplacer {
        run_num: String,
        #[serde(serialize_with = "date_iso")]
        run_date: chrono::NaiveDate,
        mep_id: usize,
        #[serde(serialize_with = "date_iso")]
        mep_date: chrono::NaiveDate,
    },
    /// Le champ est corrigé, la ligne ne bouge pas.
    Rafraichir,
    /// Vu, rien d'automatique — l'utilisateur tranche avec les outils existants.
    Signaler,
}

/// Les dates partent en ISO dans le JSON, comme partout ailleurs
/// (`plan::DetailRun`, `timeline`), mais restent des `NaiveDate` en interne :
/// `appliquer` les affecte telles quelles, sans reparser ce que ce module
/// vient de produire. `chrono` est compilé sans sa feature `serde`, la
/// conversion est donc explicite.
fn date_iso<S: serde::Serializer>(d: &chrono::NaiveDate, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&d.to_string())
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Ecart {
    pub cf: String,
    pub nature: Nature,
    pub action: Action,
    /// MEP déjà passée : le fichier a été transmis, et ils sont cumulatifs.
    pub gelee: bool,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct Rapprochement {
    pub ecarts: Vec<Ecart>,
    /// Lignes qu'aucun écart ne concerne. Une ligne dont seuls l'adressage ou
    /// la raison sociale ont changé en fait partie : ces champs sont rafraîchis
    /// sans produire d'écart.
    pub inchangees: usize,
    /// Avertissements **dérivés du calcul**. Ceux qui dépendent de l'état de la
    /// base sont ajoutés par la commande, qui seule y a accès.
    pub avertissements: Vec<String>,
}

/// Rapproche le plan du fichier courant. Ne décide rien qu'on ne puisse
/// expliquer : chaque écart porte sa nature ET son action.
pub fn calculer(
    plan: &[LignePlan],
    entrees: &[LigneEntree],
    runs: &[RunFacturation],
    meps: &[chrono::NaiveDate],
    aujourdhui: chrono::NaiveDate,
) -> Result<Rapprochement, String> {
    let _ = (plan, entrees, runs, meps, aujourdhui);
    Ok(Rapprochement::default())
}
```

- [ ] **Étape 3 : écrire les tests des retraits**

À la fin de `rapprochement.rs` :

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::Origine;
    use chrono::NaiveDate;

    pub(super) fn d(iso: &str) -> NaiveDate {
        NaiveDate::parse_from_str(iso, "%Y-%m-%d").expect("date de test valide")
    }

    /// Entrée « tout va bien » : éligible de bout en bout.
    pub(super) fn entree(cf: &str, jj: &str, pa: &str) -> LigneEntree {
        LigneEntree {
            cf: cf.into(),
            participant: format!("iso6523-actorid-upis::0225:{cf}"),
            jj_brut: jj.into(),
            raison_sociale: "ACME".into(),
            pa: pa.into(),
            resolu: true,
            ctc_ready: true,
            ctc_status: "ready".into(),
            ppf_usable: true,
            in_directory: true,
            resolved_at: 1_700_000_000,
        }
    }

    /// Pose les deux champs CTC ensemble : l'invariant de production est
    /// `ctc_ready == (ctc_status == "ready")`, une fixture ne doit pas pouvoir
    /// le violer.
    pub(super) fn avec_ctc(mut e: LigneEntree, statut: &str) -> LigneEntree {
        e.ctc_status = statut.into();
        e.ctc_ready = statut == "ready";
        e
    }

    /// Ligne du plan cohérente avec `entree(cf, jj, pa)`.
    pub(super) fn ligne(cf: &str, jj: u8, pa: &str, run: &str, mep: (usize, &str)) -> LignePlan {
        LignePlan {
            cf: cf.into(),
            participant: format!("iso6523-actorid-upis::0225:{cf}"),
            jj,
            raison_sociale: "ACME".into(),
            pa: pa.into(),
            mep_id: mep.0,
            mep_date: d(mep.1),
            run_num: run.into(),
            run_date: d("2026-09-10"),
            origine: Origine::Auto,
            in_directory: true,
            resolved_at: 1_700_000_000,
            planned_at: 1_700_000_000,
            retire: None,
        }
    }

    pub(super) fn run(num: &str, date: &str, jjs: &[u8]) -> RunFacturation {
        RunFacturation {
            num: num.into(),
            date: d(date),
            jjs: jjs.to_vec(),
            exclu: false,
        }
    }

    /// Aujourd'hui = 2026-08-01. La MEP 1 (2026-07-01) est donc passée, la
    /// MEP 2 (2026-09-01) à venir.
    pub(super) fn contexte() -> (Vec<RunFacturation>, Vec<NaiveDate>, NaiveDate) {
        let runs = vec![
            run("RF01", "2026-09-10", &[1, 5]),
            run("RF02", "2026-09-20", &[12, 22]),
        ];
        let meps = vec![d("2026-07-01"), d("2026-09-01")];
        (runs, meps, d("2026-08-01"))
    }

    #[test]
    fn un_compte_devenu_ctc_non_pret_est_propose_au_retrait() {
        let (runs, meps, auj) = contexte();
        let plan = vec![ligne("CF1", 5, "Cegedim", "RF01", (2, "2026-09-01"))];
        let entrees = vec![avec_ctc(entree("CF1", "5", "Cegedim"), "later")];
        let r = calculer(&plan, &entrees, &runs, &meps, auj).unwrap();
        assert_eq!(r.ecarts.len(), 1);
        assert_eq!(r.ecarts[0].cf, "CF1");
        assert!(matches!(r.ecarts[0].nature, Nature::EligibilitePerdue { .. }));
        let Action::Retirer { motif } = &r.ecarts[0].action else {
            panic!("attendu un retrait, obtenu {:?}", r.ecarts[0].action);
        };
        assert!(motif.contains("CTC"), "le motif doit nommer la cause : {motif}");
    }

    #[test]
    fn un_compte_devenu_ppf_non_utilisable_est_propose_au_retrait() {
        let (runs, meps, auj) = contexte();
        let plan = vec![ligne("CF1", 5, "Cegedim", "RF01", (2, "2026-09-01"))];
        let mut e = entree("CF1", "5", "Cegedim");
        e.ppf_usable = false;
        let r = calculer(&plan, &[e], &runs, &meps, auj).unwrap();
        let Action::Retirer { motif } = &r.ecarts[0].action else {
            panic!("attendu un retrait");
        };
        assert!(motif.contains("PPF"), "le motif doit nommer la cause : {motif}");
    }

    /// Motif distinct du précédent : « disparu » et « inéligible » ne
    /// s'expliquent pas pareil six mois plus tard.
    #[test]
    fn un_compte_absent_du_fichier_est_propose_au_retrait() {
        let (runs, meps, auj) = contexte();
        let plan = vec![ligne("CF1", 5, "Cegedim", "RF01", (2, "2026-09-01"))];
        let entrees: Vec<LigneEntree> = vec![];
        let r = calculer(&plan, &entrees, &runs, &meps, auj).unwrap();
        assert_eq!(r.ecarts.len(), 1);
        assert_eq!(r.ecarts[0].nature, Nature::DisparuDuFichier);
        assert!(matches!(r.ecarts[0].action, Action::Retirer { .. }));
    }

    #[test]
    fn un_compte_inchange_ne_produit_aucun_ecart() {
        let (runs, meps, auj) = contexte();
        let plan = vec![ligne("CF1", 5, "Cegedim", "RF01", (2, "2026-09-01"))];
        let entrees = vec![entree("CF1", "5", "Cegedim")];
        let r = calculer(&plan, &entrees, &runs, &meps, auj).unwrap();
        assert!(r.ecarts.is_empty(), "obtenu {:?}", r.ecarts);
        assert_eq!(r.inchangees, 1);
    }

    #[test]
    fn une_ligne_deja_retiree_est_ignoree() {
        let (runs, meps, auj) = contexte();
        let mut l = ligne("CF1", 5, "Cegedim", "RF01", (2, "2026-09-01"));
        l.retire = Some(crate::plan::Retrait { le: 1, motif: "essai".into() });
        let entrees: Vec<LigneEntree> = vec![]; // disparue, et pourtant ignorée
        let r = calculer(&[l], &entrees, &runs, &meps, auj).unwrap();
        assert!(r.ecarts.is_empty(), "une ligne retirée est déjà hors jeu");
        assert_eq!(r.inchangees, 0, "elle n'est pas non plus comptée inchangée");
    }

    #[test]
    fn un_doublon_de_cf_avec_deux_jj_est_refuse() {
        let (runs, meps, auj) = contexte();
        let plan = vec![ligne("CF1", 5, "Cegedim", "RF01", (2, "2026-09-01"))];
        let entrees = vec![entree("CF1", "5", "Cegedim"), entree("CF1", "12", "Cegedim")];
        let err = calculer(&plan, &entrees, &runs, &meps, auj).unwrap_err();
        assert!(err.contains("deux jours de cycle"), "obtenu : {err}");
    }
}
```

- [ ] **Étape 4 : lancer, vérifier l'échec**

```bash
cd client/src-tauri && cargo test rapprochement 2>&1 | tail -25
```

Attendu : les six tests échouent (`assertion failed`, écarts vides), sauf
`un_compte_inchange_ne_produit_aucun_ecart` qui échoue sur `inchangees == 0`.

- [ ] **Étape 5 : implémenter les retraits**

Remplacer le corps de `calculer` :

```rust
pub fn calculer(
    plan: &[LignePlan],
    entrees: &[LigneEntree],
    runs: &[RunFacturation],
    meps: &[chrono::NaiveDate],
    aujourdhui: chrono::NaiveDate,
) -> Result<Rapprochement, String> {
    let _ = (runs, meps);
    let par_cf: HashMap<&str, &LigneEntree> = crate::plan::dedoublonner(entrees)?
        .into_iter()
        .map(|e| (e.cf.as_str(), e))
        .collect();

    let mut r = Rapprochement::default();
    for l in plan {
        // Une ligne retirée est déjà hors des fichiers, des comptages et du
        // re-tirage : la rapprocher n'aurait aucun effet observable.
        if l.retiree() {
            continue;
        }
        let gelee = l.gelee(aujourdhui);
        let Some(e) = par_cf.get(l.cf.as_str()) else {
            r.ecarts.push(Ecart {
                cf: l.cf.clone(),
                nature: Nature::DisparuDuFichier,
                action: Action::Retirer { motif: String::new() },
                gelee,
            });
            continue;
        };
        if !e.ctc_ready || !e.ppf_usable {
            let (avant, apres) = if !e.ctc_ready {
                ("CTC prêt".to_string(), format!("CTC {}", libelle_ctc(&e.ctc_status)))
            } else {
                ("PPF utilisable".to_string(), "PPF non utilisable".to_string())
            };
            r.ecarts.push(Ecart {
                cf: l.cf.clone(),
                // `apres` sert deux fois : cloné pour la nature, déplacé dans
                // le motif. L'ordre inverse ne compilerait pas.
                nature: Nature::EligibilitePerdue { avant, apres: apres.clone() },
                action: Action::Retirer { motif: apres },
                gelee,
            });
            continue;
        }
        r.inchangees += 1;
    }
    Ok(r)
}

/// Libellé français d'un statut CTC. Vide = jamais résolu, ce qui n'est pas la
/// même chose que « pas prêt ».
fn libelle_ctc(statut: &str) -> &'static str {
    match statut {
        "later" => "prêt plus tard",
        "expired" => "expiré",
        "" => "non résolu",
        _ => "non prêt",
    }
}
```

Les motifs ne sont pas vides : les tests vérifient qu'ils nomment la cause.
Poser dès maintenant, dans la branche « disparu » :

```rust
                action: Action::Retirer { motif: "absent du fichier".into() },
```

et dans la branche « éligibilité perdue », après le calcul de `apres` :

```rust
                action: Action::Retirer { motif: apres.clone() },
```

La tâche 4 les préfixera par la date du rapprochement.

- [ ] **Étape 6 : lancer, vérifier le succès**

```bash
cd client/src-tauri && cargo test rapprochement 2>&1 | tail -25
```

Attendu : `test result: ok. 6 passed`.

- [ ] **Étape 7 : commit**

```bash
git add client/src-tauri/src/rapprochement.rs client/src-tauri/src/lib.rs
git commit -m "feat(superpopaul): rapprochement — retraits des comptes inéligibles et disparus"
```

---

## Tâche 3 : le déplacement et le choix du run cible

**Fichiers :**
- Modifier : `client/src-tauri/src/rapprochement.rs`

- [ ] **Étape 1 : écrire les tests du déplacement**

Dans `mod tests`, après les tests de retrait :

```rust
    /// Moindre perturbation : le compte reste dans le MÊME lot, seul son
    /// ordonnancement change.
    #[test]
    fn le_jj_change_prefere_un_run_de_la_meme_mep() {
        // RF01 (10/09) et RF02 (20/09) dépendent tous deux de la MEP 2.
        let (runs, meps, auj) = contexte();
        let plan = vec![ligne("CF1", 5, "Cegedim", "RF01", (2, "2026-09-01"))];
        let entrees = vec![entree("CF1", "12", "Cegedim")];
        let r = calculer(&plan, &entrees, &runs, &meps, auj).unwrap();
        assert_eq!(r.ecarts.len(), 1);
        assert_eq!(r.ecarts[0].nature, Nature::JourChange { avant: 5, apres: 12 });
        let Action::Deplacer { run_num, mep_id, .. } = &r.ecarts[0].action else {
            panic!("attendu un déplacement, obtenu {:?}", r.ecarts[0].action);
        };
        assert_eq!(run_num, "RF02", "seul RF02 couvre le jour 12");
        assert_eq!(*mep_id, 2, "la MEP ne change pas");
    }

    /// Quand plusieurs runs conviennent, celui de la MEP courante l'emporte —
    /// même s'il est plus tardif.
    #[test]
    fn a_mep_egale_le_run_de_la_mep_courante_prime_sur_le_plus_proche() {
        let runs = vec![
            run("RF01", "2026-08-10", &[12]), // MEP 1, plus tôt
            run("RF02", "2026-09-20", &[12]), // MEP 2, celle de la ligne
        ];
        let meps = vec![d("2026-07-01"), d("2026-09-01")];
        let plan = vec![ligne("CF1", 5, "Cegedim", "RF09", (2, "2026-09-01"))];
        let entrees = vec![entree("CF1", "12", "Cegedim")];
        let r = calculer(&plan, &entrees, &runs, &meps, d("2026-08-01")).unwrap();
        let Action::Deplacer { run_num, .. } = &r.ecarts[0].action else {
            panic!("attendu un déplacement");
        };
        assert_eq!(run_num, "RF02", "la MEP de rattachement prime sur la date");
    }

    #[test]
    fn sans_run_a_la_meme_mep_la_mep_la_plus_proche_est_prise() {
        let runs = vec![
            run("RF01", "2026-09-10", &[1, 5]),
            run("RF02", "2026-10-05", &[12]), // MEP 3
        ];
        let meps = vec![d("2026-07-01"), d("2026-09-01"), d("2026-10-01")];
        let plan = vec![ligne("CF1", 5, "Cegedim", "RF01", (2, "2026-09-01"))];
        let entrees = vec![entree("CF1", "12", "Cegedim")];
        let r = calculer(&plan, &entrees, &runs, &meps, d("2026-08-01")).unwrap();
        let Action::Deplacer { run_num, mep_id, .. } = &r.ecarts[0].action else {
            panic!("attendu un déplacement");
        };
        assert_eq!(run_num, "RF02");
        assert_eq!(*mep_id, 3, "le lot change, faute de mieux");
    }

    #[test]
    fn le_jj_change_sans_run_compatible_est_signale_pas_deplace() {
        let (runs, meps, auj) = contexte(); // couvre 1, 5, 12, 22
        let plan = vec![ligne("CF1", 5, "Cegedim", "RF01", (2, "2026-09-01"))];
        let entrees = vec![entree("CF1", "17", "Cegedim")];
        let r = calculer(&plan, &entrees, &runs, &meps, auj).unwrap();
        assert_eq!(r.ecarts[0].nature, Nature::JourChange { avant: 5, apres: 17 });
        assert_eq!(r.ecarts[0].action, Action::Signaler);
    }

    /// Un run passé ferait basculer la ligne dans le gel avec effet
    /// rétroactif : un lot livré changerait après coup.
    #[test]
    fn un_run_deja_passe_n_est_jamais_choisi_comme_cible() {
        let runs = vec![run("RF01", "2026-07-10", &[12])]; // avant aujourd'hui
        let meps = vec![d("2026-07-01")];
        let plan = vec![ligne("CF1", 5, "Cegedim", "RF09", (1, "2026-07-01"))];
        let entrees = vec![entree("CF1", "12", "Cegedim")];
        let r = calculer(&plan, &entrees, &runs, &meps, d("2026-08-01")).unwrap();
        assert_eq!(
            r.ecarts[0].action,
            Action::Signaler,
            "le seul run compatible est passé : rien à faire automatiquement"
        );
    }

    /// Un jour de cycle illisible dans le fichier n'est pas un changement :
    /// c'est une donnée qu'on ne sait pas lire.
    #[test]
    fn un_jj_illisible_dans_le_fichier_est_signale_sans_deplacement() {
        let (runs, meps, auj) = contexte();
        let plan = vec![ligne("CF1", 5, "Cegedim", "RF01", (2, "2026-09-01"))];
        let entrees = vec![entree("CF1", "n/a", "Cegedim")];
        let r = calculer(&plan, &entrees, &runs, &meps, auj).unwrap();
        assert_eq!(r.ecarts.len(), 1, "obtenu {:?}", r.ecarts);
        assert_eq!(r.ecarts[0].action, Action::Signaler);
    }
```

- [ ] **Étape 2 : lancer, vérifier l'échec**

```bash
cd client/src-tauri && cargo test rapprochement 2>&1 | tail -30
```

Attendu : les six nouveaux tests échouent — `calculer` compte encore ces
lignes comme inchangées.

- [ ] **Étape 3 : rendre `parse_jj` visible au module**

Dans `plan.rs`, `parse_jj` est `pub(crate)` — rien à changer, elle est déjà
accessible depuis `rapprochement`.

- [ ] **Étape 4 : implémenter le choix du run et la branche du déplacement**

Ajouter dans `rapprochement.rs`, après `libelle_ctc` :

```rust
/// Run cible pour un compte qui a changé de jour de cycle.
///
/// Moindre perturbation : la MEP la plus proche de celle où la ligne se trouve
/// déjà — distance nulle pour la MEP courante, qui l'emporte donc d'office et
/// laisse le compte dans son lot.
///
/// **Double garde temporelle.** Ni un run déjà passé, ni un run dont la MEP de
/// rattachement est passée. La seconde n'est pas redondante : `mep_de` rattache
/// un run à la dernière MEP qui le précède, donc un run futur peut porter une
/// MEP passée. Sans elle, la ligne déplacée deviendrait gelée sur-le-champ —
/// réputée appartenir à un lot déjà livré.
fn run_cible<'a>(
    jj: u8,
    mep_actuelle: usize,
    runs: &'a [RunFacturation],
    meps: &[chrono::NaiveDate],
    aujourdhui: chrono::NaiveDate,
) -> Option<(&'a RunFacturation, usize, chrono::NaiveDate)> {
    let mut candidats: Vec<(&RunFacturation, usize, chrono::NaiveDate)> = runs
        .iter()
        .filter(|r| r.couvre(jj) && r.date >= aujourdhui)
        .filter_map(|r| crate::calendrier::mep_de(r.date, meps).map(|(id, date)| (r, id, date)))
        .filter(|(_, _, mep_date)| *mep_date >= aujourdhui)
        .collect();
    // Distance à la MEP courante, puis date de run pour départager. Pas de
    // booléen « même MEP » en tête : une distance nulle est déjà le minimum.
    candidats.sort_by_key(|(r, id, _)| (id.abs_diff(mep_actuelle), r.date));
    candidats.into_iter().next()
}
```

Dans `calculer`, remplacer `let _ = (runs, meps);` par rien, et insérer après
la branche d'éligibilité (avant `r.inchangees += 1;`) :

```rust
        let jj_fichier = crate::plan::parse_jj(&e.jj_brut);
        if jj_fichier != Some(l.jj) {
            // Un jour illisible n'est pas un changement : c'est une donnée
            // qu'on ne sait pas lire. On le signale sans rien décider.
            let Some(neuf) = jj_fichier else {
                r.ecarts.push(Ecart {
                    cf: l.cf.clone(),
                    nature: Nature::JourChange { avant: l.jj, apres: 0 },
                    action: Action::Signaler,
                    gelee,
                });
                continue;
            };
            let action = match run_cible(neuf, l.mep_id, runs, meps, aujourdhui) {
                Some((run, mep_id, mep_date)) => Action::Deplacer {
                    run_num: run.num.clone(),
                    run_date: run.date,
                    mep_id,
                    mep_date,
                },
                None => Action::Signaler,
            };
            r.ecarts.push(Ecart {
                cf: l.cf.clone(),
                nature: Nature::JourChange { avant: l.jj, apres: neuf },
                action,
                gelee,
            });
            continue;
        }
```

- [ ] **Étape 5 : lancer, vérifier le succès**

```bash
cd client/src-tauri && cargo test rapprochement 2>&1 | tail -30
```

Attendu : `test result: ok. 12 passed`.

- [ ] **Étape 6 : commit**

```bash
git add client/src-tauri/src/rapprochement.rs
git commit -m "feat(superpopaul): rapprochement — déplacement vers un run de moindre perturbation"
```

---

## Tâche 4 : lignes gelées, plateforme, ordre de résolution, motifs datés

**Fichiers :**
- Modifier : `client/src-tauri/src/rapprochement.rs`

- [ ] **Étape 1 : écrire les tests**

```rust
    #[test]
    fn une_ligne_gelee_au_jj_change_est_signalee_jamais_deplacee() {
        let (runs, meps, auj) = contexte();
        // MEP 1 = 2026-07-01, passée au 2026-08-01.
        let plan = vec![ligne("CF1", 5, "Cegedim", "RF01", (1, "2026-07-01"))];
        let entrees = vec![entree("CF1", "12", "Cegedim")];
        let r = calculer(&plan, &entrees, &runs, &meps, auj).unwrap();
        assert!(r.ecarts[0].gelee);
        assert_eq!(
            r.ecarts[0].action,
            Action::Signaler,
            "sortir un compte d'un lot livré pour l'insérer dans un autre n'est autorisé nulle part"
        );
    }

    #[test]
    fn une_ligne_gelee_devenue_ineligible_est_proposee_au_retrait_et_marquee() {
        let (runs, meps, auj) = contexte();
        let plan = vec![ligne("CF1", 5, "Cegedim", "RF01", (1, "2026-07-01"))];
        let entrees = vec![avec_ctc(entree("CF1", "5", "Cegedim"), "expired")];
        let r = calculer(&plan, &entrees, &runs, &meps, auj).unwrap();
        assert!(matches!(r.ecarts[0].action, Action::Retirer { .. }));
        assert!(r.ecarts[0].gelee, "l'IHM doit pouvoir l'isoler et avertir");
    }

    #[test]
    fn un_changement_de_plateforme_rafraichit_sans_deplacer() {
        let (runs, meps, auj) = contexte();
        let plan = vec![ligne("CF1", 5, "Cegedim", "RF01", (2, "2026-09-01"))];
        let entrees = vec![entree("CF1", "5", "Esker")];
        let r = calculer(&plan, &entrees, &runs, &meps, auj).unwrap();
        assert_eq!(
            r.ecarts[0].nature,
            Nature::PlateformeChangee { avant: "Cegedim".into(), apres: "Esker".into() }
        );
        assert_eq!(r.ecarts[0].action, Action::Rafraichir);
    }

    /// Ordre de résolution : retrait > déplacement > rafraîchissement.
    #[test]
    fn un_compte_a_retirer_n_est_pas_aussi_deplace() {
        let (runs, meps, auj) = contexte();
        let plan = vec![ligne("CF1", 5, "Cegedim", "RF01", (2, "2026-09-01"))];
        // Tout a changé d'un coup : inéligible, jour 12, plateforme Esker.
        let entrees = vec![avec_ctc(entree("CF1", "12", "Esker"), "later")];
        let r = calculer(&plan, &entrees, &runs, &meps, auj).unwrap();
        assert_eq!(r.ecarts.len(), 1, "un compte, un écart : {:?}", r.ecarts);
        assert!(matches!(r.ecarts[0].action, Action::Retirer { .. }));
    }

    #[test]
    fn le_jj_prime_sur_la_plateforme() {
        let (runs, meps, auj) = contexte();
        let plan = vec![ligne("CF1", 5, "Cegedim", "RF01", (2, "2026-09-01"))];
        let entrees = vec![entree("CF1", "12", "Esker")];
        let r = calculer(&plan, &entrees, &runs, &meps, auj).unwrap();
        assert_eq!(r.ecarts.len(), 1);
        assert!(matches!(r.ecarts[0].nature, Nature::JourChange { .. }));
    }

    /// Le rapprochement n'ajoute RIEN : c'est la garantie qu'aucun re-tirage
    /// ne s'est glissé là.
    #[test]
    fn aucun_compte_eligible_hors_plan_n_est_ajoute() {
        let (runs, meps, auj) = contexte();
        let plan = vec![ligne("CF1", 5, "Cegedim", "RF01", (2, "2026-09-01"))];
        let entrees = vec![
            entree("CF1", "5", "Cegedim"),
            entree("CF2", "12", "Esker"), // éligible, jamais planifié
        ];
        let r = calculer(&plan, &entrees, &runs, &meps, auj).unwrap();
        assert!(r.ecarts.is_empty(), "obtenu {:?}", r.ecarts);
        assert_eq!(r.inchangees, 1, "CF2 n'entre pas dans le décompte du plan");
    }

    /// Un adressage ou une raison sociale qui change n'est pas un écart : sans
    /// effet sur le placement, il n'a rien à faire valider.
    #[test]
    fn un_adressage_change_ne_produit_aucun_ecart() {
        let (runs, meps, auj) = contexte();
        let plan = vec![ligne("CF1", 5, "Cegedim", "RF01", (2, "2026-09-01"))];
        let mut e = entree("CF1", "5", "Cegedim");
        e.participant = "iso6523-actorid-upis::0225:NOUVEAU".into();
        e.raison_sociale = "ACME SAS".into();
        let r = calculer(&plan, &[e], &runs, &meps, auj).unwrap();
        assert!(r.ecarts.is_empty(), "obtenu {:?}", r.ecarts);
        assert_eq!(r.inchangees, 1);
    }

    #[test]
    fn les_motifs_de_retrait_portent_la_date_du_rapprochement() {
        let (runs, meps, auj) = contexte();
        let plan = vec![ligne("CF1", 5, "Cegedim", "RF01", (2, "2026-09-01"))];
        let entrees = vec![avec_ctc(entree("CF1", "5", "Cegedim"), "later")];
        let r = calculer(&plan, &entrees, &runs, &meps, auj).unwrap();
        let Action::Retirer { motif } = &r.ecarts[0].action else {
            panic!("attendu un retrait");
        };
        assert!(
            motif.contains("01/08/2026"),
            "un motif sans date est ingérable six mois plus tard : {motif}"
        );
    }

    /// Au-delà du quart des lignes actives retirées, l'ampleur doit être dite.
    #[test]
    fn un_rapprochement_massif_produit_un_avertissement() {
        let (runs, meps, auj) = contexte();
        let plan: Vec<LignePlan> = (0..4)
            .map(|i| ligne(&format!("CF{i}"), 5, "Cegedim", "RF01", (2, "2026-09-01")))
            .collect();
        // 2 sur 4 retirés = la moitié.
        let entrees = vec![
            avec_ctc(entree("CF0", "5", "Cegedim"), "later"),
            avec_ctc(entree("CF1", "5", "Cegedim"), "later"),
            entree("CF2", "5", "Cegedim"),
            entree("CF3", "5", "Cegedim"),
        ];
        let r = calculer(&plan, &entrees, &runs, &meps, auj).unwrap();
        assert!(
            r.avertissements.iter().any(|a| a.contains("2 des 4")),
            "obtenu {:?}",
            r.avertissements
        );
    }

    /// Les quotas par plateforme ne sont PAS rejoués — ce serait du
    /// re-tirage. L'écart qu'ils prennent doit donc être dit, sinon la
    /// répartition affichée ailleurs devient fausse en silence.
    #[test]
    fn un_changement_de_plateforme_avertit_du_decalage_de_repartition() {
        let (runs, meps, auj) = contexte();
        let plan = vec![
            ligne("CF1", 5, "Cegedim", "RF01", (2, "2026-09-01")),
            ligne("CF2", 22, "Cegedim", "RF02", (2, "2026-09-01")),
        ];
        let entrees = vec![entree("CF1", "5", "Esker"), entree("CF2", "22", "Cegedim")];
        let r = calculer(&plan, &entrees, &runs, &meps, auj).unwrap();
        let a = r
            .avertissements
            .iter()
            .find(|a| a.contains("plateforme"))
            .unwrap_or_else(|| panic!("obtenu {:?}", r.avertissements));
        assert!(a.contains("Cegedim") && a.contains("Esker"), "obtenu : {a}");
    }

    #[test]
    fn sans_changement_de_plateforme_aucun_avertissement_de_repartition() {
        let (runs, meps, auj) = contexte();
        let plan = vec![ligne("CF1", 5, "Cegedim", "RF01", (2, "2026-09-01"))];
        let entrees = vec![entree("CF1", "5", "Cegedim")];
        let r = calculer(&plan, &entrees, &runs, &meps, auj).unwrap();
        assert!(r.avertissements.is_empty(), "obtenu {:?}", r.avertissements);
    }

    #[test]
    fn un_rapprochement_modeste_ne_produit_pas_d_avertissement_d_ampleur() {
        let (runs, meps, auj) = contexte();
        let plan: Vec<LignePlan> = (0..8)
            .map(|i| ligne(&format!("CF{i}"), 5, "Cegedim", "RF01", (2, "2026-09-01")))
            .collect();
        let mut entrees: Vec<LigneEntree> = (0..8)
            .map(|i| entree(&format!("CF{i}"), "5", "Cegedim"))
            .collect();
        entrees[0] = avec_ctc(entrees[0].clone(), "later"); // 1 sur 8
        let r = calculer(&plan, &entrees, &runs, &meps, auj).unwrap();
        assert!(r.avertissements.is_empty(), "obtenu {:?}", r.avertissements);
    }
```

- [ ] **Étape 2 : lancer, vérifier l'échec**

```bash
cd client/src-tauri && cargo test rapprochement 2>&1 | tail -35
```

Attendu : échecs sur le gel du déplacement, la plateforme, les motifs datés et
les avertissements d'ampleur.

- [ ] **Étape 3 : implémenter**

Dans `calculer`, remplacer les motifs vides par des motifs datés. En tête de
fonction, après la construction de `par_cf` :

```rust
    let stamp = format!("Rapprochement du {}", aujourdhui.format("%d/%m/%Y"));
```

Branche « disparu » :

```rust
            r.ecarts.push(Ecart {
                cf: l.cf.clone(),
                nature: Nature::DisparuDuFichier,
                action: Action::Retirer {
                    motif: format!("{stamp} — absent du fichier"),
                },
                gelee,
            });
```

Branche « éligibilité perdue » :

```rust
            r.ecarts.push(Ecart {
                cf: l.cf.clone(),
                nature: Nature::EligibilitePerdue { avant, apres: apres.clone() },
                action: Action::Retirer {
                    motif: format!("{stamp} — {apres}"),
                },
                gelee,
            });
```

Dans la branche du jour changé, court-circuiter le gel : remplacer le calcul
de `action` par

```rust
            let action = if gelee {
                // Sortir un compte d'un lot livré pour l'insérer dans un autre
                // n'est autorisé nulle part. Signalé, pas traité.
                Action::Signaler
            } else {
                match run_cible(neuf, l.mep_id, runs, meps, aujourdhui) {
                    Some((run, mep_id, mep_date)) => Action::Deplacer {
                        run_num: run.num.clone(),
                        run_date: run.date,
                        mep_id,
                        mep_date,
                    },
                    None => Action::Signaler,
                }
            };
```

Ajouter la branche plateforme juste avant `r.inchangees += 1;` :

```rust
        if e.pa != l.pa {
            r.ecarts.push(Ecart {
                cf: l.cf.clone(),
                nature: Nature::PlateformeChangee {
                    avant: l.pa.clone(),
                    apres: e.pa.clone(),
                },
                action: Action::Rafraichir,
                gelee,
            });
            continue;
        }
```

Et l'avertissement d'ampleur, après la boucle :

```rust
    // Seuil chiffré plutôt qu'un jugement : « beaucoup » ne se teste pas.
    let retraits = r
        .ecarts
        .iter()
        .filter(|e| matches!(e.action, Action::Retirer { .. }))
        .count();
    let actives = plan.iter().filter(|l| !l.retiree()).count();
    if actives > 0 && retraits * 4 > actives {
        r.avertissements.push(format!(
            "ce rapprochement retire {retraits} des {actives} lignes actives du plan"
        ));
    }

    // Les quotas par plateforme ne sont pas rejoués — ce serait du re-tirage.
    // Le décalage qu'ils prennent doit donc être dit, chiffres à l'appui :
    // sinon la répartition affichée ailleurs devient fausse en silence.
    let mut mouvements: std::collections::BTreeMap<(&str, &str), usize> = Default::default();
    for e in &r.ecarts {
        if let Nature::PlateformeChangee { avant, apres } = &e.nature {
            *mouvements.entry((avant.as_str(), apres.as_str())).or_insert(0) += 1;
        }
    }
    if !mouvements.is_empty() {
        let detail: Vec<String> = mouvements
            .iter()
            .map(|((a, b), n)| format!("{n} de {a} vers {b}"))
            .collect();
        r.avertissements.push(format!(
            "la répartition par plateforme change sans être rejouée : {}",
            detail.join(", ")
        ));
    }
```

- [ ] **Étape 4 : lancer, vérifier le succès**

```bash
cd client/src-tauri && cargo test 2>&1 | tail -25
```

Attendu : `test result: ok`, 22 tests dans `rapprochement`, aucune régression
ailleurs.

- [ ] **Étape 5 : commit**

```bash
git add client/src-tauri/src/rapprochement.rs
git commit -m "feat(superpopaul): rapprochement — gel, plateforme, ordre de résolution, motifs datés"
```

---

## Tâche 5 : `appliquer`

**Fichiers :**
- Modifier : `client/src-tauri/src/rapprochement.rs`

- [ ] **Étape 1 : écrire les tests**

```rust
    /// La régression la plus insidieuse : épingler les lignes déplacées les
    /// soustrairait à TOUTES les régénérations futures, et le plan se figerait
    /// un peu plus à chaque rapprochement, sans que rien ne le dise.
    #[test]
    fn appliquer_ne_change_pas_l_origine_des_lignes_deplacees() {
        let (runs, meps, auj) = contexte();
        let mut plan = vec![ligne("CF1", 5, "Cegedim", "RF01", (2, "2026-09-01"))];
        let entrees = vec![entree("CF1", "12", "Cegedim")];
        let r = calculer(&plan, &entrees, &runs, &meps, auj).unwrap();
        appliquer(&mut plan, &r, 1_800_000_000).unwrap();
        assert_eq!(
            plan[0].origine,
            Origine::Auto,
            "un rapprochement corrige une donnée, il ne change pas la provenance"
        );
    }

    #[test]
    fn appliquer_met_a_jour_le_jour_et_le_run() {
        let (runs, meps, auj) = contexte();
        let mut plan = vec![ligne("CF1", 5, "Cegedim", "RF01", (2, "2026-09-01"))];
        let entrees = vec![entree("CF1", "12", "Cegedim")];
        let r = calculer(&plan, &entrees, &runs, &meps, auj).unwrap();
        appliquer(&mut plan, &r, 1_800_000_000).unwrap();
        assert_eq!(plan[0].jj, 12, "sans le jour, le déplacement ne sert à rien");
        assert_eq!(plan[0].run_num, "RF02");
        assert_eq!(plan[0].run_date, d("2026-09-20"));
        assert_eq!(plan[0].mep_id, 2);
    }

    /// L'invariant central du chantier.
    #[test]
    fn appliquer_laisse_les_lignes_inchangees_champ_pour_champ() {
        let (runs, meps, auj) = contexte();
        let mut plan = vec![
            ligne("CF1", 5, "Cegedim", "RF01", (2, "2026-09-01")),
            ligne("CF2", 22, "Esker", "RF02", (2, "2026-09-01")),
        ];
        let temoin = plan[1].clone();
        let entrees = vec![
            avec_ctc(entree("CF1", "5", "Cegedim"), "later"),
            entree("CF2", "22", "Esker"),
        ];
        let r = calculer(&plan, &entrees, &runs, &meps, auj).unwrap();
        appliquer(&mut plan, &r, 1_800_000_000).unwrap();
        assert_eq!(plan[1], temoin, "CF2 n'a aucune raison d'avoir bougé");
    }

    #[test]
    fn appliquer_marque_le_retrait_sans_supprimer_la_ligne() {
        let (runs, meps, auj) = contexte();
        let mut plan = vec![ligne("CF1", 5, "Cegedim", "RF01", (2, "2026-09-01"))];
        let entrees: Vec<LigneEntree> = vec![];
        let r = calculer(&plan, &entrees, &runs, &meps, auj).unwrap();
        appliquer(&mut plan, &r, 1_800_000_000).unwrap();
        assert_eq!(plan.len(), 1, "un retrait marque, il ne supprime pas");
        let retrait = plan[0].retire.as_ref().expect("la ligne doit porter un retrait");
        assert_eq!(retrait.le, 1_800_000_000);
        assert!(retrait.motif.contains("absent du fichier"));
    }

    #[test]
    fn appliquer_rafraichit_la_plateforme() {
        let (runs, meps, auj) = contexte();
        let mut plan = vec![ligne("CF1", 5, "Cegedim", "RF01", (2, "2026-09-01"))];
        let entrees = vec![entree("CF1", "5", "Esker")];
        let r = calculer(&plan, &entrees, &runs, &meps, auj).unwrap();
        appliquer(&mut plan, &r, 1_800_000_000).unwrap();
        assert_eq!(plan[0].pa, "Esker");
        assert_eq!(plan[0].run_num, "RF01", "le rafraîchissement ne déplace pas");
    }

    #[test]
    fn appliquer_ne_touche_pas_aux_ecarts_signales() {
        let (runs, meps, auj) = contexte();
        let mut plan = vec![ligne("CF1", 5, "Cegedim", "RF01", (2, "2026-09-01"))];
        let temoin = plan[0].clone();
        let entrees = vec![entree("CF1", "17", "Cegedim")]; // aucun run ne couvre 17
        let r = calculer(&plan, &entrees, &runs, &meps, auj).unwrap();
        appliquer(&mut plan, &r, 1_800_000_000).unwrap();
        assert_eq!(plan[0], temoin, "signalé n'est pas traité");
    }

    /// Un rapprochement calculé sur un autre plan ne doit pas s'appliquer à
    /// moitié : tout est vérifié avant d'écrire quoi que ce soit.
    #[test]
    fn appliquer_refuse_un_ecart_dont_le_compte_est_absent_du_plan() {
        let mut plan = vec![ligne("CF1", 5, "Cegedim", "RF01", (2, "2026-09-01"))];
        let temoin = plan[0].clone();
        let r = Rapprochement {
            ecarts: vec![
                Ecart {
                    cf: "CF1".into(),
                    nature: Nature::DisparuDuFichier,
                    action: Action::Retirer { motif: "essai".into() },
                    gelee: false,
                },
                Ecart {
                    cf: "INCONNU".into(),
                    nature: Nature::DisparuDuFichier,
                    action: Action::Retirer { motif: "essai".into() },
                    gelee: false,
                },
            ],
            inchangees: 0,
            avertissements: vec![],
        };
        let err = appliquer(&mut plan, &r, 1_800_000_000).unwrap_err();
        assert!(err.contains("INCONNU"), "obtenu : {err}");
        assert_eq!(plan[0], temoin, "rien ne doit avoir été écrit");
    }
```

- [ ] **Étape 2 : lancer, vérifier l'échec**

```bash
cd client/src-tauri && cargo test rapprochement 2>&1 | tail -20
```

Attendu : ÉCHEC de compilation, `cannot find function 'appliquer'`.

- [ ] **Étape 3 : implémenter**

Ajouter dans `rapprochement.rs`, après `calculer` :

```rust
/// Applique un rapprochement au plan, **par mutation en place**. Aucune
/// ré-allocation n'est appelée : c'est ce qui garantit que le reste du plan ne
/// bouge pas.
///
/// Tout est vérifié avant d'écrire quoi que ce soit — comme `plan::ajouter` :
/// un lot à moitié appliqué serait pire qu'un refus.
pub fn appliquer(
    plan: &mut [LignePlan],
    r: &Rapprochement,
    maintenant: i64,
) -> Result<(), String> {
    let mut cibles = Vec::with_capacity(r.ecarts.len());
    for e in &r.ecarts {
        let i = plan
            .iter()
            .position(|l| l.cf == e.cf)
            .ok_or_else(|| format!("le compte « {} » n'est pas au plan", e.cf))?;
        cibles.push((i, e));
    }
    for (i, e) in cibles {
        let l = &mut plan[i];
        match &e.action {
            Action::Retirer { motif } => {
                l.retire = Some(crate::plan::Retrait {
                    le: maintenant,
                    motif: motif.clone(),
                });
            }
            Action::Deplacer { run_num, run_date, mep_id, mep_date } => {
                if let Nature::JourChange { apres, .. } = e.nature {
                    l.jj = apres;
                }
                l.run_num = run_num.clone();
                l.run_date = *run_date;
                l.mep_id = *mep_id;
                l.mep_date = *mep_date;
                // L'origine reste celle d'avant : un rapprochement corrige une
                // donnée périmée, il ne change pas la provenance de
                // l'affectation. L'épingler la soustrairait à toutes les
                // régénérations futures.
            }
            Action::Rafraichir => {
                if let Nature::PlateformeChangee { apres, .. } = &e.nature {
                    l.pa = apres.clone();
                }
            }
            Action::Signaler => {}
        }
    }
    Ok(())
}
```

- [ ] **Étape 4 : lancer, vérifier le succès**

```bash
cd client/src-tauri && cargo test 2>&1 | tail -25
```

Attendu : `test result: ok`, 29 tests dans `rapprochement`.

- [ ] **Étape 5 : commit**

```bash
git add client/src-tauri/src/rapprochement.rs
git commit -m "feat(superpopaul): rapprochement — application par mutation en place"
```

---

## Tâche 6 : `plan_meta.rapproche_le` et le réalignement sur le nouveau fichier

Après application, le plan décrit F2 : `plan_meta.fichier` et `plan_meta.hash`
doivent devenir ceux de F2, sinon `rapport_au_fichier` continuerait d'annoncer
« contenu différent » sur un plan qu'on vient précisément d'aligner.

**Fichiers :**
- Modifier : `client/src-tauri/src/store.rs:137-143` (schéma), `:225-255`
  (migration), `:632-638` (écriture), `:659-661` (lecture), `:67-77` (struct)

- [ ] **Étape 1 : écrire le test de persistance**

Dans `mod tests` de `store.rs`, à côté des tests de plan existants :

```rust
    #[test]
    fn plan_meta_conserve_la_date_de_rapprochement() {
        let s = Store::open_in_memory().unwrap();
        let l = ligne("CF1", "0225:123");
        let mut m = meta();
        s.ecrire_plan(&[l.clone()], &m).unwrap();
        let (_, relu) = s.charger_plan().unwrap().unwrap();
        assert_eq!(relu.rapproche_le, None, "un plan neuf n'a jamais été rapproché");

        // Après application, le plan décrit le NOUVEAU fichier.
        m.rapproche_le = Some(1_800_000_000);
        m.fichier = "f2.csv".into();
        m.hash = "bbb".into();
        s.ecrire_plan(&[l], &m).unwrap();
        let (_, relu) = s.charger_plan().unwrap().unwrap();
        assert_eq!(relu.rapproche_le, Some(1_800_000_000));
        assert_eq!(relu.fichier, "f2.csv");
        assert_eq!(relu.hash, "bbb");
    }
```

`ligne(cf, participant)` et `meta()` sont les helpers existants du module
(`store.rs:1341` et `:1360`). `meta()` doit gagner `rapproche_le: None` à
l'étape 3.

- [ ] **Étape 2 : lancer, vérifier l'échec**

```bash
cd client/src-tauri && cargo test plan_meta 2>&1 | tail -20
```

Attendu : ÉCHEC de compilation, `struct PlanMeta has no field named 'rapproche_le'`.

- [ ] **Étape 3 : ajouter le champ, la colonne et la migration**

`store.rs`, dans `PlanMeta` :

```rust
    /// Horodatage du dernier rapprochement appliqué, `None` si le plan n'a
    /// jamais été rapproché. Porté par le plan et non par la ligne : c'est le
    /// plan entier qui a été confronté à un fichier.
    pub rapproche_le: Option<i64>,
```

Dans `SCHEMA`, table `plan_meta` :

```sql
CREATE TABLE IF NOT EXISTS plan_meta (
  id            INTEGER PRIMARY KEY CHECK (id = 1),
  fichier       TEXT NOT NULL,
  hash          TEXT NOT NULL,
  genere_le     INTEGER NOT NULL,
  params_yaml   TEXT NOT NULL,
  rapproche_le  INTEGER
);
```

Dans `init`, après la migration des colonnes de `resolutions`, sur le même
modèle :

```rust
        // Migration : date de rapprochement (v1.5.0). Les plans existants ont
        // NULL, ce qui est exact — ils n'ont jamais été rapprochés.
        let present: bool = conn
            .prepare("SELECT 1 FROM pragma_table_info('plan_meta') WHERE name=?1")
            .and_then(|mut s| s.exists(["rapproche_le"]))
            .map_err(|e| e.to_string())?;
        if !present {
            conn.execute("ALTER TABLE plan_meta ADD COLUMN rapproche_le INTEGER", [])
                .map_err(|e| e.to_string())?;
        }
```

Dans `ecrire_plan`, l'INSERT de `plan_meta` :

```rust
        tx.execute(
            "INSERT INTO plan_meta (id, fichier, hash, genere_le, params_yaml, rapproche_le)
             VALUES (1, ?1, ?2, ?3, ?4, ?5)",
            params![meta.fichier, meta.hash, meta.genere_le, meta.params_yaml, meta.rapproche_le],
        )
        .map_err(|e| e.to_string())?;
```

Dans `charger_plan`, la lecture :

```rust
            .query_row(
                "SELECT fichier, hash, genere_le, params_yaml, rapproche_le FROM plan_meta WHERE id = 1",
```

et la construction de `PlanMeta` gagne `rapproche_le: r.get(4)?`.

- [ ] **Étape 4 : réparer les appelants**

`cargo build` liste chaque construction de `PlanMeta` à compléter par
`rapproche_le: None`. Au minimum : `commands.rs:1072` (`plan_generate`) et les
helpers de test `commands.rs:2285`.

Dans `plan_generate`, le plan neuf n'a jamais été rapproché → `None`.

- [ ] **Étape 5 : lancer, vérifier le succès**

```bash
cd client/src-tauri && cargo test 2>&1 | tail -25
```

Attendu : `test result: ok`, aucune régression.

- [ ] **Étape 6 : commit**

```bash
git add client/src-tauri/src/store.rs client/src-tauri/src/commands.rs
git commit -m "feat(superpopaul): plan_meta porte la date du dernier rapprochement"
```

---

## Tâche 7 : les deux commandes Tauri

**Fichiers :**
- Modifier : `client/src-tauri/src/commands.rs`, `client/src-tauri/src/lib.rs`

- [ ] **Étape 1 : écrire le test de l'avertissement PPF**

L'avertissement d'annuaire cumulatif est une fonction pure, testable sans
Tauri. Dans `mod tests` de `commands.rs` :

```rust
    #[test]
    fn l_annuaire_ppf_cumulatif_est_signale() {
        // Deux fichiers chargés sans reset : la perte d'éligibilité PPF n'est
        // pas détectable, et un « 0 » se lirait comme « il n'y en a pas ».
        let a = avertissement_ppf_cumulatif(2);
        let a = a.expect("deux fichiers doivent produire un avertissement");
        assert!(a.contains("recharger"), "l'avertissement doit dire quoi faire : {a}");
    }

    #[test]
    fn un_annuaire_ppf_charge_une_seule_fois_ne_declenche_rien() {
        assert!(avertissement_ppf_cumulatif(1).is_none());
        assert!(avertissement_ppf_cumulatif(0).is_none());
    }
```

- [ ] **Étape 2 : lancer, vérifier l'échec**

```bash
cd client/src-tauri && cargo test annuaire_ppf 2>&1 | tail -15
```

Attendu : `cannot find function 'avertissement_ppf_cumulatif'`.

- [ ] **Étape 3 : implémenter la fonction et les commandes**

Dans `commands.rs`, près des autres helpers du plan :

```rust
/// L'annuaire PPF est chargé **cumulativement** (`store::ingest_ppf`) : un
/// identifiant qui en sort, ou passe à un motif inactif, conserve sa ligne et
/// reste utilisable. Au-delà d'un fichier chargé, la perte d'éligibilité PPF
/// n'est donc pas détectable — et un « 0 » se lirait comme « il n'y en a
/// pas ». La correction du chargement est un lot séparé ; ici on le dit.
fn avertissement_ppf_cumulatif(fichiers: usize) -> Option<String> {
    (fichiers > 1).then(|| {
        format!(
            "l'annuaire PPF a été construit par cumul de {fichiers} fichiers : une \
             éligibilité PPF perdue n'y est pas détectable. Pour un rapprochement \
             complet, vide l'annuaire puis recharge le fichier le plus récent."
        )
    })
}

/// Enveloppe de commande : le rapprochement lui-même est pur, l'empreinte du
/// fichier vient du disque.
#[derive(Serialize)]
pub struct RapprochementVue {
    pub rapprochement: crate::rapprochement::Rapprochement,
    pub empreinte: String,
}

/// Cœur partagé par le calcul et l'application. Rend aussi le plan et sa méta,
/// dont l'application a besoin.
fn calculer_rapprochement(
    store: &Arc<Mutex<Store>>,
    input: &Path,
    cfg: &Config,
) -> Result<
    (
        crate::rapprochement::Rapprochement,
        String,
        Vec<crate::plan::LignePlan>,
        crate::store::PlanMeta,
    ),
    String,
> {
    let (lignes, meta) = charger_pour_retouche(store)?;
    let (runs, meps) = calendrier_du_plan(&meta)?;
    let (entrees, fichiers_ppf) = {
        let s = store.lock().unwrap();
        (
            plan_entrees_from_scan(&s, input, cfg, chrono::Utc::now())?,
            s.ppf_files()?.len(),
        )
    };
    let aujourdhui = chrono::Local::now().date_naive();
    let mut r = crate::rapprochement::calculer(&lignes, &entrees, &runs, &meps, aujourdhui)?;
    r.avertissements.extend(avertissement_ppf_cumulatif(fichiers_ppf));
    let empreinte = sha256_hex(&std::fs::read(input).map_err(|e| format!("lecture entrée : {e}"))?);
    Ok((r, empreinte, lignes, meta))
}

/// Rapproche le plan du fichier ouvert **sans rien écrire**.
#[tauri::command]
pub async fn plan_rapprocher(state: State<'_, AppState>) -> Result<RapprochementVue, String> {
    let cfg = state.current_config()?;
    let input = state.input_path()?;
    let store = state.store.clone();
    tokio::task::spawn_blocking(move || {
        let (rapprochement, empreinte, _, _) = calculer_rapprochement(&store, &input, &cfg)?;
        Ok(RapprochementVue { rapprochement, empreinte })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Recalcule le rapprochement **depuis zéro** et l'applique. Le diff ne
/// transite pas par le front : ce qui s'écrit ne dépend jamais de données
/// remontées par le JS. `empreinte` est celle vue au calcul — si le fichier a
/// bougé depuis, on refuse plutôt que d'appliquer autre chose que ce qui a été
/// lu à l'écran.
#[tauri::command]
pub async fn plan_rapprocher_appliquer(
    state: State<'_, AppState>,
    empreinte: String,
) -> Result<Vec<String>, String> {
    let cfg = state.current_config()?;
    let input = state.input_path()?;
    let store = state.store.clone();
    tokio::task::spawn_blocking(move || {
        let (r, courante, mut lignes, mut meta) = calculer_rapprochement(&store, &input, &cfg)?;
        if courante != empreinte {
            return Err("le fichier a changé depuis le calcul — relance le rapprochement \
                        avant d'appliquer"
                .into());
        }
        let maintenant = chrono::Utc::now().timestamp();
        crate::rapprochement::appliquer(&mut lignes, &r, maintenant)?;
        // Le plan décrit désormais le fichier ouvert : sans ça,
        // `rapport_au_fichier` continuerait d'annoncer « contenu différent »
        // sur un plan qu'on vient précisément d'aligner.
        meta.fichier = input
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        meta.hash = courante;
        meta.rapproche_le = Some(maintenant);
        sauver_apres_retouche(&store, &input, &cfg, &lignes, &meta)
    })
    .await
    .map_err(|e| e.to_string())?
}
```

Dans `lib.rs`, après `commands::plan_runs_compatibles,` :

```rust
            commands::plan_rapprocher,
            commands::plan_rapprocher_appliquer,
```

- [ ] **Étape 4 : un run exclu n'est jamais cible d'un déplacement**

`rapprochement::run_cible` ne filtre pas `RunFacturation::exclu`, et c'est
délibéré : `calendrier::runs_utilisables` (`calendrier.rs:182`) l'a déjà fait,
et `plan::runs_compatibles` (`plan.rs:820`) suit la même convention — la règle
vit à un seul endroit. Mais rien ne le prouve dans les tests du module pur,
puisqu'il ne connaît pas son appelant.

C'est ici que ça se prouve, `calendrier_du_plan` étant dans la chaîne :

```rust
    /// L'utilisateur exclut un run (férié, run annulé) et s'attend à ce que
    /// rien n'y soit envoyé. La garantie vient de `runs_utilisables`, en
    /// amont — ce test la verrouille au niveau où elle est observable.
    #[test]
    fn un_run_exclu_n_est_jamais_propose_comme_cible_de_deplacement() {
        // … construire une méta de plan dont le calendrier porte un run exclu
        // couvrant le nouveau jour de cycle, et vérifier que le rapprochement
        // le signale au lieu de proposer un déplacement vers lui.
    }
```

Écris ce test avec les fixtures réelles du module `commands` (voir
`commands.rs:2285` pour le helper `meta`). S'il s'avère que la chaîne ne
permet pas de l'exprimer sans monter un `Store` complet, dis-le et déplace la
vérification en test d'intégration plutôt que de la laisser tomber.

- [ ] **Étape 5 : lancer, vérifier le succès**

```bash
cd client/src-tauri && cargo test 2>&1 | tail -25 && cargo clippy --all-targets 2>&1 | tail -15
```

Attendu : `test result: ok`, et clippy sans avertissement neuf.

- [ ] **Étape 5 : commit**

```bash
git add client/src-tauri/src/commands.rs client/src-tauri/src/lib.rs
git commit -m "feat(superpopaul): commandes plan_rapprocher et plan_rapprocher_appliquer"
```

---

## Tâche 8 : maquette de l'écran — POINT D'ARRÊT

**Aucun code d'IHM avant validation explicite de la maquette.** C'est une règle
de l'utilisateur, pas une préférence.

**Fichiers :**
- Créer : `docs/superpowers/maquettes/2026-07-28-rapprochement.html`

- [ ] **Étape 1 : écrire la maquette**

Document HTML autonome, palette du client (`--bg:#0e1524`, `--card:#172136`,
`--gold:#d9a83f`, `--red:#e5534b`, `--amber:#e0873a`, `--green:#4cc268`),
`color-scheme: dark`. Elle doit montrer :

- le point d'entrée : bouton « Rapprocher avec le fichier ouvert » dans
  l'onglet « Comptes de facturation », disponible dans les **quatre** états de
  `RapportAuFichier` ;
- l'état « le plan est à jour », bouton d'application inerte ;
- l'écran de revue avec un groupe par nature d'écart, chacun avec son compte :
  retraits pour éligibilité perdue, retraits pour disparition, déplacements
  (jour et run, avant → après), signalés, gelés à part, inchangés ;
- les avertissements : annuaire PPF cumulatif, retraits sur MEP livrée,
  ampleur au-delà du quart, répartition par plateforme modifiée ;
- l'action « Appliquer *n* changements » et « Annuler » ;
- l'état d'erreur « le fichier a changé depuis le calcul ».

**Piège de rendu à traiter explicitement** : un jour de cycle illisible dans le
fichier produit `Nature::JourChange { avant: l.jj, apres: 0 }`. Zéro est une
sentinelle hors domaine (les jours vont de 1 à 31), pas une valeur. Rendu
littéralement, l'écran afficherait « jour de cycle : 5 → 0 », qui se lit comme
un jour zéro réel. La maquette doit montrer ce cas et le nommer « jour de cycle
illisible », jamais un chiffre.

- [ ] **Étape 2 : la faire valider**

Envoyer la maquette à l'utilisateur et **attendre un go explicite**. Ne pas
enchaîner sur la tâche 9 sans lui.

---

## Tâche 9 : l'écran de revue

**À n'entamer qu'après validation de la maquette.**

**Fichiers :**
- Modifier : `client/src/app.js`, `client/src/styles.css`
- Créer : `client/tests/rapprochement.test.js`

- [ ] **Étape 1 : écrire les tests de câblage**

Le code exact de ces tests dépend des identifiants et de la structure de
l'écran, que **la maquette validée à la tâche 8 fixe**. L'écrire ici
inventerait une IHM que l'utilisateur n'a pas encore vue. Ce qui est arrêté,
en revanche, c'est **ce que les tests doivent prouver** — et ils s'écrivent
avant l'implémentation, sur le modèle des tests JS existants (voir
`client/tests/` et `client/tests/dom_shim.js`, qui exécute le **vrai**
`src/app.js`) :

1. **l'empreinte survit à un re-rendu** — doublure de `plan_rapprocher`
   rendant `{ rapprochement: {...}, empreinte: "abc" }` ; forcer un re-rendu
   de l'écran ; déclencher l'application ; vérifier que l'appel à
   `plan_rapprocher_appliquer` porte bien `{ empreinte: "abc" }`. Sans elle,
   le backend refuse et l'utilisateur lit une erreur qu'il ne comprend pas ;
2. **sans écart, le déclencheur d'application est inerte** — doublure rendant
   `{ rapprochement: { ecarts: [], inchangees: 12, avertissements: [] },
   empreinte: "abc" }` ; vérifier qu'aucun appel à
   `plan_rapprocher_appliquer` ne part ;
3. **une erreur backend s'affiche sans passer par `innerHTML`** — doublure
   rejetant avec un message contenant `<script>` ; vérifier que le texte est
   posé en `textContent` et qu'aucun nœud `script` n'apparaît dans le DOM.

⚠️ Le faux DOM prouve qu'un nœud existe, **jamais** qu'il est visible. Les
tests ne remplacent pas le parcours en application.

⚠️ Après tout changement de forme d'un retour de commande, `grep` l'ancien
champ dans `client/tests/` : le compilateur ne couvre pas le JS.

- [ ] **Étape 2 : lancer, vérifier l'échec**

```bash
cd client && node --test "tests/*.test.js" 2>&1 | tail -20
```

- [ ] **Étape 3 : implémenter l'écran conformément à la maquette validée**

Rappel de sécurité, non négociable : **jamais d'`innerHTML` avec des données
dynamiques**. Les comptes, motifs et messages d'erreur passent par le helper
`h()` ou `textContent`. Un CSV est une entrée non fiable.

- [ ] **Étape 4 : lancer les deux suites**

```bash
cd client && node --test "tests/*.test.js" 2>&1 | tail -20
cd client/src-tauri && cargo test 2>&1 | tail -20
```

- [ ] **Étape 5 : commit**

```bash
git add client/src/app.js client/src/styles.css client/tests/rapprochement.test.js
git commit -m "feat(superpopaul): écran de revue du rapprochement"
```

---

## Tâche 10 : passe de mutation et validation

**Fichiers :**
- Modifier : `client/src-tauri/src/rapprochement.rs` (tests renforcés)

- [ ] **Étape 1 : muter le module**

Sur les cinq tâches Rust du plan de charge, la mutation a trouvé un trou à
chaque fois — jamais un test manquant, toujours un test incapable d'échouer.
Mutations à passer une par une, en vérifiant qu'**au moins un test rougit** :

1. `run_cible` : retirer `&& r.date >= aujourdhui`
2. `run_cible` : remplacer `*id != mep_actuelle` par `false`
3. `calculer` : retirer le court-circuit `if gelee` du déplacement
4. `calculer` : retirer `continue` après la branche de retrait
5. `calculer` : remplacer `retraits * 4 > actives` par `retraits > actives`
6. `appliquer` : retirer l'affectation `l.jj = apres`
7. `appliquer` : mettre `l.origine = Origine::Manuel` dans `Deplacer`
8. `appliquer` : supprimer la boucle de vérification préalable
9. `libelle_ctc` : changer la chaîne de la branche `_ => "non prêt"`
   — trou relevé par la revue qualité de la tâche 2 et reporté ici. La branche
   est inatteignable en production (`libelle_ctc` n'est appelée que si
   `!ctc_ready`, et l'invariant du projet est
   `ctc_ready == (ctc_status == "ready")`), mais elle est structurellement
   nécessaire : un `match` sur `&str` exige un cas par défaut. Un test direct
   suffit, sur le modèle de `libelle_ctc_distingue_expire_et_jamais_resolu`.

Une mutation survivante peut être **équivalente** — le prouver avant de
l'écarter, ne pas l'absorber par principe.

- [ ] **Étape 2 : combler chaque trou trouvé**

Un test par mutation survivante, qui échoue avec la mutation et passe sans.

- [ ] **Étape 3 : lancer la suite complète**

```bash
cd client/src-tauri && cargo test 2>&1 | tail -25
cd client && node --test "tests/*.test.js" 2>&1 | tail -20
```

Lire les sorties **complètes** : `grep "test result: ok"` masque une suite
rouge quand plusieurs binaires de test tournent.

- [ ] **Étape 4 : commit**

```bash
git add client/src-tauri/src/rapprochement.rs
git commit -m "test(superpopaul): combler les trous trouvés par mutation sur le rapprochement"
```

- [ ] **Étape 5 : parcours en application — geste de l'utilisateur**

Le cœur est prouvé par les tests ; l'écran ne l'est pas. Demander à
l'utilisateur de valider le parcours en GUI avant toute release :

1. générer un plan sur un fichier F1 ;
2. ouvrir un fichier F2 avec un compte disparu, un compte au jour changé et un
   compte devenu inéligible ;
3. rapprocher, lire la revue, appliquer ;
4. vérifier que les comptes inchangés n'ont pas bougé de run, que les fichiers
   MEP ont été réécrits, et que le bandeau « contenu différent » a disparu.

**Un point que seul ce parcours peut prouver — le verrou d'empreinte.**

`plan_rapprocher_appliquer` refuse d'appliquer si le fichier a changé entre le
calcul et le clic. Cette garde n'est couverte par **aucun test automatisé** :
`tauri::State` a un champ privé et `StateManager::new` est `pub(crate)`, donc
une commande `#[tauri::command]` n'est pas appelable hors d'une application
Tauri montée — et aucun test du projet ne le fait. Extraire la comparaison
dans une fonction pure ne prouverait rien de plus : le risque n'est pas que
`!=` se trompe, c'est qu'on **oublie d'appeler** la garde, ce qu'une fonction
extraite ne détecte pas davantage.

À vérifier donc à la main, au moins une fois :

5. rapprocher, puis **modifier le fichier ouvert** avant de cliquer sur
   « Appliquer » ; le refus doit être explicite et proposer de relancer le
   rapprochement.

---

## Ce que ce plan ne fait pas

- **La correction du chargement cumulatif de l'annuaire PPF** — lot séparé,
  seulement signalé par un avertissement (tâche 7).
- L'ajout au plan des comptes éligibles absents.
- Le décochage ligne à ligne avant application.
- Toute modification de `plan::regenerer` et de la rampe.
