//! Répartition des lignes (CF) du fichier par plateforme de dématérialisation
//! (Point d'Accès Peppol). Agrégat PUR : aucune DB, aucune UI — la partie
//! impure (load_map) vit dans `commands.rs::repartition_from_scan`. Comptage
//! PAR LIGNE : chaque adressage unique est pondéré par son nombre de lignes.

use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaCount {
    pub nom: String,
    pub lignes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repartition {
    /// Dénominateur des % : total des lignes du fichier (plateformes + sans).
    pub total_lignes: u64,
    /// Toutes les plateformes, triées par lignes décroissant (départage par nom).
    pub plateformes: Vec<PaCount>,
    /// Lignes sans plateforme : adressage non résolu, ou résolu sans PA.
    pub sans_plateforme: u64,
}

/// Clé de regroupement d'une résolution : nom de PA, repli sur le code si le
/// nom est absent/vide, `None` si ni l'un ni l'autre (→ « sans plateforme »).
pub fn pa_key(pa_name: Option<&str>, pa_code: Option<&str>) -> Option<String> {
    pa_name
        .filter(|s| !s.is_empty())
        .or(pa_code.filter(|s| !s.is_empty()))
        .map(str::to_string)
}

/// Agrège les entrées `(clé PA ou None, nombre de lignes)` : somme par PA,
/// `None` → `sans_plateforme`, tri décroissant (départage alphabétique stable).
pub fn compute(entrees: &[(Option<String>, u64)]) -> Repartition {
    let mut par_pa: BTreeMap<String, u64> = BTreeMap::new();
    let mut sans_plateforme = 0u64;
    let mut total_lignes = 0u64;
    for (cle, n) in entrees {
        total_lignes += n;
        match cle {
            Some(nom) => *par_pa.entry(nom.clone()).or_insert(0) += n,
            None => sans_plateforme += n,
        }
    }
    // BTreeMap → ordre alphabétique ; sort_by_key stable préserve ce départage
    // à égalité d'effectif (même discipline que telemetry::ranked).
    let mut plateformes: Vec<PaCount> = par_pa
        .into_iter()
        .map(|(nom, lignes)| PaCount { nom, lignes })
        .collect();
    plateformes.sort_by_key(|p| std::cmp::Reverse(p.lignes));
    Repartition { total_lignes, plateformes, sans_plateforme }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pa_key_prefere_le_nom() {
        assert_eq!(pa_key(Some("Cegedim"), Some("PA01")), Some("Cegedim".into()));
    }

    #[test]
    fn pa_key_replie_sur_le_code_si_nom_vide_ou_absent() {
        assert_eq!(pa_key(Some(""), Some("PA01")), Some("PA01".into()));
        assert_eq!(pa_key(None, Some("PA01")), Some("PA01".into()));
    }

    #[test]
    fn pa_key_none_si_ni_nom_ni_code() {
        assert_eq!(pa_key(None, None), None);
        assert_eq!(pa_key(Some(""), Some("")), None);
    }

    #[test]
    fn compute_agrege_les_lignes_par_pa() {
        let e = vec![
            (Some("Cegedim".to_string()), 10u64),
            (Some("Docaposte".to_string()), 4),
            (Some("Cegedim".to_string()), 6),
        ];
        let r = compute(&e);
        assert_eq!(r.total_lignes, 20);
        assert_eq!(r.sans_plateforme, 0);
        assert_eq!(
            r.plateformes,
            vec![
                PaCount { nom: "Cegedim".into(), lignes: 16 },
                PaCount { nom: "Docaposte".into(), lignes: 4 },
            ]
        );
    }

    #[test]
    fn compute_none_compte_en_sans_plateforme() {
        let e = vec![(Some("Cegedim".to_string()), 5u64), (None, 3)];
        let r = compute(&e);
        assert_eq!(r.sans_plateforme, 3);
        assert_eq!(r.total_lignes, 8);
        assert_eq!(r.plateformes, vec![PaCount { nom: "Cegedim".into(), lignes: 5 }]);
    }

    #[test]
    fn compute_trie_decroissant_puis_alphabetique() {
        // À effectif égal (A et B = 5), l'ordre alphabétique départage.
        let e = vec![
            (Some("B".to_string()), 5u64),
            (Some("A".to_string()), 5),
            (Some("C".to_string()), 9),
        ];
        let r = compute(&e);
        let noms: Vec<&str> = r.plateformes.iter().map(|p| p.nom.as_str()).collect();
        assert_eq!(noms, vec!["C", "A", "B"]);
    }
}
