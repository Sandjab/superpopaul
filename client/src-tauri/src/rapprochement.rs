//! Rapprochement d'un plan persisté avec un fichier de comptes plus récent.
//!
//! Module **pur** : aucune I/O, aucune horloge. Ce qui dépend du disque ou de
//! la base (empreinte du fichier, état de l'annuaire PPF) vit dans
//! `commands.rs`, comme pour `securisation` et `modes`.

use crate::calendrier::RunFacturation;
use crate::plan::{LigneEntree, LignePlan};
use std::collections::{BTreeMap, HashMap};

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
    let par_cf: HashMap<&str, &LigneEntree> = crate::plan::dedoublonner(entrees)?
        .into_iter()
        .map(|e| (e.cf.as_str(), e))
        .collect();
    // Un motif sans date est ingérable six mois plus tard.
    let stamp = format!("Rapprochement du {}", aujourdhui.format("%d/%m/%Y"));

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
                action: Action::Retirer { motif: format!("{stamp} — absent du fichier") },
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
                nature: Nature::EligibilitePerdue { avant, apres: apres.clone() },
                action: Action::Retirer { motif: format!("{stamp} — {apres}") },
                gelee,
            });
            continue;
        }
        let jj_fichier = crate::plan::parse_jj(&e.jj_brut);
        if jj_fichier != Some(l.jj) {
            // Un jour illisible n'est pas un changement : c'est une donnée
            // qu'on ne sait pas lire. On le signale sans rien décider.
            let Some(neuf) = jj_fichier else {
                // `apres: 0` est une sentinelle hors domaine (les jours de
                // cycle vont de 1 à 31) : elle marque un jour illisible, pas
                // un jour 0 réel.
                r.ecarts.push(Ecart {
                    cf: l.cf.clone(),
                    nature: Nature::JourChange { avant: l.jj, apres: 0 },
                    action: Action::Signaler,
                    gelee,
                });
                continue;
            };
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
            r.ecarts.push(Ecart {
                cf: l.cf.clone(),
                nature: Nature::JourChange { avant: l.jj, apres: neuf },
                action,
                gelee,
            });
            continue;
        }
        // Pas de court-circuit `gelee` ici, contrairement au jour changé : la
        // plateforme ne conditionne aucun fichier transmis aux tiers lors
        // d'une MEP (ils ne contiennent que des comptes nus) — à la
        // différence du fichier comparé ici (`entrees`), qui porte `pa`. Le
        // jour de cycle, lui, décide du lot d'appartenance d'un compte déjà
        // transmis. Le champ `pa` ne sert qu'aux quotas et à l'affichage :
        // le rafraîchir sur une ligne gelée ne rouvre rien qui ait déjà été
        // livré.
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
        r.inchangees += 1;
    }

    if let Some(a) = avertissement_ampleur(plan, &r.ecarts) {
        r.avertissements.push(a);
    }
    if let Some(a) = avertissement_repartition_plateforme(&r.ecarts) {
        r.avertissements.push(a);
    }

    Ok(r)
}

/// Applique un rapprochement au plan, **par mutation en place**. Aucune
/// ré-allocation n'est appelée : c'est ce qui garantit que le reste du plan ne
/// bouge pas.
///
/// Tout est vérifié avant d'écrire quoi que ce soit — comme `plan::ajouter` :
/// un lot à moitié appliqué serait pire qu'un refus. Un compte n'apparaît au
/// plus qu'une fois dans les écarts : c'est `calculer` qui le garantit, un
/// seul écart est produit par ligne de plan.
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
        // `nature` et `action` sont indépendants : rien ne les lie sauf ici.
        // Un `Deplacer` sans `JourChange` écrirait le run et la MEP mais pas
        // le jour, EN SILENCE — exactement le lot à moitié appliqué que cette
        // fonction refuse partout ailleurs. Un écart mal apparié fait donc
        // échouer tout le rapprochement, avant toute écriture.
        match (&e.nature, &e.action) {
            (Nature::JourChange { .. }, Action::Deplacer { .. })
            | (Nature::PlateformeChangee { .. }, Action::Rafraichir) => {}
            (_, Action::Deplacer { .. }) => {
                return Err(format!(
                    "le compte « {} » a une action de déplacement sans changement de jour de cycle",
                    e.cf
                ));
            }
            (_, Action::Rafraichir) => {
                return Err(format!(
                    "le compte « {} » a une action de rafraîchissement sans changement de plateforme",
                    e.cf
                ));
            }
            _ => {}
        }
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
            // Vu, rien à faire automatiquement : l'utilisateur tranche avec
            // les outils existants. Le vide est délibéré, pas un oubli.
            Action::Signaler => {}
        }
    }
    Ok(())
}

/// Au-delà du quart des lignes actives retirées, l'ampleur doit être dite.
/// Seuil chiffré plutôt qu'un jugement : « beaucoup » ne se teste pas.
fn avertissement_ampleur(plan: &[LignePlan], ecarts: &[Ecart]) -> Option<String> {
    let retraits = ecarts.iter().filter(|e| matches!(e.action, Action::Retirer { .. })).count();
    let actives = plan.iter().filter(|l| !l.retiree()).count();
    (actives > 0 && retraits * 4 > actives)
        .then(|| format!("ce rapprochement retire {retraits} des {actives} lignes actives du plan"))
}

/// Les quotas par plateforme ne sont pas rejoués — ce serait du re-tirage.
/// Le décalage qu'ils prennent doit donc être dit, chiffres à l'appui :
/// sinon la répartition affichée ailleurs devient fausse en silence.
fn avertissement_repartition_plateforme(ecarts: &[Ecart]) -> Option<String> {
    let mut mouvements: BTreeMap<(&str, &str), usize> = BTreeMap::new();
    for e in ecarts {
        if let Nature::PlateformeChangee { avant, apres } = &e.nature {
            *mouvements.entry((avant.as_str(), apres.as_str())).or_insert(0) += 1;
        }
    }
    if mouvements.is_empty() {
        return None;
    }
    let detail: Vec<String> =
        mouvements.iter().map(|((a, b), n)| format!("{n} de {a} vers {b}")).collect();
    Some(format!(
        "la répartition par plateforme change sans être rejouée : {}",
        detail.join(", ")
    ))
}

/// Run cible pour un compte qui a changé de jour de cycle.
///
/// Moindre perturbation : d'abord un run de la MÊME MEP que la ligne actuelle
/// — le compte reste dans son lot, seul son ordonnancement change. À défaut,
/// le run compatible dont la MEP est la plus PROCHE de la MEP actuelle (par
/// distance, pas par ancienneté — une MEP plus tardive mais voisine perturbe
/// moins qu'une MEP plus ancienne mais lointaine). **Ni un run déjà passé, ni
/// un run rattaché à une MEP déjà passée** : dans les deux cas la ligne
/// basculerait dans le gel avec effet rétroactif. Le second cas n'est pas
/// qu'en théorie : `mep_de` rattache un run à la dernière MEP qui ne lui est
/// pas postérieure, qui peut donc être passée même pour un run futur, si
/// aucune MEP n'a été déclarée entre les deux.
///
/// Précondition non revérifiée ici : `runs` est censé provenir de
/// `calendrier::runs_utilisables`, qui a déjà écarté les runs exclus (et hors
/// fenêtre). Un run exclu n'est donc jamais candidat, sans qu'il soit besoin
/// de refiltrer `exclu` — même choix que `plan::runs_compatibles`, qui ne le
/// refait pas non plus pour la même raison.
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
    // Distance à la MEP actuelle, puis date de run pour départager. La MEP
    // actuelle (distance 0) l'emporte donc déjà sur toute autre MEP sans
    // indicateur séparé : une distance ne descend jamais sous zéro.
    // Troisième clé (`*id`) pour un ordre total : deux MEP à distance égale
    // dont les runs tombent le même jour ne doivent pas dépendre de l'ordre
    // d'itération de `runs`.
    candidats.sort_by_key(|(r, id, _)| (id.abs_diff(mep_actuelle), r.date, *id));
    candidats.into_iter().next()
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
        // Égalité stricte, pas `contains` : un simple préfixe « CTC » laisserait
        // passer n'importe quel libellé (cf. mutation trouvée par la revue sur
        // `libelle_ctc`).
        assert_eq!(motif, "Rapprochement du 01/08/2026 — CTC prêt plus tard");
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
        assert_eq!(motif, "Rapprochement du 01/08/2026 — PPF non utilisable");
    }

    /// `libelle_ctc` seule : couvre les branches qu'aucun test de `calculer`
    /// n'exerçait (`expired`, statut vide). Un statut vide (jamais résolu)
    /// n'est pas la même chose que « pas prêt » — deux motifs distincts le
    /// prouvent, un `contains` commun aux deux ne le prouverait pas.
    #[test]
    fn libelle_ctc_distingue_expire_et_jamais_resolu() {
        assert_eq!(libelle_ctc("expired"), "expiré");
        assert_eq!(libelle_ctc(""), "non résolu");
        assert_ne!(libelle_ctc("expired"), libelle_ctc(""));
    }

    /// Branche `_` : structurellement nécessaire (un `match` sur `&str` exige
    /// un cas par défaut) mais inatteignable en production — `output::ctc_status`
    /// ne rend que `""`, `"ready"`, `"later"` ou `"expired"`, et `libelle_ctc`
    /// n'est appelée que quand `!ctc_ready` (donc jamais avec `"ready"`), qui
    /// sont tous les trois nommés explicitement. Un statut hors de ce domaine
    /// (donnée corrompue, valeur future non encore mappée) ne doit pas non
    /// plus planter ni s'afficher vide : la valeur de repli est verrouillée
    /// ici, seul test qui exercerait cette branche.
    #[test]
    fn libelle_ctc_a_une_valeur_de_repli_pour_un_statut_hors_domaine() {
        assert_eq!(libelle_ctc("statut_futur_inconnu"), "non prêt");
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

    /// DÉFAUT (démonstration) : `run_cible` filtre sur la date du RUN
    /// (`r.date >= aujourdhui`), pas sur la date de la MEP à laquelle il est
    /// rattaché. `mep_de` rattache un run à la dernière MEP qui ne lui est pas
    /// postérieure — un run futur peut donc retomber sur une MEP déjà passée
    /// si aucune MEP n'a été déclarée entre les deux. La ligne déplacée
    /// hériterait alors d'un `mep_date` passé, et `gelee(aujourdhui)`
    /// deviendrait vraie immédiatement : gel rétroactif d'un lot qui n'a
    /// jamais été livré à ce jour.
    #[test]
    fn un_run_futur_rattache_a_une_mep_deja_passee_n_est_pas_choisi() {
        // RF01 est un run FUTUR (10/08, après aujourd'hui 01/08), mais il
        // précède la MEP 2 (01/09) : sa MEP de rattachement est la MEP 1
        // (01/07), déjà passée à aujourd'hui.
        let runs = vec![run("RF01", "2026-08-10", &[12])];
        let meps = vec![d("2026-07-01"), d("2026-09-01")];
        let plan = vec![ligne("CF1", 5, "Cegedim", "RF09", (2, "2026-09-01"))];
        let entrees = vec![entree("CF1", "12", "Cegedim")];
        let r = calculer(&plan, &entrees, &runs, &meps, d("2026-08-01")).unwrap();
        assert_eq!(
            r.ecarts[0].action,
            Action::Signaler,
            "le seul run compatible est rattaché à une MEP déjà passée : rien à faire automatiquement"
        );
    }

    /// DÉFAUT (démonstration) : le tri départage les candidats hors MEP
    /// courante par identifiant croissant (« la plus ancienne d'abord »),
    /// pas par distance à la MEP courante (« la plus proche d'abord »).
    #[test]
    fn a_defaut_de_meme_mep_le_run_de_la_mep_la_plus_proche_est_choisi() {
        let runs = vec![
            run("RF_A", "2026-02-10", &[20]), // MEP 2 : distance 2 à la MEP 4
            run("RF_B", "2026-03-10", &[20]), // MEP 3 : distance 1 à la MEP 4, la plus proche
        ];
        let meps = vec![d("2026-01-01"), d("2026-02-01"), d("2026-03-01"), d("2026-04-01")];
        let plan = vec![ligne("CF1", 5, "Cegedim", "RF09", (4, "2026-04-01"))];
        let entrees = vec![entree("CF1", "20", "Cegedim")];
        let r = calculer(&plan, &entrees, &runs, &meps, d("2026-01-15")).unwrap();
        let Action::Deplacer { run_num, .. } = &r.ecarts[0].action else {
            panic!("attendu un déplacement, obtenu {:?}", r.ecarts[0].action);
        };
        assert_eq!(run_num, "RF_B", "la MEP 3 est plus proche de la MEP 4 que la MEP 2");
    }

    /// `run_date`/`mep_date` doivent partir en JSON au même format ISO que
    /// partout ailleurs dans l'appli (`plan::LignePlan` sérialisé via
    /// `.to_string()` — voir `commands.rs`) : rien ne le vérifiait, alors que
    /// c'est tout l'objet du commentaire sur `date_iso`. Un format qui diverge
    /// (ex. jour/mois/année) casserait silencieusement toute comparaison ou
    /// tri de dates côté écran, sans qu'aucun test Rust ne le voie — les tests
    /// existants ne comparent que des `NaiveDate`, jamais le JSON produit.
    #[test]
    fn deplacer_serialise_ses_dates_au_meme_format_iso_que_le_reste_du_plan() {
        let (runs, meps, auj) = contexte();
        let plan = vec![ligne("CF1", 5, "Cegedim", "RF01", (2, "2026-09-01"))];
        let entrees = vec![entree("CF1", "12", "Cegedim")];
        let r = calculer(&plan, &entrees, &runs, &meps, auj).unwrap();
        let v = serde_json::to_value(&r.ecarts[0].action).expect("sérialisation JSON");
        assert_eq!(v["run_date"], "2026-09-20", "obtenu {v}");
        assert_eq!(v["mep_date"], "2026-09-01", "obtenu {v}");
    }

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

    /// L'ordre d'insertion est volontairement PAS alphabétique : si le détail
    /// provenait d'une `HashMap` plutôt que d'une `BTreeMap`, l'ordre du
    /// message dépendrait du hasard de hachage du processus — le même
    /// rapprochement pourrait afficher un ordre différent d'une exécution à
    /// l'autre, pour des données identiques.
    #[test]
    fn plusieurs_changements_de_plateforme_sont_dans_un_ordre_deterministe() {
        let (runs, meps, auj) = contexte();
        let plan = vec![
            ligne("CF1", 5, "Zeta", "RF01", (2, "2026-09-01")),
            ligne("CF2", 5, "Cegedim", "RF01", (2, "2026-09-01")),
            ligne("CF3", 5, "Esker", "RF01", (2, "2026-09-01")),
        ];
        let entrees = vec![
            entree("CF1", "5", "Alpha"),
            entree("CF2", "5", "Esker"),
            entree("CF3", "5", "Cegedim"),
        ];
        let r = calculer(&plan, &entrees, &runs, &meps, auj).unwrap();
        let a = r
            .avertissements
            .iter()
            .find(|a| a.contains("plateforme"))
            .unwrap_or_else(|| panic!("obtenu {:?}", r.avertissements));
        assert_eq!(
            a,
            "la répartition par plateforme change sans être rejouée : \
             1 de Cegedim vers Esker, 1 de Esker vers Cegedim, 1 de Zeta vers Alpha",
            "l'ordre doit être trié, pas dépendant du hachage : obtenu {a}"
        );
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

    /// La spec dit « au-delà du quart », pas « à partir du quart » : à la
    /// limite exacte, aucun avertissement. Protège spécifiquement la
    /// frontière `>` contre une mutation en `>=`.
    #[test]
    fn un_rapprochement_a_exactement_un_quart_ne_produit_pas_d_avertissement_d_ampleur() {
        let (runs, meps, auj) = contexte();
        let plan: Vec<LignePlan> = (0..8)
            .map(|i| ligne(&format!("CF{i}"), 5, "Cegedim", "RF01", (2, "2026-09-01")))
            .collect();
        let mut entrees: Vec<LigneEntree> = (0..8)
            .map(|i| entree(&format!("CF{i}"), "5", "Cegedim"))
            .collect();
        entrees[0] = avec_ctc(entrees[0].clone(), "later"); // 2 sur 8 : pile un quart
        entrees[1] = avec_ctc(entrees[1].clone(), "later");
        let r = calculer(&plan, &entrees, &runs, &meps, auj).unwrap();
        assert!(r.avertissements.is_empty(), "obtenu {:?}", r.avertissements);
    }

    /// La plateforme n'est pas gardée contre le gel comme le jour de cycle :
    /// elle ne conditionne aucun fichier de livraison, contrairement au jour
    /// qui décide du lot. Une ligne gelée est donc rafraîchie normalement,
    /// et l'écart reste marqué gelé pour l'IHM.
    #[test]
    fn une_ligne_gelee_dont_la_plateforme_change_est_rafraichie_et_marquee() {
        let (runs, meps, auj) = contexte();
        // MEP 1 = 2026-07-01, passée au 2026-08-01.
        let plan = vec![ligne("CF1", 5, "Cegedim", "RF01", (1, "2026-07-01"))];
        let entrees = vec![entree("CF1", "5", "Esker")];
        let r = calculer(&plan, &entrees, &runs, &meps, auj).unwrap();
        assert_eq!(r.ecarts[0].action, Action::Rafraichir);
        assert!(r.ecarts[0].gelee, "l'IHM doit pouvoir isoler ce cas aussi");
    }

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

    /// Un écart mal apparié (nature/action incohérentes) ne doit jamais
    /// s'appliquer à moitié en silence : un `Deplacer` sans `JourChange`
    /// écrirait le run et la MEP mais pas le jour, sans lever d'erreur.
    #[test]
    fn appliquer_refuse_un_ecart_dont_la_nature_ne_correspond_pas_a_l_action() {
        let mut plan = vec![ligne("CF1", 5, "Cegedim", "RF01", (2, "2026-09-01"))];
        let temoin = plan[0].clone();
        let r = Rapprochement {
            ecarts: vec![Ecart {
                cf: "CF1".into(),
                nature: Nature::DisparuDuFichier,
                action: Action::Deplacer {
                    run_num: "RF02".into(),
                    run_date: d("2026-09-20"),
                    mep_id: 2,
                    mep_date: d("2026-09-01"),
                },
                gelee: false,
            }],
            inchangees: 0,
            avertissements: vec![],
        };
        let err = appliquer(&mut plan, &r, 1_800_000_000).unwrap_err();
        assert!(err.contains("CF1"), "obtenu : {err}");
        assert_eq!(plan[0], temoin, "rien ne doit avoir été écrit");
    }

    /// Symétrique du test précédent, pour l'autre branche du même
    /// appariement : un `Rafraichir` sans `PlateformeChangee` écrirait le
    /// champ `pa` depuis une nature qui ne le porte pas. Seule la branche
    /// `Deplacer` était couverte jusqu'ici — celle-ci ne l'était par rien.
    #[test]
    fn appliquer_refuse_un_ecart_rafraichir_dont_la_nature_ne_correspond_pas() {
        let mut plan = vec![ligne("CF1", 5, "Cegedim", "RF01", (2, "2026-09-01"))];
        let temoin = plan[0].clone();
        let r = Rapprochement {
            ecarts: vec![Ecart {
                cf: "CF1".into(),
                nature: Nature::DisparuDuFichier,
                action: Action::Rafraichir,
                gelee: false,
            }],
            inchangees: 0,
            avertissements: vec![],
        };
        let err = appliquer(&mut plan, &r, 1_800_000_000).unwrap_err();
        assert!(err.contains("CF1"), "obtenu : {err}");
        assert_eq!(plan[0], temoin, "rien ne doit avoir été écrit");
    }
}
