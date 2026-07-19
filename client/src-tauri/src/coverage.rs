//! Couverture déclarative des identifiants d'entrée par les annuaires chargés
//! (Peppol `in_directory` + 4 drapeaux PPF). Agrégat PUR : aucune DB, aucune
//! UI — la partie impure (scan CSV, requêtes store) vit dans `commands.rs`.
//! Comptage PAR LIGNE (chaque identifiant unique pondéré par son nombre de
//! lignes) et PAR ÉLIGIBILITÉ 0225 (seuls les identifiants 0225 comptent au
//! dénominateur ; les autres sont « non applicables »).

use crate::store::PpfFlags;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PeppolCoverage {
    /// Lignes 0225 présentes dans l'annuaire Peppol chargé.
    pub present: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PpfCoverage {
    pub present: usize,     // annuaire_ppf
    pub active: usize,      // ppf_active
    pub pdp_definie: usize, // pdp_definie
    pub usable: usize,      // ppf_usable
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Coverage {
    pub total_lines: usize,
    /// Dénominateur : lignes portant un identifiant 0225.
    pub eligible_0225: usize,
    /// Lignes non-0225 (hors calcul), affichées en clair.
    pub non_applicable: usize,
    /// `None` = annuaire Peppol non chargé (bloc masqué).
    pub peppol: Option<PeppolCoverage>,
    /// `None` = annuaire PPF non chargé (bloc masqué).
    pub ppf: Option<PpfCoverage>,
}

/// Agrège la couverture.
///
/// - `eligible` : `(valeur 0225, nombre de lignes)` pour chaque identifiant
///   0225 unique de l'entrée.
/// - `non_applicable` : total des lignes non-0225.
/// - `present` : `Some(ensemble des valeurs présentes en annuaire Peppol)`, ou
///   `None` si l'annuaire Peppol n'est pas chargé (gate).
/// - `ppf` : `Some(map identifiant → drapeaux)`, ou `None` si l'annuaire PPF
///   n'est pas chargé. Une valeur absente de la map compte 0 (drapeaux défaut).
pub fn compute(
    eligible: &[(String, usize)],
    non_applicable: usize,
    present: Option<&HashSet<String>>,
    ppf: Option<&HashMap<String, PpfFlags>>,
) -> Coverage {
    let eligible_0225: usize = eligible.iter().map(|(_, n)| n).sum();

    let peppol = present.map(|set| PeppolCoverage {
        present: eligible
            .iter()
            .filter(|(v, _)| set.contains(v))
            .map(|(_, n)| n)
            .sum(),
    });

    let ppf = ppf.map(|map| {
        let mut c = PpfCoverage { present: 0, active: 0, pdp_definie: 0, usable: 0 };
        for (v, n) in eligible {
            let f = map.get(v).copied().unwrap_or_default();
            if f.in_ppf {
                c.present += n;
            }
            if f.active {
                c.active += n;
            }
            if f.pdp_definie {
                c.pdp_definie += n;
            }
            if f.usable {
                c.usable += n;
            }
        }
        c
    });

    Coverage {
        total_lines: eligible_0225 + non_applicable,
        eligible_0225,
        non_applicable,
        peppol,
        ppf,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flags(in_ppf: bool, active: bool, pdp_definie: bool, usable: bool) -> PpfFlags {
        PpfFlags { in_ppf, active, pdp_definie, usable }
    }

    // Peppol seul chargé → bloc PPF masqué, présence Peppol comptée par ligne.
    #[test]
    fn peppol_seul_charge() {
        let eligible = vec![("a".into(), 2usize), ("b".into(), 1usize)];
        let present: HashSet<String> = ["a".to_string()].into_iter().collect();
        let cov = compute(&eligible, 0, Some(&present), None);
        assert_eq!(cov.peppol, Some(PeppolCoverage { present: 2 })); // "a" sur 2 lignes
        assert_eq!(cov.ppf, None);
        assert_eq!(cov.eligible_0225, 3);
    }

    // PPF seul chargé → bloc Peppol masqué.
    #[test]
    fn ppf_seul_charge() {
        let eligible = vec![("a".into(), 1usize)];
        let mut map = HashMap::new();
        map.insert("a".to_string(), flags(true, true, true, true));
        let cov = compute(&eligible, 0, None, Some(&map));
        assert_eq!(cov.peppol, None);
        assert_eq!(cov.ppf, Some(PpfCoverage { present: 1, active: 1, pdp_definie: 1, usable: 1 }));
    }

    // Aucun annuaire → les deux blocs masqués.
    #[test]
    fn aucun_annuaire() {
        let cov = compute(&[("a".into(), 1)], 0, None, None);
        assert_eq!(cov.peppol, None);
        assert_eq!(cov.ppf, None);
    }

    // Dénominateur : les non-applicables ne comptent ni au num ni au dénom.
    #[test]
    fn denominateur_eligibles_et_non_applicables() {
        let eligible = vec![("a".into(), 3usize)];
        let present: HashSet<String> = ["a".to_string()].into_iter().collect();
        let cov = compute(&eligible, 7, Some(&present), None);
        assert_eq!(cov.eligible_0225, 3);
        assert_eq!(cov.non_applicable, 7);
        assert_eq!(cov.total_lines, 10);
        assert_eq!(cov.peppol.unwrap().present, 3);
    }

    // Comptage par ligne : même valeur sur N lignes comptée N fois.
    #[test]
    fn comptage_par_ligne() {
        let eligible = vec![("dup".into(), 4usize)];
        let present: HashSet<String> = ["dup".to_string()].into_iter().collect();
        let cov = compute(&eligible, 0, Some(&present), None);
        assert_eq!(cov.peppol.unwrap().present, 4);
    }

    // Entonnoir PPF : usable ≠ (active ∧ pdp_definie). Une valeur active + PDP
    // réelle mais sur des lignes DIFFÉRENTES compte en active et pdp_definie,
    // PAS en usable (miroir du cas store `id_split`).
    #[test]
    fn entonnoir_usable_strict() {
        let eligible = vec![("split".into(), 1usize)];
        let mut map = HashMap::new();
        map.insert("split".to_string(), flags(true, true, true, false)); // usable=false
        let cov = compute(&eligible, 0, None, Some(&map));
        let p = cov.ppf.unwrap();
        assert_eq!(p.active, 1);
        assert_eq!(p.pdp_definie, 1);
        assert_eq!(p.usable, 0, "usable exige la MÊME ligne active+PDP réelle");
    }

    // Éligible mais absent de l'annuaire → compté au dénom, 0 au num (distinct
    // de « non applicable »).
    #[test]
    fn eligible_absent_de_l_annuaire() {
        let eligible = vec![("absent".into(), 5usize)];
        let present: HashSet<String> = HashSet::new(); // annuaire chargé mais vide
        let map: HashMap<String, PpfFlags> = HashMap::new();
        let cov = compute(&eligible, 0, Some(&present), Some(&map));
        assert_eq!(cov.eligible_0225, 5);
        assert_eq!(cov.peppol.unwrap().present, 0);
        assert_eq!(cov.ppf.unwrap(), PpfCoverage { present: 0, active: 0, pdp_definie: 0, usable: 0 });
    }

    // Round-trip serde (contrat cockpit JS ↔ rapport).
    #[test]
    fn round_trip_serde() {
        let cov = Coverage {
            total_lines: 1000,
            eligible_0225: 900,
            non_applicable: 100,
            peppol: Some(PeppolCoverage { present: 812 }),
            ppf: Some(PpfCoverage { present: 640, active: 590, pdp_definie: 610, usable: 570 }),
        };
        let json = serde_json::to_string(&cov).unwrap();
        let back: Coverage = serde_json::from_str(&json).unwrap();
        assert_eq!(cov, back);
        // Contrat de noms de champs consommés côté JS.
        assert!(json.contains("\"eligible_0225\""));
        assert!(json.contains("\"non_applicable\""));
        assert!(json.contains("\"pdp_definie\""));
        assert!(json.contains("\"usable\""));
    }
}
