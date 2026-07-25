//! Plan de charge : pool éligible, quotas, rampe, allocation aux Runs de
//! Facturation. Agrégat PUR (aucune DB, aucune UI) — la jointure vit dans
//! `commands::plan_pool_from_scan`, comme `coverage_from_scan` et
//! `securisation_from_scan`.
//!
//! Éligibilité (critères durs) : statut CTC **prêt** ET **PPF utilisable**
//! (motif actif ET PDP réelle sur la MÊME ligne d'annuaire). `ppf_active`
//! seul ne suffit pas : il laisserait entrer des comptes dont la seule ligne
//! active pointe vers une PDP fictive.

use crate::calendrier::RunFacturation;
use std::collections::BTreeMap;
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

// ---------------------------------------------------------------------------
// Répartition, quotas, rampe
// ---------------------------------------------------------------------------

/// Profil des volumes de premières factures par Run de Facturation.
#[derive(Debug, Clone, PartialEq)]
pub enum Forme {
    Plate,
    Lineaire,
    Geometrique { raison: f64 },
    /// Volumes saisis run par run : rendus verbatim, la cible est ignorée.
    Manuelle { volumes: BTreeMap<String, usize> },
}

/// Phase pilote : `runs` premiers runs à `cf_par_run` comptes chacun.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pilote {
    pub runs: usize,
    pub cf_par_run: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Rampe {
    pub forme: Forme,
    pub pilote: Option<Pilote>,
}

/// Répartit `total` proportionnellement aux poids, par plus forts restes.
/// La somme rendue est **exactement** `total`. Départage déterministe : reste
/// fractionnaire décroissant, puis clé croissante — sans quoi deux exécutions
/// identiques pourraient produire deux plans différents.
pub fn plus_forts_restes(total: usize, poids: &BTreeMap<String, f64>) -> BTreeMap<String, usize> {
    let somme: f64 = poids.values().sum();
    let mut out: BTreeMap<String, usize> = poids.keys().map(|k| (k.clone(), 0)).collect();
    if somme <= 0.0 || total == 0 {
        return out;
    }
    let exact: BTreeMap<&str, f64> = poids
        .iter()
        .map(|(k, w)| (k.as_str(), total as f64 * w / somme))
        .collect();
    for (k, v) in &mut out {
        *v = exact[k.as_str()].floor() as usize;
    }
    let mut reste = total.saturating_sub(out.values().sum::<usize>());
    let mut cles: Vec<&String> = poids.keys().collect();
    cles.sort_by(|a, b| {
        let fa = exact[a.as_str()].fract();
        let fb = exact[b.as_str()].fract();
        fb.partial_cmp(&fa)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.cmp(b))
    });
    for k in cles {
        if reste == 0 {
            break;
        }
        *out.get_mut(k).expect("clé issue de poids") += 1;
        reste -= 1;
    }
    out
}

/// Quotas cibles par plateforme : proportionnels au pool, **plancher 1** (toute
/// plateforme ayant au moins un compte éligible doit être représentée),
/// **plafond au stock**, l'excédent étant redistribué.
///
/// Ce sont des cibles **souples** : à l'allocation, le volume d'un run prime
/// sur les quotas restants des plateformes présentes.
pub fn quotas_par_pa(cible: usize, pool_par_pa: &BTreeMap<String, usize>) -> BTreeMap<String, usize> {
    let stock: BTreeMap<String, usize> = pool_par_pa
        .iter()
        .filter(|(_, n)| **n > 0)
        .map(|(h, n)| (h.clone(), *n))
        .collect();
    if stock.is_empty() {
        return BTreeMap::new();
    }
    let mut quotas: BTreeMap<String, usize> = stock.keys().map(|h| (h.clone(), 0)).collect();

    // Plancher 1, aux mieux dotées d'abord quand la cible ne suffit pas à
    // couvrir toutes les plateformes.
    let mut par_taille: Vec<&String> = stock.keys().collect();
    par_taille.sort_by(|a, b| stock[*b].cmp(&stock[*a]).then_with(|| a.cmp(b)));
    for h in par_taille.into_iter().take(stock.len().min(cible)) {
        *quotas.get_mut(h).expect("clé issue de stock") = 1;
    }

    let restant = cible.saturating_sub(quotas.values().sum::<usize>());
    if restant > 0 {
        let poids: BTreeMap<String, f64> =
            stock.iter().map(|(h, n)| (h.clone(), *n as f64)).collect();
        for (h, n) in plus_forts_restes(restant, &poids) {
            *quotas.get_mut(&h).expect("clé issue de stock") += n;
        }
    }

    // Plafond au stock, l'excédent repart vers celles qui ont de la place.
    loop {
        let excedent: usize = stock
            .iter()
            .map(|(h, n)| quotas[h].saturating_sub(*n))
            .sum();
        if excedent == 0 {
            break;
        }
        for (h, n) in &stock {
            let q = quotas.get_mut(h).expect("clé issue de stock");
            *q = (*q).min(*n);
        }
        let place: BTreeMap<String, f64> = stock
            .iter()
            .filter(|(h, n)| **n > quotas[*h])
            .map(|(h, n)| (h.clone(), (n - quotas[h]) as f64))
            .collect();
        if place.is_empty() {
            break;
        }
        for (h, n) in plus_forts_restes(excedent, &place) {
            *quotas.get_mut(&h).expect("clé issue de stock") += n;
        }
    }
    quotas
}

/// Volumes de premières factures par run.
///
/// Le point subtil est le **socle du pilote** : quand un pilote est actif
/// (P premiers runs à V comptes), chaque run suivant démarre à V et la forme
/// ne répartit que le surplus — la rampe prolonge le pilote sans jamais
/// redescendre sous son niveau. Si la cible ne suffit pas à tenir V partout,
/// le socle est abandonné au profit de la forme pure (creux sous V), cas que
/// `rampe_pilote_infaisable` signale.
///
/// Somme **exactement** égale à la cible dès que `runs` est non vide et
/// `cible > 0` — sauf en forme manuelle, où les volumes saisis font foi.
pub fn construire_rampe(
    cible: usize,
    runs: &[RunFacturation],
    rampe: &Rampe,
) -> BTreeMap<String, usize> {
    let vide = || -> BTreeMap<String, usize> {
        runs.iter().map(|r| (r.num.clone(), 0)).collect()
    };
    if let Forme::Manuelle { volumes } = &rampe.forme {
        return runs
            .iter()
            .map(|r| (r.num.clone(), volumes.get(&r.num).copied().unwrap_or(0)))
            .collect();
    }
    if runs.is_empty() || cible == 0 {
        return vide();
    }

    let (v, p) = niveau_pilote(rampe, runs.len());
    let mut volumes: BTreeMap<String, usize> = BTreeMap::new();
    let mut budget = cible;
    for r in &runs[..p] {
        let pris = v.min(budget);
        volumes.insert(r.num.clone(), pris);
        budget -= pris;
    }

    let suite = &runs[p..];
    if !suite.is_empty() && budget > 0 {
        let poids: BTreeMap<String, f64> = suite
            .iter()
            .enumerate()
            .map(|(i, r)| {
                let w = match &rampe.forme {
                    Forme::Plate => 1.0,
                    Forme::Lineaire => (i + 1) as f64,
                    Forme::Geometrique { raison } => raison.powi(i as i32),
                    Forme::Manuelle { .. } => unreachable!("traité plus haut"),
                };
                (r.num.clone(), w)
            })
            .collect();
        let socle = suite.len() * v;
        if budget >= socle {
            // Le niveau du pilote devient un plancher : la forme ne répartit
            // que ce qui dépasse.
            let surplus = plus_forts_restes(budget - socle, &poids);
            for r in suite {
                volumes.insert(r.num.clone(), v + surplus[&r.num]);
            }
        } else {
            // Cible trop basse pour tenir le socle : forme pure, creux assumé
            // (signalé par rampe_pilote_infaisable).
            volumes.extend(plus_forts_restes(budget, &poids));
        }
    } else if budget > 0 {
        // Pilote couvrant tous les runs : le reliquat va sur le dernier, pour
        // que la somme reste exactement égale à la cible.
        let dernier = &runs[runs.len() - 1].num;
        *volumes.entry(dernier.clone()).or_insert(0) += budget;
    }

    runs.iter()
        .map(|r| (r.num.clone(), volumes.get(&r.num).copied().unwrap_or(0)))
        .collect()
}

/// Niveau et durée effectifs du pilote : `(v, p)`. Un pilote à volume nul ou
/// à durée nulle est inerte — la forme principale s'étale alors sur tous les
/// runs.
fn niveau_pilote(rampe: &Rampe, n_runs: usize) -> (usize, usize) {
    match rampe.pilote {
        Some(p) if p.cf_par_run > 0 && p.runs > 0 => (p.cf_par_run, p.runs.min(n_runs)),
        _ => (0, 0),
    }
}

/// Vrai si un pilote est demandé mais que la cible ne permet pas de tenir son
/// niveau sur tous les runs suivants : le socle est alors impossible et
/// `construire_rampe` bascule sur la forme pure, avec un creux sous V.
pub fn rampe_pilote_infaisable(cible: usize, n_runs: usize, rampe: &Rampe) -> bool {
    let (v, p) = niveau_pilote(rampe, n_runs);
    let suite = n_runs.saturating_sub(p);
    if v == 0 || p == 0 || suite == 0 || cible == 0 {
        return false;
    }
    cible.saturating_sub(p * v) < suite * v
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn d(iso: &str) -> NaiveDate {
        NaiveDate::parse_from_str(iso, "%Y-%m-%d").expect("date de test valide")
    }

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

    // ---------------------------------------------------------------- restes

    fn poids(v: &[(&str, f64)]) -> BTreeMap<String, f64> {
        v.iter().map(|(k, w)| (k.to_string(), *w)).collect()
    }

    #[test]
    fn restes_somme_exactement_le_total() {
        // 100 sur trois poids égaux : 34/33/33, jamais 33/33/33.
        let out = plus_forts_restes(100, &poids(&[("a", 1.0), ("b", 1.0), ("c", 1.0)]));
        assert_eq!(out.values().sum::<usize>(), 100);
    }

    #[test]
    fn restes_departage_deterministe_par_cle() {
        // À reste fractionnaire égal, la clé croissante tranche : deux
        // exécutions identiques doivent donner le même plan.
        let p = poids(&[("b", 1.0), ("a", 1.0), ("c", 1.0)]);
        let out = plus_forts_restes(100, &p);
        assert_eq!(out["a"], 34, "la clé la plus petite reçoit l'unité en trop");
        assert_eq!(out["b"], 33);
        assert_eq!(out["c"], 33);
        assert_eq!(plus_forts_restes(100, &p), out, "stable d'un appel à l'autre");
    }

    #[test]
    fn restes_proportionnels() {
        let out = plus_forts_restes(100, &poids(&[("a", 1.0), ("b", 4.0)]));
        assert_eq!(out["a"], 20);
        assert_eq!(out["b"], 80);
    }

    #[test]
    fn restes_total_nul_ou_poids_nuls() {
        assert_eq!(plus_forts_restes(0, &poids(&[("a", 1.0)]))["a"], 0);
        assert_eq!(plus_forts_restes(10, &poids(&[("a", 0.0), ("b", 0.0)]))["a"], 0);
        assert!(plus_forts_restes(10, &poids(&[])).is_empty());
    }

    // ---------------------------------------------------------------- quotas

    fn stock(v: &[(&str, usize)]) -> BTreeMap<String, usize> {
        v.iter().map(|(k, n)| (k.to_string(), *n)).collect()
    }

    #[test]
    fn quotas_plancher_un_pour_toute_plateforme_dotee() {
        // Une plateforme minuscule doit être représentée : c'est la garantie
        // « chaque plateforme au moins une fois ».
        let q = quotas_par_pa(100, &stock(&[("grosse", 1000), ("minuscule", 1)]));
        assert_eq!(q["minuscule"], 1);
        assert_eq!(q.values().sum::<usize>(), 100);
    }

    #[test]
    fn quotas_cible_inferieure_au_nombre_de_plateformes() {
        // Pas assez de place pour toutes : les mieux dotées d'abord.
        let q = quotas_par_pa(2, &stock(&[("a", 10), ("b", 5), ("c", 1)]));
        assert_eq!(q.values().sum::<usize>(), 2);
        assert_eq!(q["a"], 1);
        assert_eq!(q["b"], 1);
        assert_eq!(q["c"], 0);
    }

    #[test]
    fn quotas_plafonnes_au_stock_avec_redistribution() {
        // « petite » ne peut pas absorber sa part proportionnelle : le surplus
        // repart vers celles qui ont de la place.
        let q = quotas_par_pa(100, &stock(&[("grande", 900), ("petite", 3)]));
        assert!(q["petite"] <= 3, "jamais plus que le stock : {q:?}");
        assert_eq!(q.values().sum::<usize>(), 100);
    }

    #[test]
    fn quotas_cible_superieure_au_stock_total() {
        // On ne peut pas distribuer plus que ce qui existe.
        let q = quotas_par_pa(1000, &stock(&[("a", 10), ("b", 5)]));
        assert_eq!(q["a"], 10);
        assert_eq!(q["b"], 5);
    }

    #[test]
    fn quotas_ignore_les_plateformes_sans_stock() {
        let q = quotas_par_pa(10, &stock(&[("a", 10), ("vide", 0)]));
        assert_eq!(q.get("vide").copied().unwrap_or(0), 0);
    }

    #[test]
    fn quotas_pool_vide() {
        assert!(quotas_par_pa(10, &stock(&[])).is_empty());
    }

    // ----------------------------------------------------------------- rampe

    fn runs_n(n: usize) -> Vec<RunFacturation> {
        (0..n)
            .map(|i| RunFacturation {
                num: format!("R{}", i + 1),
                date: d(&format!("2026-0{}-01", i + 1)),
                jjs: vec![1],
                exclu: false,
            })
            .collect()
    }

    fn vols(r: &BTreeMap<String, usize>, runs: &[RunFacturation]) -> Vec<usize> {
        runs.iter().map(|x| r[&x.num]).collect()
    }

    fn rampe(forme: Forme) -> Rampe {
        Rampe { forme, pilote: None }
    }

    #[test]
    fn rampe_plate_equirepartit() {
        let rs = runs_n(4);
        let v = construire_rampe(100, &rs, &rampe(Forme::Plate));
        assert_eq!(vols(&v, &rs), vec![25, 25, 25, 25]);
    }

    #[test]
    fn rampe_lineaire_croit_doucement() {
        let rs = runs_n(4);
        let v = construire_rampe(100, &rs, &rampe(Forme::Lineaire));
        assert_eq!(vols(&v, &rs), vec![10, 20, 30, 40]);
    }

    #[test]
    fn rampe_geometrique_double_a_chaque_run() {
        let rs = runs_n(4);
        let v = construire_rampe(150, &rs, &rampe(Forme::Geometrique { raison: 2.0 }));
        assert_eq!(vols(&v, &rs), vec![10, 20, 40, 80]);
    }

    #[test]
    fn rampe_somme_toujours_egale_a_la_cible() {
        let rs = runs_n(7);
        for f in [
            Forme::Plate,
            Forme::Lineaire,
            Forme::Geometrique { raison: 1.55 },
        ] {
            for cible in [1, 13, 100, 4000] {
                let v = construire_rampe(cible, &rs, &rampe(f.clone()));
                assert_eq!(
                    v.values().sum::<usize>(),
                    cible,
                    "forme {f:?}, cible {cible}"
                );
            }
        }
    }

    #[test]
    fn rampe_manuelle_rend_les_volumes_verbatim() {
        let rs = runs_n(3);
        let volumes: BTreeMap<String, usize> =
            [("R1".to_string(), 7), ("R3".to_string(), 9)].into_iter().collect();
        // La cible est ignorée, et un run absent vaut 0.
        let v = construire_rampe(9999, &rs, &rampe(Forme::Manuelle { volumes }));
        assert_eq!(vols(&v, &rs), vec![7, 0, 9]);
    }

    #[test]
    fn rampe_pilote_pose_un_socle_jamais_franchi_vers_le_bas() {
        // 5 runs, pilote 2×10, cible 100 : les 2 premiers à 10, et AUCUN run
        // suivant sous 10 — la rampe prolonge le pilote.
        let rs = runs_n(5);
        let r = Rampe {
            forme: Forme::Plate,
            pilote: Some(Pilote { runs: 2, cf_par_run: 10 }),
        };
        let v = construire_rampe(100, &rs, &r);
        let got = vols(&v, &rs);
        assert_eq!(got[0], 10);
        assert_eq!(got[1], 10);
        assert!(got[2..].iter().all(|&x| x >= 10), "socle percé : {got:?}");
        assert_eq!(got.iter().sum::<usize>(), 100);
    }

    #[test]
    fn rampe_pilote_infaisable_bascule_sur_la_forme_pure() {
        // Cible trop basse pour tenir 10 par run : le socle est abandonné.
        let rs = runs_n(5);
        let r = Rampe {
            forme: Forme::Plate,
            pilote: Some(Pilote { runs: 2, cf_par_run: 10 }),
        };
        assert!(rampe_pilote_infaisable(25, 5, &r));
        let v = construire_rampe(25, &rs, &r);
        assert_eq!(v.values().sum::<usize>(), 25);
    }

    #[test]
    fn rampe_pilote_faisable_n_est_pas_signale() {
        let r = Rampe {
            forme: Forme::Plate,
            pilote: Some(Pilote { runs: 2, cf_par_run: 10 }),
        };
        assert!(!rampe_pilote_infaisable(100, 5, &r));
    }

    #[test]
    fn rampe_pilote_inerte_si_volume_ou_duree_nuls() {
        let rs = runs_n(4);
        for p in [
            Pilote { runs: 0, cf_par_run: 10 },
            Pilote { runs: 3, cf_par_run: 0 },
        ] {
            let r = Rampe { forme: Forme::Plate, pilote: Some(p) };
            assert_eq!(
                vols(&construire_rampe(100, &rs, &r), &rs),
                vec![25, 25, 25, 25],
                "pilote {p:?} doit être inerte"
            );
            assert!(!rampe_pilote_infaisable(100, 4, &r));
        }
    }

    #[test]
    fn rampe_pilote_couvrant_tous_les_runs_verse_le_reliquat_sur_le_dernier() {
        let rs = runs_n(3);
        let r = Rampe {
            forme: Forme::Plate,
            pilote: Some(Pilote { runs: 3, cf_par_run: 10 }),
        };
        let v = construire_rampe(100, &rs, &r);
        assert_eq!(vols(&v, &rs), vec![10, 10, 80]);
        assert_eq!(v.values().sum::<usize>(), 100);
    }

    #[test]
    fn rampe_sans_run_ou_sans_cible() {
        let rs = runs_n(3);
        assert_eq!(vols(&construire_rampe(0, &rs, &rampe(Forme::Plate)), &rs), vec![0, 0, 0]);
        assert!(construire_rampe(100, &[], &rampe(Forme::Plate)).is_empty());
    }
}
