//! Classeur XLSX du périmètre du plan — **tous** les comptes du fichier
//! d'entrée, au plan ou non.
//!
//! La composition du tableau (`lignes`) est PURE et testable ; l'écriture
//! (`ecrire`) n'a aucune logique métier. Même séparation que `charge` et
//! `charts` pour le rapport.

use crate::plan::{LigneEntree, LignePlan};

/// Rapport d'un compte au plan. Trois états, pas deux : un compte **retiré**
/// n'est pas un compte jamais placé — ce sont deux décisions opposées.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Appartenance {
    Oui,
    Retire,
    Non,
}

impl Appartenance {
    /// Libellé de la colonne « Dans le plan ».
    pub fn libelle(self) -> &'static str {
        match self {
            Appartenance::Oui => "oui",
            Appartenance::Retire => "retiré",
            Appartenance::Non => "non",
        }
    }
}

/// Une ligne du classeur : un compte du fichier d'entrée.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LigneExport {
    /// Vide si le compte n'a jamais été placé ; **conservé** s'il a été retiré.
    pub run: String,
    pub cf: String,
    /// Le jour de cycle tel qu'il figurait dans le fichier, même illisible.
    pub jj: String,
    /// Adressage sous forme nue quand le schéma s'y prête.
    pub adressage: String,
    pub raison_sociale: String,
    pub ctc_status: String,
    pub ppf_usable: bool,
    pub appartenance: Appartenance,
}

/// Compose le tableau : une ligne par compte du fichier d'entrée, dans l'ordre
/// où il les fournit. PURE — ni disque, ni format.
pub fn lignes(entrees: &[LigneEntree], plan: &[LignePlan]) -> Vec<LigneExport> {
    let par_cf: std::collections::HashMap<&str, &LignePlan> =
        plan.iter().map(|l| (l.cf.as_str(), l)).collect();

    entrees
        .iter()
        .map(|e| {
            let (run, appartenance) = match par_cf.get(e.cf.as_str()) {
                Some(l) if l.retiree() => (l.run_num.clone(), Appartenance::Retire),
                Some(l) => (l.run_num.clone(), Appartenance::Oui),
                None => (String::new(), Appartenance::Non),
            };
            LigneExport {
                run,
                cf: e.cf.clone(),
                jj: e.jj_brut.clone(),
                // `parse_0225_value` rend la valeur SANS son ICD (forme stockée
                // en base) ; le classeur est lu par un humain, on lui rend le
                // « 0225: » qui dit de quel schéma vient l'identifiant.
                adressage: crate::directory::parse_0225_value(&e.participant)
                    .map(|v| format!("0225:{v}"))
                    .unwrap_or_else(|| e.participant.clone()),
                raison_sociale: e.raison_sociale.clone(),
                ctc_status: e.ctc_status.clone(),
                ppf_usable: e.ppf_usable,
                appartenance,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jour(iso: &str) -> chrono::NaiveDate {
        chrono::NaiveDate::parse_from_str(iso, "%Y-%m-%d").unwrap()
    }

    fn entree(cf: &str, jj: &str, ctc: &str) -> LigneEntree {
        LigneEntree {
            cf: cf.into(),
            participant: "iso6523-actorid-upis::0225:12345678900012".into(),
            jj_brut: jj.into(),
            raison_sociale: "ACME".into(),
            pa: "Cegedim".into(),
            resolu: true,
            ctc_ready: ctc == "ready",
            ctc_status: ctc.into(),
            ppf_usable: true,
            in_directory: true,
            resolved_at: 0,
        }
    }

    fn ligne_plan(cf: &str, run: &str) -> LignePlan {
        LignePlan {
            cf: cf.into(),
            participant: "0225:12345678900012".into(),
            jj: 5,
            raison_sociale: "ACME".into(),
            pa: "Cegedim".into(),
            mep_id: 1,
            mep_date: jour("2026-08-01"),
            run_num: run.into(),
            run_date: jour("2026-08-11"),
            origine: crate::plan::Origine::Auto,
            in_directory: true,
            resolved_at: 0,
            planned_at: 0,
            retire: None,
        }
    }

    #[test]
    fn export_couvre_toutes_les_lignes_du_fichier() {
        // Un compte hors du pool (non résolu, sans plateforme) figure quand même
        // au tableau : « l'intégralité des comptes du fichier ».
        let mut hors_pool = entree("CF2", "5", "");
        hors_pool.resolu = false;
        hors_pool.pa = String::new();
        let entrees = vec![entree("CF1", "5", "ready"), hors_pool];
        let plan = vec![ligne_plan("CF1", "R1")];
        let out = lignes(&entrees, &plan);
        assert_eq!(out.len(), 2);
        let cf2 = out.iter().find(|l| l.cf == "CF2").expect("CF2 absent");
        assert_eq!(cf2.appartenance, Appartenance::Non);
        assert_eq!(cf2.run, "", "jamais placé : pas de run");
    }

    #[test]
    fn un_compte_retire_conserve_son_run_et_vaut_retire() {
        let mut retiree = ligne_plan("CF1", "R1");
        retiree.retire = Some(crate::plan::Retrait { le: 1, motif: "clôturé".into() });
        let out = lignes(&[entree("CF1", "5", "ready")], &[retiree]);
        assert_eq!(out[0].appartenance, Appartenance::Retire);
        assert_eq!(out[0].run, "R1", "le run est la trace de ce dont on l'a sorti");
    }

    #[test]
    fn un_compte_au_plan_porte_son_run() {
        let out = lignes(&[entree("CF1", "5", "ready")], &[ligne_plan("CF1", "R1")]);
        assert_eq!(out[0].appartenance, Appartenance::Oui);
        assert_eq!(out[0].run, "R1");
    }

    #[test]
    fn l_adressage_sort_sous_forme_nue() {
        let out = lignes(&[entree("CF1", "5", "ready")], &[]);
        assert_eq!(out[0].adressage, "0225:12345678900012", "forme canonique non réduite");
    }

    #[test]
    fn un_adressage_non_0225_sort_sous_forme_canonique() {
        // Repli : pas de valeur nue à extraire d'un autre schéma.
        let mut e = entree("CF1", "5", "ready");
        e.participant = "iso6523-actorid-upis::0088:7300010000001".into();
        let out = lignes(&[e], &[]);
        assert_eq!(out[0].adressage, "iso6523-actorid-upis::0088:7300010000001");
    }

    #[test]
    fn le_statut_ctc_nest_pas_aplati() {
        let out = lignes(&[entree("CF1", "5", "later")], &[]);
        assert_eq!(out[0].ctc_status, "later");
    }

    #[test]
    fn le_jour_de_cycle_illisible_est_rendu_tel_quel() {
        // Le classeur documente le fichier, il ne le corrige pas.
        let out = lignes(&[entree("CF1", "zzz", "")], &[]);
        assert_eq!(out[0].jj, "zzz");
    }
}
