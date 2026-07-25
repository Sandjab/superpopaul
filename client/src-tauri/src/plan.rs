//! Plan de charge : pool éligible, quotas, rampe, allocation aux Runs de
//! Facturation. Agrégat PUR (aucune DB, aucune UI) — la jointure vit dans
//! `commands::plan_pool_from_scan`, comme `coverage_from_scan` et
//! `securisation_from_scan`.
//!
//! Éligibilité (critères durs) : statut CTC **prêt** ET **PPF utilisable**
//! (motif actif ET PDP réelle sur la MÊME ligne d'annuaire). `ppf_active`
//! seul ne suffit pas : il laisserait entrer des comptes dont la seule ligne
//! active pointe vers une PDP fictive.

use std::collections::HashMap;
use std::collections::HashSet;

/// Une ligne du fichier d'entrée, jointures déjà faites par l'appelant.
/// Les drapeaux sont des booléens (et non des dates brutes) pour la même
/// raison que `securisation::LineFlags` : le module reste pur et trivialement
/// testable, le calcul temporel vit à la frontière.
#[derive(Debug, Clone)]
pub struct LigneEntree {
    pub cf: String,
    /// Adressage sous forme canonique (longue).
    pub participant: String,
    /// Jour de cycle tel qu'il figure dans le CSV — validé ici, pas avant.
    pub jj_brut: String,
    pub raison_sociale: String,
    /// Plateforme (`repartition::pa_key` déjà appliqué). Vide si inconnue :
    /// l'appelant met alors `resolu = false` — un compte sans plateforme
    /// identifiable ne peut pas entrer dans les quotas, qui sont par PA.
    pub pa: String,
    /// Résolu en base, `api_status == "ok"`, avec une plateforme identifiée.
    pub resolu: bool,
    /// `output::ctc_status == "ready"` au moment du calcul.
    pub ctc_ready: bool,
    pub ppf_usable: bool,
    pub in_directory: bool,
    pub resolved_at: i64,
}

/// Un compte de facturation retenu au pool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CfCandidat {
    pub cf: String,
    pub participant: String,
    pub jj: u8,
    pub raison_sociale: String,
    pub pa: String,
    pub in_directory: bool,
    pub resolved_at: i64,
}

/// Entonnoir d'éligibilité. **Tous les champs sont des effectifs RESTANTS**,
/// monotones décroissants : la perte de chaque marche se lit par différence
/// avec la précédente. Aucune marche ne doit disparaître en silence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize)]
pub struct Funnel {
    /// Toutes les lignes du fichier, doublons compris.
    pub lignes: u64,
    pub cf_distincts: u64,
    pub jj_valide: u64,
    pub resolus: u64,
    pub ctc_ready: u64,
    pub ppf_usable: u64,
    /// Après retrait des plateformes exclues — c'est le pool.
    pub eligibles: u64,
}

/// Construit le pool éligible et l'entonnoir.
///
/// Dédoublonnage : deux lignes STRICTEMENT identiques pour un même CF sont
/// fondues en silence. En revanche un même CF porté par deux jours de cycle
/// (ou deux adressages) différents est une incohérence de données, pas un cas
/// nominal : **refus fort**, avec un message nommant le compte et les valeurs
/// en conflit.
///
/// Les comptes rendus sont dans l'ordre d'apparition ; le tri de priorité est
/// l'affaire de l'allocation.
pub fn construire_pool(
    entrees: &[LigneEntree],
    pa_exclues: &HashSet<String>,
) -> Result<(Vec<CfCandidat>, Funnel), String> {
    let mut f = Funnel {
        lignes: entrees.len() as u64,
        ..Funnel::default()
    };

    // 1) Dédoublonnage par CF. Première occurrence retenue ; toute divergence
    //    sur le JJ ou l'adressage est une incohérence de données → refus fort.
    let mut vus: HashMap<&str, &LigneEntree> = HashMap::new();
    let mut ordre: Vec<&LigneEntree> = Vec::new();
    for l in entrees {
        match vus.get(l.cf.as_str()) {
            None => {
                vus.insert(&l.cf, l);
                ordre.push(l);
            }
            Some(prem) => {
                if prem.jj_brut.trim() != l.jj_brut.trim() {
                    return Err(format!(
                        "compte de facturation « {} » : deux jours de cycle différents \
                         dans le fichier ({} et {}) — corrige la donnée avant de planifier",
                        l.cf,
                        prem.jj_brut.trim(),
                        l.jj_brut.trim()
                    ));
                }
                if prem.participant != l.participant {
                    return Err(format!(
                        "compte de facturation « {} » : deux adressages différents \
                         dans le fichier ({} et {}) — corrige la donnée avant de planifier",
                        l.cf, prem.participant, l.participant
                    ));
                }
            }
        }
    }
    f.cf_distincts = ordre.len() as u64;

    // 2) Entonnoir : chaque marche retire ce qu'elle doit, et rien d'autre.
    let mut pool = Vec::new();
    for l in ordre {
        let Some(jj) = parse_jj(&l.jj_brut) else {
            continue;
        };
        f.jj_valide += 1;
        if !l.resolu {
            continue;
        }
        f.resolus += 1;
        if !l.ctc_ready {
            continue;
        }
        f.ctc_ready += 1;
        if !l.ppf_usable {
            continue;
        }
        f.ppf_usable += 1;
        if pa_exclues.contains(&l.pa) {
            continue;
        }
        pool.push(CfCandidat {
            cf: l.cf.clone(),
            participant: l.participant.clone(),
            jj,
            raison_sociale: l.raison_sociale.clone(),
            pa: l.pa.clone(),
            in_directory: l.in_directory,
            resolved_at: l.resolved_at,
        });
    }
    f.eligibles = pool.len() as u64;
    Ok((pool, f))
}

/// Jour de cycle : entier 1..=31, espaces tolérés. Tout le reste est écarté
/// (et compté par le funnel) — un JJ hors bornes ne correspond à aucun run.
fn parse_jj(brut: &str) -> Option<u8> {
    let jj: u8 = brut.trim().parse().ok()?;
    (1..=31).contains(&jj).then_some(jj)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ligne « tout va bien » : éligible de bout en bout.
    fn ligne(cf: &str, jj: &str, pa: &str) -> LigneEntree {
        LigneEntree {
            cf: cf.into(),
            participant: format!("iso6523-actorid-upis::0225:{cf}"),
            jj_brut: jj.into(),
            raison_sociale: "ACME".into(),
            pa: pa.into(),
            resolu: true,
            ctc_ready: true,
            ppf_usable: true,
            in_directory: true,
            resolved_at: 1_700_000_000,
        }
    }

    fn sans_exclusion() -> HashSet<String> {
        HashSet::new()
    }

    #[test]
    fn pool_nominal() {
        let e = vec![ligne("CF1", "5", "Cegedim"), ligne("CF2", "12", "Esker")];
        let (pool, f) = construire_pool(&e, &sans_exclusion()).unwrap();
        assert_eq!(pool.len(), 2);
        assert_eq!(pool[0].cf, "CF1");
        assert_eq!(pool[0].jj, 5);
        assert_eq!(f.lignes, 2);
        assert_eq!(f.eligibles, 2);
    }

    #[test]
    fn funnel_monotone_decroissant() {
        let mut e = vec![ligne("CF1", "5", "PA")];
        e.push(ligne("CF2", "99", "PA")); // JJ hors bornes
        e.push({
            let mut l = ligne("CF3", "5", "PA");
            l.resolu = false;
            l
        });
        e.push({
            let mut l = ligne("CF4", "5", "PA");
            l.ctc_ready = false;
            l
        });
        e.push({
            let mut l = ligne("CF5", "5", "PA");
            l.ppf_usable = false;
            l
        });
        let (_, f) = construire_pool(&e, &sans_exclusion()).unwrap();
        assert!(f.lignes >= f.cf_distincts, "{f:?}");
        assert!(f.cf_distincts >= f.jj_valide, "{f:?}");
        assert!(f.jj_valide >= f.resolus, "{f:?}");
        assert!(f.resolus >= f.ctc_ready, "{f:?}");
        assert!(f.ctc_ready >= f.ppf_usable, "{f:?}");
        assert!(f.ppf_usable >= f.eligibles, "{f:?}");
    }

    #[test]
    fn ppf_active_sans_usable_est_exclu() {
        // LE test qui encode la décision : `ppf_active` ne suffit pas — un
        // compte dont la seule ligne active pointe vers une PDP fictive n'est
        // pas utilisable. Seul `ppf_usable` (motif actif ET pdp réelle sur la
        // MÊME ligne) ouvre le pool.
        let mut l = ligne("CF1", "5", "PA");
        l.ppf_usable = false;
        let (pool, f) = construire_pool(&[l], &sans_exclusion()).unwrap();
        assert!(pool.is_empty(), "un CF non `ppf_usable` ne doit jamais entrer");
        assert_eq!(f.ctc_ready, 1, "il a bien franchi la marche précédente");
        assert_eq!(f.ppf_usable, 0);
    }

    #[test]
    fn ctc_non_pret_est_exclu() {
        let mut l = ligne("CF1", "5", "PA");
        l.ctc_ready = false; // « later » ou « expired »
        let (pool, f) = construire_pool(&[l], &sans_exclusion()).unwrap();
        assert!(pool.is_empty());
        assert_eq!(f.resolus, 1);
        assert_eq!(f.ctc_ready, 0);
    }

    #[test]
    fn adressage_non_resolu_est_exclu() {
        let mut l = ligne("CF1", "5", "PA");
        l.resolu = false;
        let (pool, f) = construire_pool(&[l], &sans_exclusion()).unwrap();
        assert!(pool.is_empty());
        assert_eq!(f.jj_valide, 1);
        assert_eq!(f.resolus, 0);
    }

    #[test]
    fn jj_invalide_est_compte_jamais_silencieux() {
        // Un JJ absent, hors bornes ou non numérique sort du pool, mais la
        // marche du funnel doit le montrer.
        for brut in ["", "0", "32", "abc", " ", "5.5"] {
            let l = ligne("CF1", brut, "PA");
            let (pool, f) = construire_pool(&[l], &sans_exclusion()).unwrap();
            assert!(pool.is_empty(), "JJ « {brut} » ne doit pas passer");
            assert_eq!(f.cf_distincts, 1, "JJ « {brut} »");
            assert_eq!(f.jj_valide, 0, "JJ « {brut} »");
        }
    }

    #[test]
    fn jj_accepte_les_bornes_et_les_espaces() {
        for brut in ["1", "31", " 5 "] {
            let l = ligne("CF1", brut, "PA");
            let (pool, _) = construire_pool(&[l], &sans_exclusion()).unwrap();
            assert_eq!(pool.len(), 1, "JJ « {brut} » doit passer");
        }
    }

    #[test]
    fn doublon_strict_est_fondu_en_silence() {
        let e = vec![ligne("CF1", "5", "PA"), ligne("CF1", "5", "PA")];
        let (pool, f) = construire_pool(&e, &sans_exclusion()).unwrap();
        assert_eq!(pool.len(), 1, "un seul compte");
        assert_eq!(f.lignes, 2, "les deux lignes sont comptées");
        assert_eq!(f.cf_distincts, 1);
    }

    #[test]
    fn jj_divergents_pour_un_meme_cf_est_un_refus_fort() {
        let e = vec![ligne("CF1", "5", "PA"), ligne("CF1", "12", "PA")];
        let err = construire_pool(&e, &sans_exclusion()).unwrap_err();
        assert!(err.contains("CF1"), "le compte doit être nommé : {err}");
        assert!(err.contains('5') && err.contains("12"), "valeurs en conflit : {err}");
    }

    #[test]
    fn adressages_divergents_pour_un_meme_cf_est_un_refus_fort() {
        // Même nature d'incohérence que les JJ : « dédoublonner » reviendrait
        // à choisir un adressage au hasard pour ce compte.
        let mut a = ligne("CF1", "5", "PA");
        let mut b = ligne("CF1", "5", "PA");
        a.participant = "iso6523-actorid-upis::0225:111".into();
        b.participant = "iso6523-actorid-upis::0225:222".into();
        let err = construire_pool(&[a, b], &sans_exclusion()).unwrap_err();
        assert!(err.contains("CF1"), "{err}");
    }

    #[test]
    fn plateforme_exclue_retire_ses_comptes() {
        let e = vec![ligne("CF1", "5", "Cegedim"), ligne("CF2", "5", "Esker")];
        let exclues: HashSet<String> = ["Esker".to_string()].into_iter().collect();
        let (pool, f) = construire_pool(&e, &exclues).unwrap();
        assert_eq!(pool.len(), 1);
        assert_eq!(pool[0].pa, "Cegedim");
        assert_eq!(f.ppf_usable, 2, "les deux franchissent la marche précédente");
        assert_eq!(f.eligibles, 1);
    }

    #[test]
    fn candidat_porte_les_infos_de_tri_et_d_affichage() {
        let mut l = ligne("CF1", "5", "Cegedim");
        l.raison_sociale = "Aubertin Réseaux SAS".into();
        l.in_directory = false;
        l.resolved_at = 42;
        let (pool, _) = construire_pool(&[l], &sans_exclusion()).unwrap();
        assert_eq!(pool[0].raison_sociale, "Aubertin Réseaux SAS");
        assert!(!pool[0].in_directory);
        assert_eq!(pool[0].resolved_at, 42);
        assert_eq!(pool[0].participant, "iso6523-actorid-upis::0225:CF1");
    }

    #[test]
    fn pool_vide() {
        let (pool, f) = construire_pool(&[], &sans_exclusion()).unwrap();
        assert!(pool.is_empty());
        assert_eq!(f, Funnel::default());
    }
}
