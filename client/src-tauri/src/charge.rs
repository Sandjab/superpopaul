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
// `Datelike`, `HashMap` et `HashSet` ne servent qu'aux tâches 2 et 3
// (premières factures, récurrences mensuelles) — pas encore à celle-ci.
#[allow(unused_imports)]
use chrono::Datelike;
#[allow(unused_imports)]
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::Origine;

    // `jour`, `run` et `ligne` ne sont pas encore appelées : le seul test de
    // cette tâche passe des tranches vides. Elles servent aux tâches 2 et 3.
    #[allow(dead_code)]
    fn jour(iso: &str) -> NaiveDate {
        NaiveDate::parse_from_str(iso, "%Y-%m-%d").unwrap()
    }

    #[allow(dead_code)]
    fn run(num: &str, date: &str, jjs: &[u8]) -> RunFacturation {
        RunFacturation {
            num: num.into(),
            date: jour(date),
            jjs: jjs.to_vec(),
            exclu: false,
        }
    }

    /// Une ligne de plan : seuls `jj` et `run_num` comptent pour ce module.
    #[allow(dead_code)]
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
