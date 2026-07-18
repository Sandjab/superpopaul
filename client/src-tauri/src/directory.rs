//! Ingestion de l'annuaire Peppol (fichier export-all-participants.csv) —
//! fonctionnalité CLIENT-ONLY : aucune parité avec cli/popaul.py.
//! On ne charge que l'adressage 0225 (SIRENE français), stocké sans son
//! préfixe de scheme/ICD.

use std::io::Read;

/// Préfixe des Participant ID d'adressage 0225. Le scheme est l'invariant de
/// `pid::DEFAULT_SCHEME` ; le « 0225 » est l'exigence explicite du chantier
/// (test `prefixe_coherent_avec_pid` en garde-fou contre la dérive).
const PREFIX_0225: &str = "iso6523-actorid-upis::0225:";

/// URL d'export de l'annuaire Peppol (Télécharger).
pub const DIRECTORY_URL: &str = "https://directory.peppol.eu/export/participants-csv";

/// Renvoie la valeur (partie après `iso6523-actorid-upis::0225:`) si le
/// Participant ID est en 0225, sinon `None`. Verbatim : les suffixes
/// (`_replyto`, `_cdv_…`, `_SIRET`) sont conservés. Préfixe seul sans valeur → `None`.
pub fn parse_0225_value(participant_id: &str) -> Option<String> {
    match participant_id.trim().strip_prefix(PREFIX_0225) {
        Some(rest) if !rest.is_empty() => Some(rest.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefixe_coherent_avec_pid() {
        // Garde-fou : le préfixe 0225 doit rester aligné sur le scheme par
        // défaut de la canonicalisation.
        assert_eq!(PREFIX_0225, format!("{}::0225:", crate::pid::DEFAULT_SCHEME));
    }

    #[test]
    fn extrait_la_valeur_0225_nue() {
        assert_eq!(
            parse_0225_value("iso6523-actorid-upis::0225:000122308"),
            Some("000122308".to_string())
        );
    }

    #[test]
    fn conserve_les_suffixes_techniques_verbatim() {
        // Les entrées à suffixe (_replyto, _cdv_…) sont de vrais inscrits :
        // on les garde tels quels, on ne normalise pas.
        assert_eq!(
            parse_0225_value("iso6523-actorid-upis::0225:000009777_0054_replyto"),
            Some("000009777_0054_replyto".to_string())
        );
        assert_eq!(
            parse_0225_value("iso6523-actorid-upis::0225:005580436_cdv_d6a4bbca"),
            Some("005580436_cdv_d6a4bbca".to_string())
        );
    }

    #[test]
    fn ignore_les_autres_schemes() {
        assert_eq!(parse_0225_value("iso6523-actorid-upis::0002:000126010"), None);
        assert_eq!(parse_0225_value("iso6523-actorid-upis::0009:552100554"), None);
    }

    #[test]
    fn ignore_le_prefixe_seul_et_l_entete() {
        assert_eq!(parse_0225_value("iso6523-actorid-upis::0225:"), None);
        assert_eq!(parse_0225_value("Participant ID"), None);
        assert_eq!(parse_0225_value(""), None);
    }

    #[test]
    fn trimme_l_entree() {
        assert_eq!(
            parse_0225_value("  iso6523-actorid-upis::0225:000122308  "),
            Some("000122308".to_string())
        );
    }
}
