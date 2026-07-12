use std::collections::HashSet;

pub const DEFAULT_SCHEME: &str = "iso6523-actorid-upis";

/// Forme canonique du participant_id, identique à popaul.py : ajoute le
/// scheme par défaut si le PID est donné en forme courte "0009:x".
pub fn canonical(pid: &str) -> String {
    let pid = pid.trim();
    if pid.contains("::") {
        pid.to_string()
    } else {
        format!("{DEFAULT_SCHEME}::{pid}")
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
