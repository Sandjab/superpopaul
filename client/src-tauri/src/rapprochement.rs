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
