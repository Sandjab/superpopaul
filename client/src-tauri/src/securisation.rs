//! Sécurisation de la montée en charge : par ligne du fichier principal, croise
//! l'état de résolution (réseau Peppol + extension FR prête), le drapeau PPF
//! `usable` et la présence en annuaire Peppol. Agrégat PUR (aucune DB, aucune
//! UI) ; la jointure vit dans `commands::securisation_from_scan`. Chaque niveau
//! de l'entonnoir est un sous-ensemble strict du précédent.

use serde::{Deserialize, Serialize};

/// Drapeaux d'une ligne d'entrée (poids = nombre de lignes du PID canonique).
pub struct LineFlags {
    pub weight: usize,
    pub in_peppol: bool,    // exists_in_peppol == Some(true)
    pub ctc_ready: bool,    // extension FR prête aujourd'hui (output::ctc_status == "ready")
    pub ppf_usable: bool,   // drapeau PPF usable (0225)
    pub in_directory: bool, // présent annuaire Peppol (0225)
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Securisation {
    pub total_lines: usize,
    pub provisionnes: usize,    // in_peppol
    pub avec_extension: usize,  // in_peppol ∧ ctc_ready
    pub coeur: usize,           // in_peppol ∧ ctc_ready ∧ ppf_usable
    pub pleinement: usize,      // coeur ∧ in_directory
    pub ppf_usable_seul: usize, // composante autonome
    pub ctc_ready_seul: usize,  // composante autonome
}

pub fn compute(lines: &[LineFlags]) -> Securisation {
    let mut s = Securisation {
        total_lines: 0,
        provisionnes: 0,
        avec_extension: 0,
        coeur: 0,
        pleinement: 0,
        ppf_usable_seul: 0,
        ctc_ready_seul: 0,
    };
    for f in lines {
        s.total_lines += f.weight;
        if f.in_peppol {
            s.provisionnes += f.weight;
        }
        if f.in_peppol && f.ctc_ready {
            s.avec_extension += f.weight;
        }
        if f.in_peppol && f.ctc_ready && f.ppf_usable {
            s.coeur += f.weight;
        }
        if f.in_peppol && f.ctc_ready && f.ppf_usable && f.in_directory {
            s.pleinement += f.weight;
        }
        if f.ppf_usable {
            s.ppf_usable_seul += f.weight;
        }
        if f.ctc_ready {
            s.ctc_ready_seul += f.weight;
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(weight: usize, in_peppol: bool, ctc_ready: bool, ppf_usable: bool, in_directory: bool) -> LineFlags {
        LineFlags { weight, in_peppol, ctc_ready, ppf_usable, in_directory }
    }

    // Emboîtement : tout-vrai compte à tous les niveaux.
    #[test]
    fn emboitement_tout_vrai() {
        let s = compute(&[line(1, true, true, true, true)]);
        assert_eq!((s.provisionnes, s.avec_extension, s.coeur, s.pleinement), (1, 1, 1, 1));
    }

    // Sans in_directory : cœur oui, pleinement non.
    #[test]
    fn sans_annuaire_pas_pleinement() {
        let s = compute(&[line(1, true, true, true, false)]);
        assert_eq!(s.coeur, 1);
        assert_eq!(s.pleinement, 0);
    }

    // Sans ppf_usable : avec_extension oui, cœur non.
    #[test]
    fn sans_ppf_usable_pas_coeur() {
        let s = compute(&[line(1, true, true, false, true)]);
        assert_eq!(s.avec_extension, 1);
        assert_eq!(s.coeur, 0);
    }

    // Pondération par le poids.
    #[test]
    fn ponderation() {
        let s = compute(&[line(4, true, true, true, true)]);
        assert_eq!(s.total_lines, 4);
        assert_eq!(s.pleinement, 4);
    }

    // Composantes autonomes : ppf_usable/ctc_ready comptent sans in_peppol.
    #[test]
    fn composantes_autonomes() {
        let s = compute(&[line(1, false, true, true, true)]); // pas provisionné
        assert_eq!(s.provisionnes, 0);
        assert_eq!(s.avec_extension, 0);
        assert_eq!(s.coeur, 0);
        assert_eq!(s.ppf_usable_seul, 1);
        assert_eq!(s.ctc_ready_seul, 1);
    }

    // Non-0225 (ppf_usable=false, in_directory=false) mais provisionné+extension :
    // entre dans l'entonnoir haut, jamais dans cœur/pleinement.
    #[test]
    fn non_0225_reste_en_haut_de_l_entonnoir() {
        let s = compute(&[line(3, true, true, false, false)]);
        assert_eq!(s.provisionnes, 3);
        assert_eq!(s.avec_extension, 3);
        assert_eq!(s.coeur, 0);
        assert_eq!(s.pleinement, 0);
    }

    #[test]
    fn total_vide() {
        let s = compute(&[]);
        assert_eq!(s.total_lines, 0);
        assert_eq!(s.provisionnes, 0);
    }
}
