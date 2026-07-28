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
                action: Action::Retirer { motif: "absent du fichier".into() },
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
                action: Action::Retirer { motif: apres },
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
        r.inchangees += 1;
    }
    Ok(r)
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
        assert_eq!(motif, "CTC prêt plus tard");
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
        assert_eq!(motif, "PPF non utilisable");
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
}
