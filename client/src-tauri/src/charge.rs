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
use chrono::NaiveDate;
// `Datelike` ne sert qu'aux récurrences mensuelles — pas encore à cette tâche.
#[allow(unused_imports)]
use chrono::Datelike;
use std::collections::HashMap;

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

/// Factures émises à chaque run.
///
/// `lignes` : lignes **actives** du plan. `runs` : runs **retenus**, triés par
/// date croissante (contrat de `calendrier::runs_utilisables`).
///
/// Règle : un compte facture **une fois par mois civil**, au premier run du
/// mois dont les jours de cycle couvrent le sien.
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

    #[test]
    fn pas_de_recurrence_avant_le_demarrage() {
        // Un seul run : le compte y démarre. Il ne peut pas y « revenir ».
        let runs = vec![run("R1", "2026-08-11", &[5])];
        let lignes = vec![ligne("CF1", 5, "R1"), ligne("CF2", 5, "R1")];
        let c = charge(&lignes, &runs);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].premieres, 2);
        assert_eq!(
            c[0].recurrences, 0,
            "une première facture n'est pas une récurrence"
        );
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
}
