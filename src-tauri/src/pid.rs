use std::collections::HashSet;

pub const DEFAULT_SCHEME: &str = "iso6523-actorid-upis";
pub const DEFAULT_ICD: &str = "0225";

/// Forme canonique du participant_id, identique à popaul.py.
/// - "scheme::icd:x" : déjà canonique, inchangé ;
/// - "icd:x" : scheme par défaut ajouté ;
/// - adressage brut sans « : » (SIREN, SIREN_SIRET, SIREN_SIRET_CODEROUTAGE,
///   SIREN_SUFFIXELIBRE) : préfixé de l'ICD français 0225 — sans lui, le hash
///   SML porte sur la valeur nue et tout ressortirait « absent de Peppol ».
pub fn canonical(pid: &str) -> String {
    let pid = pid.trim();
    if pid.contains("::") {
        pid.to_string()
    } else if pid.contains(':') {
        format!("{DEFAULT_SCHEME}::{pid}")
    } else {
        format!("{DEFAULT_SCHEME}::{DEFAULT_ICD}:{pid}")
    }
}

/// Canonicalise, ignore les valeurs vides, déduplique en conservant l'ordre
/// de première apparition.
pub fn unique_canonical<I: IntoIterator<Item = String>>(values: I) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for v in values {
        let v = v.trim();
        if v.is_empty() {
            continue;
        }
        let c = canonical(v);
        if seen.insert(c.clone()) {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_ajoute_le_scheme_par_defaut() {
        assert_eq!(
            canonical("0009:552100554"),
            "iso6523-actorid-upis::0009:552100554"
        );
    }

    #[test]
    fn canonical_conserve_un_pid_deja_complet() {
        assert_eq!(
            canonical("iso6523-actorid-upis::0009:552100554"),
            "iso6523-actorid-upis::0009:552100554"
        );
    }

    #[test]
    fn canonical_trimme() {
        assert_eq!(canonical("  0009:1  "), "iso6523-actorid-upis::0009:1");
    }

    #[test]
    fn canonical_prefixe_0225_sur_un_adressage_brut_sans_icd() {
        // Règle métier : l'adressage brut (SIREN, SIREN_SIRET,
        // SIREN_SIRET_CODEROUTAGE, SIREN_SUFFIXELIBRE — jamais de « : »)
        // se canonicalise TOUJOURS en iso6523-actorid-upis::0225:<brut>.
        // Sans l'ICD, le hash SML porterait sur la valeur nue → faux négatif
        // systématique (« absent de Peppol » pour un inscrit).
        for brut in ["552100554", "552100554_55210055400013",
                     "552100554_55210055400013_ROUTAGE1", "552100554_SERVICE_ACHATS"] {
            assert_eq!(canonical(brut), format!("iso6523-actorid-upis::0225:{brut}"));
        }
    }

    #[test]
    fn canonical_respecte_un_icd_explicite() {
        // Un « : » simple signale un ICD déjà présent : on n'empile pas 0225.
        assert_eq!(canonical("0225:552100554"), "iso6523-actorid-upis::0225:552100554");
        assert_eq!(canonical("0009:552100554"), "iso6523-actorid-upis::0009:552100554");
    }

    #[test]
    fn unique_canonical_deduplique_en_gardant_l_ordre() {
        let vals = [
            "0009:1",
            "iso6523-actorid-upis::0009:1",
            "",
            "0009:2",
            "0009:1",
        ]
        .map(String::from);
        assert_eq!(
            unique_canonical(vals),
            vec![
                "iso6523-actorid-upis::0009:1".to_string(),
                "iso6523-actorid-upis::0009:2".to_string()
            ]
        );
    }
}
