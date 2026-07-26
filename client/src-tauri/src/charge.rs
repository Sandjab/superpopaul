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
use chrono::Datelike;
use chrono::NaiveDate;
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
///
/// Le tri est une **exigence**, pas une préférence : les récurrences sont
/// cherchées après le run de départ *par rang dans le slice*, pas par date.
/// Sur des runs non triés, un compte placé sur un run tardif du slice ne
/// récurre nulle part — silencieusement, sans erreur ni série vide. En
/// production `calendrier::runs_utilisables` garantit l'ordre ; le test
/// `des_runs_non_tries_perdent_les_recurrences` fige ce qui arrive sinon.
pub fn charge(lignes: &[LignePlan], runs: &[RunFacturation]) -> Vec<ChargeRun> {
    let index_par_num: HashMap<&str, usize> = runs
        .iter()
        .enumerate()
        .map(|(i, r)| (r.num.as_str(), i))
        .collect();

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
        let Some(&depart) = index_par_num.get(l.run_num.as_str()) else {
            continue;
        };
        out[depart].premieres += 1;
        let mois_depart = (runs[depart].date.year(), runs[depart].date.month());
        if let Some(porteurs) = porteurs_par_jj.get(&l.jj) {
            for &i in porteurs.iter().filter(|&&i| i > depart) {
                // La règle porte sur le MOIS, pas sur le rang du run : le mois
                // du démarrage, la facture est déjà comptée comme première —
                // y compris quand le run de départ ne couvre pas le JJ du
                // compte, ce qu'un plan relu après édition du runs.csv permet.
                if (runs[i].date.year(), runs[i].date.month()) == mois_depart {
                    continue;
                }
                out[i].recurrences += 1;
            }
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
    fn le_total_additionne_premieres_et_recurrences() {
        let c = ChargeRun {
            num: "R1".into(),
            date: jour("2026-08-11"),
            premieres: 3,
            recurrences: 4,
        };
        assert_eq!(c.total(), 7);
    }

    #[test]
    fn des_runs_non_tries_perdent_les_recurrences() {
        // Contrat documenté, pas comportement souhaitable : `charge` exige des runs
        // triés par date. `calendrier::runs_utilisables` le garantit ; ce test fige
        // ce qui arrive sinon, pour qu'un futur appelant ne le découvre pas en prod.
        let runs = vec![run("R2", "2026-09-08", &[5]), run("R1", "2026-08-11", &[5])];
        let lignes = vec![ligne("CF1", 5, "R1")];
        let c = charge(&lignes, &runs);
        assert_eq!(c[0].recurrences, 0);
        assert_eq!(c[1].recurrences, 0, "runs non triés : la récurrence est perdue");
    }

    #[test]
    fn un_run_multi_jours_de_cycle_fait_recurrer_chaque_jour() {
        // Cas normal : `parse_runs_csv` accepte « 1-5-15 ». Chaque jour de cycle
        // du run porte sa propre facture mensuelle.
        let runs = vec![
            run("R1", "2026-08-11", &[1, 5, 15]),
            run("R2", "2026-09-08", &[1, 5, 15]),
        ];
        let lignes = vec![
            ligne("CF1", 1, "R1"),
            ligne("CF2", 5, "R1"),
            ligne("CF3", 15, "R1"),
        ];
        let c = charge(&lignes, &runs);
        assert_eq!(c[0].premieres, 3);
        assert_eq!(c[1].recurrences, 3, "chaque jour de cycle doit récurrer");
    }

    #[test]
    fn le_passage_dannee_ne_confond_pas_les_mois() {
        // L'horizon du plan est de deux ans : un même mois y revient. Sans
        // l'année dans la clé du porteur, décembre 2027 passerait pour déjà
        // facturé par décembre 2026 et perdrait sa récurrence — le mois seul ne
        // suffit pas à identifier un mois civil.
        let runs = vec![
            run("R1", "2026-12-08", &[5]),
            run("R2", "2027-01-12", &[5]),
            run("R3", "2027-12-07", &[5]),
        ];
        let lignes = vec![ligne("CF1", 5, "R1")];
        let c = charge(&lignes, &runs);
        assert_eq!(c[1].recurrences, 1, "janvier 2027 n'est pas décembre 2026");
        assert_eq!(c[2].recurrences, 1, "décembre 2027 n'est pas décembre 2026");
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
        assert_eq!(
            c[1].recurrences, 1,
            "le premier run de septembre porte la facture"
        );
        assert_eq!(
            c[2].recurrences, 0,
            "le second run du mois ne refacture pas"
        );
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
        assert_eq!(
            c[1].recurrences, 0,
            "aucun run de septembre ne couvre le jour 5"
        );
        assert_eq!(
            c[2].recurrences, 1,
            "octobre reprend, sans rattraper septembre"
        );
    }

    #[test]
    fn les_runs_sans_premiere_facture_portent_les_recurrences() {
        // Régime de croisière : après la dernière MEP, les runs ne démarrent plus
        // personne mais facturent tout le parc. Ils ne doivent pas disparaître.
        let runs = vec![run("R1", "2026-08-11", &[5]), run("R2", "2026-09-08", &[5])];
        let lignes = vec![ligne("CF1", 5, "R1"), ligne("CF2", 5, "R1")];
        let c = charge(&lignes, &runs);
        assert_eq!(c[1].premieres, 0);
        assert_eq!(c[1].recurrences, 2);
        assert_eq!(c[1].total(), 2);
    }

    #[test]
    fn pas_de_seconde_facture_le_mois_du_demarrage() {
        // Le run de départ ne couvre PAS le jour de cycle du compte — cas atteignable
        // quand un plan enregistré est relu après édition des jours de cycle du
        // runs.csv. Le compte ne doit pas facturer deux fois en août pour autant :
        // la règle porte sur le mois civil, pas sur le rang du run.
        let runs = vec![
            run("R1", "2026-08-05", &[3]),
            run("R2", "2026-08-11", &[5]),
            run("R3", "2026-09-08", &[5]),
        ];
        let lignes = vec![ligne("CF1", 5, "R1")];
        let c = charge(&lignes, &runs);
        assert_eq!(c[0].premieres, 1);
        assert_eq!(c[1].recurrences, 0, "seconde facture dans le mois du démarrage");
        assert_eq!(c[2].recurrences, 1, "septembre reprend normalement");
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
        assert_eq!(
            c[0].recurrences, 0,
            "le porteur de septembre lui est antérieur"
        );
        assert_eq!(c[2].recurrences, 1, "octobre le reprend");
    }
}
