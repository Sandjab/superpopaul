//! Verdicts de la résolution unitaire (loupe de l'en-tête). Module PUR :
//! aucune I/O, aucun accès base — l'appelant fournit ce qu'il a lu.
//!
//! Règle qui gouverne tout le module : une source qui ne peut pas répondre ne
//! répond JAMAIS `false`. « Je ne sais pas » et « non » sont deux réponses
//! différentes, et les confondre ferait lire un constat rassurant là où il n'y
//! en a pas (même discipline que `avertissement_ppf_cumulatif`).

use serde::Serialize;

/// Pourquoi une source reste muette.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Muette {
    /// L'annuaire Peppol n'a jamais été chargé.
    AnnuaireNonCharge,
    /// L'annuaire PPF est vide.
    AnnuaireVide,
    /// L'adressage n'est pas un 0225 : les deux annuaires sont indexés sur la
    /// valeur nue 0225, un autre ICD n'y est pas « absent », il n'y est pas
    /// cherchable.
    HorsPerimetre0225,
}

/// Ce que l'annuaire Peppol a à dire. Répond OU se tait, jamais les deux.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "etat", rename_all = "snake_case")]
pub enum Annuaire {
    Repond { in_directory: bool },
    Muette { raison: Muette },
}

/// Verdict de l'annuaire Peppol. `valeur_0225` vient de
/// `directory::parse_0225_value` (None = autre ICD) ; `charge` de
/// `store::peppol_directory_status().is_some()` ; `present` de
/// `store::directory_present`.
pub fn etat_annuaire_peppol(valeur_0225: Option<&str>, charge: bool, present: bool) -> Annuaire {
    match (valeur_0225, charge) {
        (None, _) => Annuaire::Muette { raison: Muette::HorsPerimetre0225 },
        (Some(_), false) => Annuaire::Muette { raison: Muette::AnnuaireNonCharge },
        (Some(_), true) => Annuaire::Repond { in_directory: present },
    }
}

/// Ce que l'annuaire PPF a à dire. Les quatre drapeaux sont ceux de
/// `store::ppf_flags`, recopiés tels quels.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "etat", rename_all = "snake_case")]
pub enum Ppf {
    Repond {
        annuaire_ppf: bool,
        ppf_active: bool,
        pdp_definie: bool,
        ppf_usable: bool,
    },
    Muette { raison: Muette },
}

/// Verdict de l'annuaire PPF. `non_vide` vient de
/// `store::ppf_summary().distinct_addr > 0` ; `flags` de `store::ppf_flags`,
/// qui n'a d'entrée que pour les identifiants trouvés.
pub fn etat_ppf(
    valeur_0225: Option<&str>,
    non_vide: bool,
    flags: Option<&crate::store::PpfFlags>,
) -> Ppf {
    match (valeur_0225, non_vide) {
        (None, _) => Ppf::Muette { raison: Muette::HorsPerimetre0225 },
        (Some(_), false) => Ppf::Muette { raison: Muette::AnnuaireVide },
        (Some(_), true) => match flags {
            Some(f) => Ppf::Repond {
                annuaire_ppf: f.in_ppf,
                ppf_active: f.active,
                pdp_definie: f.pdp_definie,
                ppf_usable: f.usable,
            },
            None => Ppf::Repond {
                annuaire_ppf: false,
                ppf_active: false,
                pdp_definie: false,
                ppf_usable: false,
            },
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::PpfFlags;

    fn flags(in_ppf: bool, active: bool, pdp_definie: bool, usable: bool) -> PpfFlags {
        PpfFlags { in_ppf, active, pdp_definie, usable }
    }

    #[test]
    fn hors_0225_prime_sur_l_annuaire_non_charge() {
        // Les deux causes peuvent coexister. On nomme la STRUCTURELLE : charger
        // l'annuaire ne rendrait pas cet adressage cherchable pour autant.
        assert_eq!(
            etat_annuaire_peppol(None, false, false),
            Annuaire::Muette { raison: Muette::HorsPerimetre0225 }
        );
    }

    #[test]
    fn annuaire_jamais_charge_ne_dit_pas_false() {
        assert_eq!(
            etat_annuaire_peppol(Some("552100554"), false, false),
            Annuaire::Muette { raison: Muette::AnnuaireNonCharge }
        );
    }

    #[test]
    fn annuaire_charge_rend_la_presence() {
        assert_eq!(
            etat_annuaire_peppol(Some("552100554"), true, true),
            Annuaire::Repond { in_directory: true }
        );
        assert_eq!(
            etat_annuaire_peppol(Some("552100554"), true, false),
            Annuaire::Repond { in_directory: false }
        );
    }

    #[test]
    fn ppf_hors_0225_est_muet() {
        assert_eq!(
            etat_ppf(None, true, None),
            Ppf::Muette { raison: Muette::HorsPerimetre0225 }
        );
    }

    #[test]
    fn ppf_vide_ne_dit_pas_false() {
        assert_eq!(
            etat_ppf(Some("552100554"), false, None),
            Ppf::Muette { raison: Muette::AnnuaireVide }
        );
    }

    #[test]
    fn absent_d_un_annuaire_charge_est_un_vrai_non() {
        // `ppf_flags` ne rend une entrée QUE pour les identifiants trouvés :
        // absent de la map, annuaire non vide = il n'y est pas, pour de bon.
        assert_eq!(
            etat_ppf(Some("552100554"), true, None),
            Ppf::Repond {
                annuaire_ppf: false,
                ppf_active: false,
                pdp_definie: false,
                ppf_usable: false,
            }
        );
    }

    #[test]
    fn les_quatre_drapeaux_sont_recopies_sans_recalcul() {
        // ppf_usable ne se déduit pas de active && pdp_definie : le store exige
        // les deux sur la MÊME ligne. Recalculer ici inventerait un `true`.
        assert_eq!(
            etat_ppf(Some("x"), true, Some(&flags(true, true, true, false))),
            Ppf::Repond {
                annuaire_ppf: true,
                ppf_active: true,
                pdp_definie: true,
                ppf_usable: false,
            }
        );
    }
}
