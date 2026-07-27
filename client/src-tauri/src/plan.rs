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
    /// Statut CTC complet : `"ready"` | `"later"` | `"expired"` | `""`.
    /// `ctc_ready` en est l'aplatissement et reste consommé par
    /// `construire_pool` et le funnel ; on conserve la chaîne parce que
    /// « prêt plus tard » et « expiré » ne s'arbitrent pas de la même façon.
    pub ctc_status: String,
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
pub(crate) fn parse_jj(brut: &str) -> Option<u8> {
    let jj: u8 = brut.trim().parse().ok()?;
    (1..=31).contains(&jj).then_some(jj)
}

/// Diagnostic de mapping : des comptes lus, mais **aucun** jour de cycle
/// valide.
///
/// Ce n'est pas un problème de données — un fichier réel a toujours quelques
/// jours lisibles — c'est une colonne mal désignée. Le funnel le montre déjà
/// (`jj_valide` à 0), mais en silence : l'écran n'affiche alors que des stocks
/// vides, sans dire pourquoi. Nommer la colonne fautive change un écran muet
/// en diagnostic.
///
/// `None` dès qu'un seul jour est lisible (la colonne est la bonne, le reste
/// relève des données), et `None` sans aucun compte (il n'y a rien à lire :
/// accuser le mapping enverrait corriger ce qui va).
pub fn alerte_colonne_jj(f: &Funnel, colonne: &str) -> Option<String> {
    (f.cf_distincts > 0 && f.jj_valide == 0).then(|| {
        format!(
            "la colonne « {colonne} » ne contient aucun jour de cycle valide (1 à 31) \
             sur {} comptes — colonne mal désignée ?",
            f.cf_distincts
        )
    })
}

// ---------------------------------------------------------------------------
// Répartition, quotas, rampe
// ---------------------------------------------------------------------------

/// Profil des volumes de premières factures par Run de Facturation.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "forme", rename_all = "lowercase")]
pub enum Forme {
    Plate,
    Lineaire,
    Geometrique { raison: f64 },
    /// Volumes saisis run par run : rendus verbatim, la cible est ignorée.
    Manuelle { volumes: BTreeMap<String, usize> },
}

/// Phase pilote : `runs` premiers runs à `cf_par_run` comptes chacun.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Pilote {
    pub runs: usize,
    pub cf_par_run: usize,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Rampe {
    #[serde(flatten)]
    pub forme: Forme,
    #[serde(default)]
    pub pilote: Option<Pilote>,
}

impl Rampe {
    /// Refuse une rampe dont les poids ne seraient pas exploitables sur
    /// `n_runs` runs. **Refus fort, pas correction** : deviner la raison qu'un
    /// utilisateur voulait vaudrait mieux ne rien dire, et un plan faux ne se
    /// voit pas — il ressemble à un plan.
    ///
    /// Seule la forme géométrique est en cause : c'est la seule à porter un
    /// paramètre continu, et `raison.powi(n_runs - 1)` sort de `f64` bien avant
    /// les valeurs qu'un champ de saisie peut recevoir. La borne n'est pas un
    /// nombre choisi mais exactement la précondition de `plus_forts_restes`.
    pub fn valider(&self, n_runs: usize) -> Result<(), String> {
        let Forme::Geometrique { raison } = self.forme else {
            return Ok(());
        };
        if !(raison.is_finite() && raison > 0.0) {
            return Err(format!(
                "rampe géométrique : la raison doit être un nombre strictement positif \
                 (reçu {raison})"
            ));
        }
        let plus_grand = raison.powi(n_runs.saturating_sub(1) as i32);
        if !plus_grand.is_finite() {
            return Err(format!(
                "rampe géométrique : la raison {raison} est trop grande pour {n_runs} \
                 Runs de Facturation — les volumes ne sont plus calculables (réduis la \
                 raison)"
            ));
        }
        Ok(())
    }
}

/// Répartit `total` proportionnellement aux poids, par plus forts restes.
/// La somme rendue est **exactement** `total`. Départage déterministe : reste
/// fractionnaire décroissant, puis clé croissante — sans quoi deux exécutions
/// identiques pourraient produire deux plans différents.
///
/// Le contrat de somme suppose des poids **finis et positifs** : un poids
/// négatif sature les parts entières à zéro, un poids infini les rend toutes
/// `NaN`, et la boucle des restes ne rattrape alors qu'une unité par clé.
/// C'est `Rampe::valider` qui garantit cette précondition en amont.
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

/// Un jour de cycle : sa part du pool, et s'il est couvert par un run retenu.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct StockJJ {
    pub jj: u8,
    pub comptes: usize,
    pub couvert: bool,
}

/// Distribution du pool sur les jours de cycle, et couverture par les runs
/// **retenus**. Toujours 31 entrées : un jour de cycle vide est une
/// information, pas une absence.
pub fn stock_par_jj(pool: &[CfCandidat], retenus: &[RunFacturation]) -> Vec<StockJJ> {
    let mut comptes = [0usize; 32];
    for c in pool {
        if (1..=31).contains(&c.jj) {
            comptes[c.jj as usize] += 1;
        }
    }
    (1..=31u8)
        .map(|jj| StockJJ {
            jj,
            comptes: comptes[jj as usize],
            couvert: retenus.iter().any(|r| r.couvre(jj)),
        })
        .collect()
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

/// D'où vient une ligne du plan. Absorbe l'ancien booléen `coverage_fill` :
/// un remplissage de couverture *est* une origine d'affectation, et en faire
/// un drapeau à part obligerait à un second drapeau dès la troisième origine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Origine {
    /// Allouée par la rampe.
    Auto,
    /// Placée hors quota pour qu'une plateforme soit représentée.
    Couverture,
    /// Ajoutée ou déplacée à la main — épinglée, survit à la régénération.
    Manuel,
}

/// Une ligne du plan. **Auto-porteuse** : elle embarque tout ce qu'il faut
/// pour être relue sans le fichier d'entrée, car le gel doit survivre à un
/// changement de fichier.
#[derive(Debug, Clone, PartialEq)]
pub struct LignePlan {
    pub cf: String,
    pub participant: String,
    pub jj: u8,
    pub raison_sociale: String,
    pub pa: String,
    pub mep_id: usize,
    pub mep_date: chrono::NaiveDate,
    pub run_num: String,
    pub run_date: chrono::NaiveDate,
    pub origine: Origine,
    pub in_directory: bool,
    pub resolved_at: i64,
    /// Horodatage d'affectation (rempli à l'écriture, 0 avant).
    pub planned_at: i64,
    /// Retrait manuel. La ligne n'est jamais supprimée : elle est conservée
    /// avec son motif, exclue des fichiers, des comptages et du re-tirage —
    /// sinon impossible d'expliquer plus tard pourquoi un fichier a changé.
    pub retire: Option<Retrait>,
}

/// Trace d'un retrait manuel. Le motif est **obligatoire** : un retrait sans
/// motif est ingérable six mois plus tard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Retrait {
    pub le: i64,
    pub motif: String,
}

impl LignePlan {
    /// Gelée = rattachée à une MEP déjà passée. **Calculé, jamais stocké** :
    /// une ligne bascule seule le jour venu, comme le statut CTC.
    pub fn gelee(&self, aujourdhui: chrono::NaiveDate) -> bool {
        self.mep_date < aujourdhui
    }

    /// Épinglée = issue d'une retouche manuelle, donc préservée à la
    /// régénération.
    pub fn epinglee(&self) -> bool {
        self.origine == Origine::Manuel
    }

    pub fn retiree(&self) -> bool {
        self.retire.is_some()
    }
}

/// Hash FNV-1a 64 bits d'un compte, salé par le seed. Remplace le
/// `random.shuffle` de peppolstat : un générateur pseudo-aléatoire ne se porte
/// pas d'un langage à l'autre, alors qu'un hash donne la même propriété
/// (départage pseudo-aléatoire, reproductible, réglable) sans dépendance.
/// Volontairement local : le `fnv1a` de `csv_io` sert à une signature
/// PERSISTÉE dont le contrat est de ne jamais changer.
fn hash_seede(seed: u64, cf: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325 ^ seed;
    for &b in cf.as_bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Trie le pool par priorité décroissante : présence à l'annuaire, puis
/// fraîcheur de résolution, puis départage seedé. Une seule clé composite là
/// où peppolstat empile un shuffle et trois tris stables successifs.
pub fn trier_par_priorite(pool: &[CfCandidat], seed: u64) -> Vec<CfCandidat> {
    let mut out = pool.to_vec();
    out.sort_by(|a, b| {
        b.in_directory
            .cmp(&a.in_directory)
            .then_with(|| b.resolved_at.cmp(&a.resolved_at))
            .then_with(|| hash_seede(seed, &a.cf).cmp(&hash_seede(seed, &b.cf)))
    });
    out
}

/// Ce qui s'est passé sur un Run de Facturation. `report_entrant` est une
/// donnée à part entière et non une différence à recalculer : à l'écran, un
/// run qui place plus que son volume de rampe est incompréhensible sans elle.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct DetailRun {
    pub run_num: String,
    pub run_date: String,
    pub jjs: Vec<u8>,
    pub mep_id: usize,
    pub mep_date: String,
    /// Volume issu de la rampe, hors report.
    pub vise: usize,
    pub report_entrant: usize,
    /// Comptes atteignables sur les jours de cycle du run au moment où il joue.
    pub stock: usize,
    pub place: usize,
    /// Ce qui glisse au run suivant.
    pub reliquat: usize,
}

/// Résultat d'une allocation.
#[derive(Debug, Clone, Default)]
pub struct Allocation {
    pub lignes: Vec<LignePlan>,
    pub details: Vec<DetailRun>,
    pub avertissements: Vec<String>,
}

/// Affecte les comptes du pool aux Runs de Facturation, chronologiquement.
///
/// - le volume qu'un run ne peut pas absorber (stock insuffisant sur ses JJ)
///   **glisse** au run suivant ; ce qui reste à la fin est un avertissement,
///   pas une erreur ;
/// - les quotas par plateforme sont des cibles **souples** : si le volume d'un
///   run dépasse les quotas restants des plateformes présentes, le volume
///   prime et le complément est pris toutes plateformes confondues ;
/// - **couverture** : toute plateforme du pool non servie reçoit un compte
///   hors quota sur le premier run couvrant le JJ d'un de ses candidats.
pub fn allouer(
    pool: &[CfCandidat],
    runs: &[RunFacturation],
    meps: &[chrono::NaiveDate],
    seed: u64,
    cible: usize,
    rampe: &Rampe,
    preserves: &Preserves,
) -> Allocation {
    let mut avertissements: Vec<String> = Vec::new();
    let mut details: Vec<DetailRun> = Vec::new();
    let classes = trier_par_priorite(pool, seed);

    // Rang de chaque compte dans l'ordre de priorité : sert à compléter un run
    // toutes plateformes confondues quand les quotas ne suffisent pas.
    let rang: HashMap<&str, usize> = classes
        .iter()
        .enumerate()
        .map(|(i, c)| (c.cf.as_str(), i))
        .collect();
    let mut par_pa: BTreeMap<String, Vec<&CfCandidat>> = BTreeMap::new();
    for c in &classes {
        par_pa.entry(c.pa.clone()).or_default().push(c);
    }
    // Quotas sur le plan COMPLET (préservées incluses), pas seulement sur ce
    // qu'il reste à placer : sinon une plateforme déjà largement servie par le
    // gel recevrait encore une part pleine.
    let mut stock_par_pa: BTreeMap<String, usize> =
        par_pa.iter().map(|(h, v)| (h.clone(), v.len())).collect();
    let mut places_par_pa: HashMap<&str, usize> = HashMap::new();
    for l in preserves.gelees.iter().chain(&preserves.epinglees) {
        *stock_par_pa.entry(l.pa.clone()).or_insert(0) += 1;
    }
    let quotas = quotas_par_pa(cible + preserves.consomme(), &stock_par_pa);
    for l in preserves.gelees.iter().chain(&preserves.epinglees) {
        *places_par_pa.entry(l.pa.as_str()).or_insert(0) += 1;
    }

    let mut affectes: HashSet<&str> = HashSet::new();
    let mut lignes: Vec<LignePlan> = Vec::new();

    let volumes = construire_rampe(cible, runs, rampe);
    if rampe_pilote_infaisable(cible, runs.len(), rampe) {
        let v = rampe.pilote.map(|p| p.cf_par_run).unwrap_or(0);
        avertissements.push(format!(
            "pilote : la cible {cible} est trop basse pour tenir {v} comptes par run sur \
             toute la rampe — creux sous le niveau du pilote (augmenter la cible ou \
             réduire le pilote)"
        ));
    }

    let mut report = 0usize;
    for run in runs {
        let Some((mep_id, mep_date)) = crate::calendrier::mep_de(run.date, meps) else {
            // Un run utilisable a toujours une MEP à sa date ou avant ; si ce
            // n'est pas le cas, le dire plutôt que de produire une ligne
            // bancale.
            avertissements.push(format!(
                "Run de Facturation {} ({}) : aucune MEP à cette date ou avant — run ignoré",
                run.num, run.date
            ));
            continue;
        };
        let vise = volumes.get(&run.num).copied().unwrap_or(0) + report;

        let mut dispo: BTreeMap<&str, Vec<&CfCandidat>> = BTreeMap::new();
        for (pa, cands) in &par_pa {
            let l: Vec<&CfCandidat> = cands
                .iter()
                .filter(|c| run.couvre(c.jj) && !affectes.contains(c.cf.as_str()))
                .copied()
                .collect();
            if !l.is_empty() {
                dispo.insert(pa.as_str(), l);
            }
        }
        let stock: usize = dispo.values().map(Vec::len).sum();
        let pris = vise.min(stock);

        // Quotas restants des plateformes présentes sur ce run.
        let restants: BTreeMap<String, f64> = dispo
            .keys()
            .filter_map(|pa| {
                let q = quotas.get(*pa).copied().unwrap_or(0);
                let deja = places_par_pa.get(*pa).copied().unwrap_or(0);
                (q > deja).then(|| ((*pa).to_string(), (q - deja) as f64))
            })
            .collect();
        let cibles = plus_forts_restes(pris, &restants);

        let mut choisis: Vec<&CfCandidat> = Vec::new();
        for (pa, n) in &cibles {
            if let Some(l) = dispo.get(pa.as_str()) {
                choisis.extend(l.iter().take(*n));
            }
        }
        // Quotas souples : le volume du run prime. Le complément est pris
        // toutes plateformes confondues, dans l'ordre de priorité.
        if choisis.len() < pris {
            let deja: HashSet<&str> = choisis.iter().map(|c| c.cf.as_str()).collect();
            let mut reste: Vec<&CfCandidat> = dispo
                .values()
                .flatten()
                .filter(|c| !deja.contains(c.cf.as_str()))
                .copied()
                .collect();
            reste.sort_by_key(|c| rang[c.cf.as_str()]);
            choisis.extend(reste.into_iter().take(pris - choisis.len()));
        }

        for c in choisis.into_iter().take(pris) {
            affectes.insert(&c.cf);
            *places_par_pa.entry(c.pa.as_str()).or_insert(0) += 1;
            lignes.push(ligne_de(c, run, mep_id, mep_date, Origine::Auto));
        }
        details.push(DetailRun {
            run_num: run.num.clone(),
            run_date: run.date.to_string(),
            jjs: run.jjs.clone(),
            mep_id,
            mep_date: mep_date.to_string(),
            vise: volumes.get(&run.num).copied().unwrap_or(0),
            report_entrant: report,
            stock,
            place: pris,
            reliquat: vise - pris,
        });
        report = vise - pris;
    }

    if runs.is_empty() && cible > 0 {
        avertissements.push(format!(
            "cible non atteinte : {cible} comptes manquants (aucun Run de Facturation utilisable)"
        ));
    } else if report > 0 {
        avertissements.push(format!(
            "cible non atteinte : {report} comptes manquants (stock insuffisant sur les \
             jours de cycle des runs retenus)"
        ));
    }

    // Couverture : chaque plateforme du pool représentée au moins une fois,
    // sur le PREMIER run chronologique couvrant le JJ d'un de ses candidats.
    for (pa, cands) in &par_pa {
        if places_par_pa.get(pa.as_str()).copied().unwrap_or(0) > 0 {
            continue;
        }
        let mut place = false;
        for run in runs {
            let Some((mep_id, mep_date)) = crate::calendrier::mep_de(run.date, meps) else {
                continue;
            };
            if let Some(c) = cands
                .iter()
                .find(|c| run.couvre(c.jj) && !affectes.contains(c.cf.as_str()))
            {
                affectes.insert(&c.cf);
                *places_par_pa.entry(c.pa.as_str()).or_insert(0) += 1;
                lignes.push(ligne_de(c, run, mep_id, mep_date, Origine::Couverture));
                place = true;
                break;
            }
        }
        if !place {
            avertissements.push(format!(
                "plateforme {pa} : aucune couverture possible — les jours de cycle de ses \
                 comptes ne sont couverts par aucun Run de Facturation retenu"
            ));
        }
    }
    Allocation { lignes, details, avertissements }
}

/// Régénère le plan : les lignes préservées sont reprises telles quelles, le
/// reste est ré-alloué. Refuse si une MEP gelée a disparu de la configuration
/// — les fichiers étant cumulatifs, un lot déjà livré changerait en silence.
pub fn regenerer(
    pool: &[CfCandidat],
    runs: &[RunFacturation],
    meps: &[chrono::NaiveDate],
    seed: u64,
    cible: usize,
    rampe: &Rampe,
    preserves: &Preserves,
) -> Result<Allocation, String> {
    // Une MEP gelée absente de la configuration ferait changer un fichier
    // cumulatif déjà transmis, en silence. Refus explicite.
    for g in &preserves.gelees {
        if !meps.contains(&g.mep_date) {
            return Err(format!(
                "la MEP du {} est gelée mais a disparu de la configuration — les MEP \
                 livrées doivent y rester (les fichiers sont cumulatifs)",
                g.mep_date
            ));
        }
    }

    let exclus = preserves.comptes();
    let candidats: Vec<CfCandidat> = pool
        .iter()
        .filter(|c| !exclus.contains(c.cf.as_str()))
        .cloned()
        .collect();

    // Les préservées actives consomment leur part : la rampe ne pourvoit que
    // le complément, mais les quotas raisonnent sur le plan complet.
    let restante = cible.saturating_sub(preserves.consomme());
    let mut a = allouer(&candidats, runs, meps, seed, restante, rampe, preserves);

    let mut plan = preserves.conservees();
    plan.append(&mut a.lignes);
    a.lignes = plan;
    Ok(a)
}

/// Runs pouvant accueillir ce jour de cycle. Le sélecteur de l'IHM ne propose
/// que ceux-là : un compte au JJ 12 ne facturera jamais un run qui traite les
/// JJ 1 et 5, ce n'est pas une préférence mais de l'arithmétique.
pub fn runs_compatibles(jj: u8, runs: &[RunFacturation]) -> Vec<&RunFacturation> {
    runs.iter().filter(|r| r.couvre(jj)).collect()
}

/// Garde commune aux ajouts et déplacements : le run doit couvrir le jour de
/// cycle du compte, et posséder une MEP de rattachement.
fn verifier_placement(
    cf: &str,
    jj: u8,
    run: &RunFacturation,
    meps: &[chrono::NaiveDate],
) -> Result<(usize, chrono::NaiveDate), String> {
    if !run.couvre(jj) {
        return Err(format!(
            "le compte « {cf} » facture au jour {jj}, que le Run de Facturation {} ne \
             traite pas (jours couverts : {})",
            run.num,
            run.jjs.iter().map(u8::to_string).collect::<Vec<_>>().join(", ")
        ));
    }
    crate::calendrier::mep_de(run.date, meps).ok_or_else(|| {
        format!(
            "le Run de Facturation {} ({}) n'a aucune MEP à cette date ou avant",
            run.num, run.date
        )
    })
}

/// Ajoute des comptes au plan, sur un run donné. Les lignes ajoutées sont
/// **épinglées** (`Origine::Manuel`).
///
/// `candidats` est l'ensemble des comptes du FICHIER (pas du pool) : un compte
/// non éligible est ajoutable — cas assumé, on force parfois un compte pilote
/// qu'on sait prêt côté PDP. En revanche un compte absent du fichier est
/// refusé : sans lui, ni jour de cycle ni adressage.
pub fn ajouter(
    plan: &mut Vec<LignePlan>,
    candidats: &[CfCandidat],
    cfs: &[String],
    run: &RunFacturation,
    meps: &[chrono::NaiveDate],
    maintenant: i64,
) -> Result<(), String> {
    // Tout est vérifié avant d'écrire quoi que ce soit : un lot à moitié
    // ajouté serait pire qu'un refus.
    let mut a_ajouter = Vec::new();
    for cf in cfs {
        if plan.iter().any(|l| l.cf == *cf) {
            return Err(format!("le compte « {cf} » est déjà au plan"));
        }
        let c = candidats
            .iter()
            .find(|c| c.cf == *cf)
            .ok_or_else(|| format!("le compte « {cf} » est absent du fichier d'entrée"))?;
        let (mep_id, mep_date) = verifier_placement(cf, c.jj, run, meps)?;
        a_ajouter.push((c, mep_id, mep_date));
    }
    for (c, mep_id, mep_date) in a_ajouter {
        let mut l = ligne_de(c, run, mep_id, mep_date, Origine::Manuel);
        l.planned_at = maintenant;
        plan.push(l);
    }
    Ok(())
}

/// Déplace des comptes vers un autre run. Les lignes déplacées deviennent
/// **épinglées**. Refus si le run ne couvre pas le jour de cycle du compte.
pub fn deplacer(
    plan: &mut [LignePlan],
    cfs: &[String],
    run: &RunFacturation,
    meps: &[chrono::NaiveDate],
) -> Result<(), String> {
    let mut cibles = Vec::new();
    for cf in cfs {
        let i = plan
            .iter()
            .position(|l| l.cf == *cf)
            .ok_or_else(|| format!("le compte « {cf} » n'est pas au plan"))?;
        let (mep_id, mep_date) = verifier_placement(cf, plan[i].jj, run, meps)?;
        cibles.push((i, mep_id, mep_date));
    }
    for (i, mep_id, mep_date) in cibles {
        let l = &mut plan[i];
        l.run_num = run.num.clone();
        l.run_date = run.date;
        l.mep_id = mep_id;
        l.mep_date = mep_date;
        l.origine = Origine::Manuel;
    }
    Ok(())
}

/// Retire des comptes du plan — **sans les supprimer**. La ligne est conservée
/// avec sa date et son motif, exclue des fichiers, des comptages et du
/// re-tirage. Autorisé partout, y compris sur une MEP gelée : c'est un besoin
/// réel (on sait qu'un compte va échouer). L'avertissement sur le changement
/// d'un fichier déjà transmis est l'affaire de l'IHM ; ici, le motif est
/// simplement obligatoire.
pub fn retirer(
    plan: &mut [LignePlan],
    cfs: &[String],
    motif: &str,
    maintenant: i64,
) -> Result<(), String> {
    let motif = motif.trim();
    if motif.is_empty() {
        return Err("un motif est obligatoire pour retirer un compte du plan — sans lui, \
                    le retrait est ingérable plus tard"
            .into());
    }
    let mut cibles = Vec::new();
    for cf in cfs {
        cibles.push(
            plan.iter()
                .position(|l| l.cf == *cf)
                .ok_or_else(|| format!("le compte « {cf} » n'est pas au plan"))?,
        );
    }
    for i in cibles {
        plan[i].retire = Some(Retrait { le: maintenant, motif: motif.to_string() });
    }
    Ok(())
}

/// Réactive des comptes retirés.
pub fn annuler_retrait(plan: &mut [LignePlan], cfs: &[String]) -> Result<(), String> {
    let mut cibles = Vec::new();
    for cf in cfs {
        cibles.push(
            plan.iter()
                .position(|l| l.cf == *cf)
                .ok_or_else(|| format!("le compte « {cf} » n'est pas au plan"))?,
        );
    }
    for i in cibles {
        plan[i].retire = None;
    }
    Ok(())
}

fn ligne_de(
    c: &CfCandidat,
    run: &RunFacturation,
    mep_id: usize,
    mep_date: chrono::NaiveDate,
    origine: Origine,
) -> LignePlan {
    LignePlan {
        cf: c.cf.clone(),
        participant: c.participant.clone(),
        jj: c.jj,
        raison_sociale: c.raison_sociale.clone(),
        pa: c.pa.clone(),
        mep_id,
        mep_date,
        run_num: run.num.clone(),
        run_date: run.date,
        origine,
        in_directory: c.in_directory,
        resolved_at: c.resolved_at,
        planned_at: 0,
        retire: None,
    }
}

/// Paramètres d'un plan, tels qu'ils circulent entre l'IHM et le moteur et
/// tels qu'ils sont persistés (`plan_meta.params_yaml`).
///
/// Les dates sont des chaînes ISO, comme en base : pas de dépendance à une
/// feature `serde` de chrono, et le front manipule déjà des chaînes ISO.
/// Les valeurs illisibles sont refusées à la conversion, jamais devinées.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PlanParams {
    /// Calendrier importé (`runs.csv`), exclusions comprises.
    pub runs: Vec<RunParam>,
    pub debut: String,
    pub fin: String,
    pub meps: Vec<String>,
    #[serde(default)]
    pub mep_count: usize,
    /// `None` = tout le pool éligible atteignable.
    #[serde(default)]
    pub cible: Option<usize>,
    #[serde(default)]
    pub seed: u64,
    #[serde(default)]
    pub pa_exclues: Vec<String>,
    pub rampe: Rampe,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RunParam {
    pub num: String,
    pub date: String,
    pub jjs: Vec<u8>,
    #[serde(default)]
    pub exclu: bool,
}

fn jour_iso(brut: &str, champ: &str) -> Result<chrono::NaiveDate, String> {
    chrono::NaiveDate::parse_from_str(brut.trim(), "%Y-%m-%d")
        .map_err(|_| format!("{champ} : date ISO attendue, reçu « {brut} »"))
}

impl PlanParams {
    /// Reconstruit le calendrier. Refuse une fenêtre inversée : c'est une
    /// saisie fausse, pas un cas limite à absorber.
    pub fn calendrier(
        &self,
    ) -> Result<(Vec<RunFacturation>, chrono::NaiveDate, chrono::NaiveDate, Vec<chrono::NaiveDate>), String>
    {
        let debut = jour_iso(&self.debut, "début de fenêtre")?;
        let fin = jour_iso(&self.fin, "fin de fenêtre")?;
        if fin <= debut {
            return Err(format!("fenêtre FUT : la fin ({fin}) doit suivre le début ({debut})"));
        }
        let mut runs = Vec::with_capacity(self.runs.len());
        for r in &self.runs {
            let mut jjs = r.jjs.clone();
            jjs.sort_unstable();
            jjs.dedup();
            runs.push(RunFacturation {
                num: r.num.clone(),
                date: jour_iso(&r.date, &format!("run {}", r.num))?,
                jjs,
                exclu: r.exclu,
            });
        }
        runs.sort_by(|a, b| a.date.cmp(&b.date).then_with(|| a.num.cmp(&b.num)));
        let mut meps = Vec::with_capacity(self.meps.len());
        for m in &self.meps {
            meps.push(jour_iso(m, "MEP")?);
        }
        meps.sort_unstable();
        meps.dedup();
        Ok((runs, debut, fin, meps))
    }

    pub fn pa_exclues(&self) -> HashSet<String> {
        self.pa_exclues.iter().cloned().collect()
    }

    pub fn vers_yaml(&self) -> Result<String, String> {
        serde_yaml::to_string(self).map_err(|e| format!("sérialisation des paramètres : {e}"))
    }

    pub fn depuis_yaml(yaml: &str) -> Result<Self, String> {
        serde_yaml::from_str(yaml).map_err(|e| format!("paramètres illisibles : {e}"))
    }
}

/// Les trois ensembles de lignes qui échappent au re-tirage. Même mécanique
/// pour les trois : leurs comptes sortent du pool des candidats et consomment
/// leur part de la cible.
#[derive(Debug, Clone, Default)]
pub struct Preserves {
    /// MEP déjà passées : un lot livré ne bouge pas.
    pub gelees: Vec<LignePlan>,
    /// Retouches manuelles : sans ça, une retouche disparaîtrait au premier
    /// changement de rampe.
    pub epinglees: Vec<LignePlan>,
    /// Comptes retirés à la main : sans ça, la rampe les replacerait au
    /// prochain calcul et le retrait ne tiendrait pas.
    pub retirees: Vec<LignePlan>,
}

impl Preserves {
    /// Répartit les lignes d'un plan existant selon leur sort à la
    /// régénération. Un compte retiré l'emporte sur tout le reste : il est
    /// écarté même s'il est gelé ou épinglé.
    pub fn depuis(plan: &[LignePlan], aujourdhui: chrono::NaiveDate) -> Self {
        let mut p = Preserves::default();
        for l in plan {
            // L'ordre des tests compte : un compte retiré est écarté même s'il
            // est gelé ou épinglé — c'est justement ce qu'on ne veut pas livrer.
            if l.retiree() {
                p.retirees.push(l.clone());
            } else if l.gelee(aujourdhui) {
                p.gelees.push(l.clone());
            } else if l.epinglee() {
                p.epinglees.push(l.clone());
            }
        }
        p
    }

    /// Comptes à retirer du pool des candidats.
    pub fn comptes(&self) -> HashSet<&str> {
        self.gelees
            .iter()
            .chain(&self.epinglees)
            .chain(&self.retirees)
            .map(|l| l.cf.as_str())
            .collect()
    }

    /// Lignes conservées telles quelles dans le plan régénéré (les retirées en
    /// font partie : elles restent consultables et annulables).
    pub fn conservees(&self) -> Vec<LignePlan> {
        self.gelees
            .iter()
            .chain(&self.epinglees)
            .chain(&self.retirees)
            .cloned()
            .collect()
    }

    /// Part de cible déjà consommée : gelées et épinglées **actives**. Les
    /// retirées ne comptent pas — c'est justement ce qu'on a décidé de ne pas
    /// livrer.
    pub fn consomme(&self) -> usize {
        self.gelees.len() + self.epinglees.len()
    }
}

/// Cible par défaut, quand l'utilisateur n'en saisit aucune : « tout ce qui est
/// atteignable ».
///
/// Ce n'est PAS `pool.len() + preserves.consomme()` : une ligne préservée dont
/// le compte est encore au pool y occupe déjà une place, et l'additionner
/// réclamerait des comptes qui n'existent pas — « cible non atteinte » sur un
/// plan pourtant complet. Seules les préservées **absentes du pool** (forcées à
/// la main, ou devenues inéligibles) ajoutent une place que le pool ne fournit
/// pas.
pub fn cible_auto(pool: &[CfCandidat], preserves: &Preserves) -> usize {
    let au_pool: HashSet<&str> = pool.iter().map(|c| c.cf.as_str()).collect();
    let hors_pool = preserves
        .gelees
        .iter()
        .chain(&preserves.epinglees)
        .filter(|l| !au_pool.contains(l.cf.as_str()))
        .count();
    pool.len() + hors_pool
}

/// Vrai si un pilote est demandé mais que la cible ne permet pas de tenir son
/// niveau sur tous les runs suivants : le socle est alors impossible et
/// `construire_rampe` bascule sur la forme pure, avec un creux sous V.
pub fn rampe_pilote_infaisable(cible: usize, n_runs: usize, rampe: &Rampe) -> bool {
    // En forme manuelle, `construire_rampe` retourne avant le pilote : aucun
    // socle n'est posé, les volumes saisis font foi. Signaler un pilote
    // infaisable désignerait une cause qui n'agit pas.
    if matches!(rampe.forme, Forme::Manuelle { .. }) {
        return false;
    }
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
            ctc_status: "ready".into(),
            ppf_usable: true,
            in_directory: true,
            resolved_at: 1_700_000_000,
        }
    }

    /// Pose les deux champs CTC ensemble : l'invariant de production est
    /// `ctc_ready == (ctc_status == "ready")`, une fixture ne doit pas
    /// pouvoir le violer.
    fn avec_ctc(mut l: LigneEntree, statut: &str) -> LigneEntree {
        l.ctc_status = statut.into();
        l.ctc_ready = statut == "ready";
        l
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
        e.push(avec_ctc(ligne("CF4", "5", "PA"), "later"));
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
        let l = avec_ctc(ligne("CF1", "5", "PA"), "later");
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
    fn valider_accepte_une_raison_usuelle() {
        assert!(rampe(Forme::Geometrique { raison: 1.55 }).valider(12).is_ok());
        assert!(rampe(Forme::Geometrique { raison: 1.0 }).valider(12).is_ok());
    }

    #[test]
    fn valider_refuse_une_raison_non_positive() {
        // Une raison négative alterne le signe des poids : `plus_forts_restes`
        // sature les parts à zéro et rend un plan VIDE, sans un mot. Le champ de
        // l'écran porte un `min`, mais un attribut HTML n'empêche aucune saisie.
        for mauvaise in [-2.0, 0.0] {
            let e = rampe(Forme::Geometrique { raison: mauvaise })
                .valider(4)
                .expect_err("une raison non positive doit être refusée");
            assert!(e.contains("raison"), "message : {e}");
        }
    }

    #[test]
    fn valider_refuse_une_raison_qui_deborde() {
        // `raison.powi(n-1)` sort de f64 : la somme des poids devient infinie,
        // les parts `NaN`, et la rampe ne place plus qu'un compte par run.
        let e = rampe(Forme::Geometrique { raison: 1e300 })
            .valider(6)
            .expect_err("une raison qui déborde doit être refusée");
        assert!(e.contains("6"), "le message doit nommer le nombre de runs : {e}");
    }

    #[test]
    fn valider_ne_juge_que_la_forme_geometrique() {
        // Les autres formes n'ont aucun paramètre continu : rien à refuser.
        assert!(rampe(Forme::Plate).valider(4).is_ok());
        assert!(rampe(Forme::Lineaire).valider(4).is_ok());
        assert!(rampe(Forme::Manuelle { volumes: BTreeMap::new() }).valider(4).is_ok());
    }

    #[test]
    fn une_rampe_validee_tient_le_contrat_de_somme() {
        // Le POURQUOI de `valider` : ce qu'elle laisse passer doit répartir la
        // cible exactement. Les valeurs refusées plus haut donnaient 0 et 6
        // pour une cible de 100.
        for raison in [1.0, 1.55, 3.0, 50.0] {
            let rs = runs_n(6);
            let r = rampe(Forme::Geometrique { raison });
            r.valider(rs.len()).expect("raison acceptée");
            let somme: usize = construire_rampe(100, &rs, &r).values().sum();
            assert_eq!(somme, 100, "raison {raison}");
        }
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
    fn colonne_jj_sans_aucun_jour_valide_est_signalee() {
        // Vécu en application : la colonne d'adressage désignée comme jour de
        // cycle. Le funnel tombait à zéro dès la deuxième marche, et l'écran
        // n'affichait que des stocks vides — muet sur la cause.
        let f = Funnel { lignes: 125_712, cf_distincts: 40_000, jj_valide: 0, ..Funnel::default() };
        let a = alerte_colonne_jj(&f, "ADRESSAGE_ID").expect("le mapping fautif doit être nommé");
        assert!(a.contains("ADRESSAGE_ID"), "la colonne en cause doit être nommée : {a}");
        assert!(a.contains("40 000") || a.contains("40000"), "l'ampleur doit être dite : {a}");
    }

    #[test]
    fn colonne_jj_partiellement_valide_n_est_pas_signalee() {
        // Un seul jour de cycle lisible suffit à prouver que la colonne est la
        // bonne : le reste est un problème de données, pas de mapping, et le
        // funnel le montre déjà marche par marche.
        let f = Funnel { lignes: 100, cf_distincts: 100, jj_valide: 1, ..Funnel::default() };
        assert_eq!(alerte_colonne_jj(&f, "ACTG_CYCLE_DOM"), None);
    }

    #[test]
    fn fichier_sans_aucun_compte_n_accuse_pas_la_colonne_jj() {
        // `jj_valide` vaut 0 parce qu'il n'y a rien à lire, pas parce que la
        // colonne est fausse. Accuser le mapping enverrait corriger ce qui va.
        let f = Funnel { lignes: 3, cf_distincts: 0, jj_valide: 0, ..Funnel::default() };
        assert_eq!(alerte_colonne_jj(&f, "ACTG_CYCLE_DOM"), None);
    }

    #[test]
    fn pilote_infaisable_jamais_signale_en_forme_manuelle() {
        // Mêmes chiffres que le cas plat ci-dessus — la seule différence est la
        // forme. En manuelle, `construire_rampe` retourne avant le pilote : les
        // volumes saisis font foi, aucun socle n'est posé. Avertir que la cible
        // « est trop basse pour tenir 10 comptes par run » désignerait alors une
        // cause qui n'agit pas, et enverrait l'utilisateur corriger la mauvaise
        // chose. Un YAML persisté peut porter les deux : l'UI qui force
        // `pilote: null` ne suffit pas à fermer ce chemin.
        let r = Rampe {
            forme: Forme::Manuelle { volumes: BTreeMap::new() },
            pilote: Some(Pilote { runs: 2, cf_par_run: 10 }),
        };
        assert!(!rampe_pilote_infaisable(25, 5, &r));
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
    fn le_signalement_du_pilote_suit_exactement_le_socle() {
        // Le seuil est écrit DEUX fois : `construire_rampe` pose le socle quand
        // `budget >= socle`, `rampe_pilote_infaisable` le prédit par
        // `cible - p*v < suite*v`. Deux expressions séparées qu'il faut garder
        // alignées — les cas isolés (25 infaisable, 100 faisable) laissaient la
        // frontière libre, et un `<=` y aurait annoncé un creux pile à la cible
        // qui tient (40 : volumes [10, 10, 10, 10]). L'invariant se teste seul :
        // l'avertissement dit vrai si et seulement si un run de la suite descend
        // sous le niveau du pilote.
        let rs = runs_n(4);
        let r = Rampe {
            forme: Forme::Plate,
            pilote: Some(Pilote { runs: 2, cf_par_run: 10 }),
        };
        // Depuis 1 : à cible nulle il n'y a pas de plan du tout, et signaler un
        // pilote infaisable désignerait une cause qui n'agit pas — `cible == 0`
        // est écarté explicitement par la fonction.
        for cible in 1..=60 {
            let v = construire_rampe(cible, &rs, &r);
            let creux = rs[2..].iter().any(|x| v[&x.num] < 10);
            assert_eq!(
                rampe_pilote_infaisable(cible, rs.len(), &r),
                creux,
                "cible {cible} : volumes {:?}",
                vols(&v, &rs)
            );
        }
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

    // ------------------------------------------------------------------ tri

    fn cand(cf: &str, jj: u8, pa: &str) -> CfCandidat {
        CfCandidat {
            cf: cf.into(),
            participant: format!("0225:{cf}"),
            jj,
            raison_sociale: "ACME".into(),
            pa: pa.into(),
            in_directory: false,
            resolved_at: 0,
        }
    }

    #[test]
    fn tri_annuaire_prime_sur_la_fraicheur() {
        let mut vieux_dans_annuaire = cand("A", 1, "PA");
        vieux_dans_annuaire.in_directory = true;
        vieux_dans_annuaire.resolved_at = 1;
        let mut frais_hors_annuaire = cand("B", 1, "PA");
        frais_hors_annuaire.resolved_at = 999;
        let t = trier_par_priorite(&[frais_hors_annuaire, vieux_dans_annuaire], 42);
        assert_eq!(t[0].cf, "A", "l'annuaire passe avant la fraîcheur");
    }

    #[test]
    fn tri_fraicheur_departage_a_annuaire_egal() {
        let mut vieux = cand("A", 1, "PA");
        vieux.resolved_at = 1;
        let mut frais = cand("B", 1, "PA");
        frais.resolved_at = 999;
        let t = trier_par_priorite(&[vieux, frais], 42);
        assert_eq!(t[0].cf, "B", "le plus frais d'abord");
    }

    #[test]
    fn tri_est_deterministe_a_seed_egal() {
        let pool: Vec<CfCandidat> = (0..20).map(|i| cand(&format!("CF{i:02}"), 1, "PA")).collect();
        let a = trier_par_priorite(&pool, 42);
        let b = trier_par_priorite(&pool, 42);
        assert_eq!(a, b, "même seed, même ordre — c'est la reproductibilité du plan");
    }

    #[test]
    fn tri_change_avec_le_seed() {
        let pool: Vec<CfCandidat> = (0..20).map(|i| cand(&format!("CF{i:02}"), 1, "PA")).collect();
        let a = trier_par_priorite(&pool, 1);
        let b = trier_par_priorite(&pool, 2);
        assert_ne!(a, b, "le seed doit réellement rebattre les cartes");
    }

    // ----------------------------------------------------------- stock/jj

    #[test]
    fn stock_par_jj_rend_les_trente_et_un_jours() {
        let s = stock_par_jj(&[cand("A", 8, "PA1"), cand("A2", 8, "PA1")], &[]);
        assert_eq!(s.len(), 31, "les jours de cycle vides comptent aussi");
        assert_eq!(s[0].jj, 1);
        assert_eq!(s[30].jj, 31);
        assert_eq!(s[7].comptes, 2, "deux comptes sur le même jour de cycle s'additionnent");
    }

    #[test]
    fn stock_par_jj_signale_un_jour_sans_run() {
        // Sans ce signal, les comptes hors d'atteinte restent invisibles :
        // l'écran ne sait dire que « stock insuffisant », jamais où.
        let pool = vec![cand("A", 8, "PA1"), cand("B", 19, "PA1")];
        let retenus = [RunFacturation {
            num: "3320".into(),
            date: d("2026-07-09"),
            jjs: vec![8],
            exclu: false,
        }];
        let s = stock_par_jj(&pool, &retenus);
        assert!(s[7].couvert, "le jour de cycle 8 est couvert par le run");
        assert!(!s[18].couvert, "aucun run ne couvre le jour de cycle 19");
        assert_eq!(s[18].comptes, 1, "et pourtant un compte y est bloqué");
    }

    #[test]
    fn stock_par_jj_sans_run_retenu_ne_couvre_aucun_jour() {
        // Sans run retenu, aucun jour de cycle ne doit ressortir couvert —
        // c'est le cas où `all` (vrai par vacuité sur un itérateur vide)
        // passerait pour `any` et peindrait en vert des jours que rien ne
        // sert. Le filtrage de `exclu` lui-même vit en amont, dans
        // `calendrier::runs_utilisables` : cette fonction ne reçoit que des
        // runs déjà retenus, elle n'a pas à le refaire.
        let s = stock_par_jj(&[cand("A", 9, "PA1")], &[]);
        assert!(s.iter().all(|x| !x.couvert), "aucun jour de cycle n'est couvert");
    }

    #[test]
    fn stock_par_jj_borne_le_jj_sans_paniquer() {
        // `comptes` est un tableau fixe de 32 cases : un `jj` ≥ 32 (le type
        // est un `u8`, donc jusqu'à 255) indexerait hors bornes et ferait
        // paniquer sans ce filtre — ce n'est pas qu'une histoire de comptage
        // silencieux, c'est un accès mémoire à garder valide.
        let s = stock_par_jj(&[cand("A", 31, "PA1")], &[]);
        assert_eq!(s[30].comptes, 1, "jj=31 est une borne haute valide");
        let s1 = stock_par_jj(&[cand("Y", 1, "PA1")], &[]);
        assert_eq!(s1[0].comptes, 1, "jj=1 est une borne basse valide");

        let mut hors_bornes = cand("Z", 1, "PA1");
        hors_bornes.jj = 0;
        let s0 = stock_par_jj(&[hors_bornes.clone()], &[]);
        assert_eq!(s0.iter().map(|x| x.comptes).sum::<usize>(), 0, "jj=0 est ignoré");

        hors_bornes.jj = 32;
        let s32 = stock_par_jj(&[hors_bornes], &[]);
        assert_eq!(
            s32.iter().map(|x| x.comptes).sum::<usize>(),
            0,
            "jj=32 est ignoré, pas de panic hors bornes"
        );
    }

    // ----------------------------------------------------------- allocation

    /// Runs mensuels couvrant tous les mêmes JJ, à partir de février 2026.
    fn runs_jj(n: usize, jjs: &[u8]) -> Vec<RunFacturation> {
        (0..n)
            .map(|i| RunFacturation {
                num: format!("R{}", i + 1),
                date: d(&format!("2026-{:02}-15", i + 2)),
                jjs: jjs.to_vec(),
                exclu: false,
            })
            .collect()
    }

    fn meps1() -> Vec<chrono::NaiveDate> {
        vec![d("2026-01-01")]
    }

    #[test]
    fn allocation_expose_le_detail_chiffre_de_chaque_run() {
        // L'IHM affiche visé · report · stock · placé · reliquat. Le report
        // entrant DOIT être une donnée distincte : sans lui, un run qui place
        // plus que son volume de rampe est incompréhensible à l'écran.
        let mut pool: Vec<CfCandidat> =
            (0..10).map(|i| cand(&format!("A{i}"), 5, "PA")).collect();
        pool.extend((0..10).map(|i| cand(&format!("B{i}"), 20, "PA")));
        let rs = vec![
            RunFacturation { num: "R1".into(), date: d("2026-02-15"), jjs: vec![5], exclu: false },
            RunFacturation { num: "R2".into(), date: d("2026-03-15"), jjs: vec![5, 20], exclu: false },
        ];
        let a = allouer(&pool, &rs, &meps1(), 42, 20, &rampe(Forme::Plate), &Preserves::default());
        assert_eq!(a.details.len(), 2);
        let r1 = &a.details[0];
        assert_eq!(r1.run_num, "R1");
        assert_eq!(r1.vise, 10);
        assert_eq!(r1.report_entrant, 0);
        assert_eq!(r1.stock, 10, "seuls les comptes au JJ 5 sont atteignables");
        assert_eq!(r1.place, 10);
        assert_eq!(r1.reliquat, 0);
        let r2 = &a.details[1];
        assert_eq!(r2.vise, 10);
        assert_eq!(r2.report_entrant, 0);
        assert_eq!(r2.place, 10);
    }

    #[test]
    fn detail_montre_le_report_entrant_du_run_suivant() {
        // R1 ne peut placer que 2 comptes sur 10 visés : 8 glissent sur R2,
        // qui place alors plus que son propre volume.
        let mut pool: Vec<CfCandidat> = (0..2).map(|i| cand(&format!("A{i}"), 5, "PA")).collect();
        pool.extend((0..18).map(|i| cand(&format!("B{i:02}"), 20, "PA")));
        let rs = vec![
            RunFacturation { num: "R1".into(), date: d("2026-02-15"), jjs: vec![5], exclu: false },
            RunFacturation { num: "R2".into(), date: d("2026-03-15"), jjs: vec![5, 20], exclu: false },
        ];
        let a = allouer(&pool, &rs, &meps1(), 42, 20, &rampe(Forme::Plate), &Preserves::default());
        assert_eq!(a.details[0].place, 2);
        assert_eq!(a.details[0].reliquat, 8);
        assert_eq!(a.details[1].report_entrant, 8);
        assert_eq!(a.details[1].place, 18, "10 visés + 8 reportés");
    }

    #[test]
    fn allocation_nominale_respecte_les_volumes_de_rampe() {
        let pool: Vec<CfCandidat> = (0..100).map(|i| cand(&format!("CF{i:03}"), 5, "PA")).collect();
        let rs = runs_jj(4, &[5]);
        let a = allouer(&pool, &rs, &meps1(), 42, 100, &rampe(Forme::Plate), &Preserves::default());
        let (lignes, warns) = (a.lignes, a.avertissements);
        assert_eq!(lignes.len(), 100);
        assert!(warns.is_empty(), "{warns:?}");
        for r in &rs {
            let n = lignes.iter().filter(|l| l.run_num == r.num).count();
            assert_eq!(n, 25, "run {}", r.num);
        }
    }

    #[test]
    fn allocation_rattache_chaque_ligne_a_sa_mep() {
        let pool: Vec<CfCandidat> = (0..4).map(|i| cand(&format!("CF{i}"), 5, "PA")).collect();
        let rs = runs_jj(2, &[5]);
        let meps = vec![d("2026-01-01"), d("2026-03-01")];
        let lignes = allouer(&pool, &rs, &meps, 42, 4, &rampe(Forme::Plate), &Preserves::default()).lignes;
        // R1 = 15/02 → MEP 1 ; R2 = 15/03 → MEP 2.
        let l1 = lignes.iter().find(|l| l.run_num == "R1").unwrap();
        let l2 = lignes.iter().find(|l| l.run_num == "R2").unwrap();
        assert_eq!(l1.mep_id, 1);
        assert_eq!(l2.mep_id, 2);
        assert_eq!(l2.mep_date, d("2026-03-01"));
    }

    #[test]
    fn allocation_fait_glisser_le_reliquat_au_run_suivant() {
        // 10 comptes au JJ 5, 10 au JJ 20. R1 ne facture que le JJ 5 : il ne
        // peut pas absorber son volume, le reste glisse sur R2.
        let mut pool: Vec<CfCandidat> =
            (0..10).map(|i| cand(&format!("A{i}"), 5, "PA")).collect();
        pool.extend((0..10).map(|i| cand(&format!("B{i}"), 20, "PA")));
        let rs = vec![
            RunFacturation { num: "R1".into(), date: d("2026-02-15"), jjs: vec![5], exclu: false },
            RunFacturation { num: "R2".into(), date: d("2026-03-15"), jjs: vec![5, 20], exclu: false },
        ];
        let a = allouer(&pool, &rs, &meps1(), 42, 20, &rampe(Forme::Plate), &Preserves::default());
        let (lignes, warns) = (a.lignes, a.avertissements);
        assert_eq!(lignes.len(), 20, "tout est placé au final : {warns:?}");
        assert_eq!(lignes.iter().filter(|l| l.run_num == "R1").count(), 10);
        assert_eq!(lignes.iter().filter(|l| l.run_num == "R2").count(), 10);
    }

    #[test]
    fn allocation_reliquat_final_avertit_sans_echouer() {
        // Cible 20, mais seulement 5 comptes atteignables.
        let pool: Vec<CfCandidat> = (0..5).map(|i| cand(&format!("CF{i}"), 5, "PA")).collect();
        let rs = runs_jj(2, &[5]);
        let a = allouer(&pool, &rs, &meps1(), 42, 20, &rampe(Forme::Plate), &Preserves::default());
        let (lignes, warns) = (a.lignes, a.avertissements);
        assert_eq!(lignes.len(), 5, "ce qui peut être placé l'est");
        assert!(
            warns.iter().any(|w| w.contains("15")),
            "le manque doit être chiffré : {warns:?}"
        );
    }

    #[test]
    fn allocation_sans_run_utilisable_avertit() {
        let pool = vec![cand("CF1", 5, "PA")];
        let a = allouer(&pool, &[], &meps1(), 42, 10, &rampe(Forme::Plate), &Preserves::default());
        let (lignes, warns) = (a.lignes, a.avertissements);
        assert!(lignes.is_empty());
        assert!(warns.iter().any(|w| w.contains("10")), "{warns:?}");
    }

    #[test]
    fn allocation_n_affecte_jamais_deux_fois_le_meme_compte() {
        let pool: Vec<CfCandidat> = (0..30).map(|i| cand(&format!("CF{i:02}"), 5, "PA")).collect();
        let rs = runs_jj(5, &[5]);
        let lignes = allouer(&pool, &rs, &meps1(), 42, 30, &rampe(Forme::Lineaire), &Preserves::default()).lignes;
        let uniques: HashSet<&str> = lignes.iter().map(|l| l.cf.as_str()).collect();
        assert_eq!(uniques.len(), lignes.len(), "doublon d'affectation");
    }

    #[test]
    fn couverture_place_une_plateforme_non_servie_sur_le_premier_run_possible() {
        // Cible 1 : le plancher de quota ne peut couvrir qu'UNE plateforme,
        // et c'est la mieux dotée qui l'emporte. « Petite » n'est donc servie
        // par aucun quota — c'est là que le filet de couverture doit jouer.
        // (Avec une cible ≥ 2, le plancher la servirait déjà, en origine Auto.)
        let mut pool: Vec<CfCandidat> =
            (0..50).map(|i| cand(&format!("G{i:02}"), 5, "Grosse")).collect();
        pool.push(cand("P1", 20, "Petite"));
        let rs = vec![
            RunFacturation { num: "R1".into(), date: d("2026-02-15"), jjs: vec![5], exclu: false },
            RunFacturation { num: "R2".into(), date: d("2026-03-15"), jjs: vec![5, 20], exclu: false },
            RunFacturation { num: "R3".into(), date: d("2026-04-15"), jjs: vec![20], exclu: false },
        ];
        let lignes = allouer(&pool, &rs, &meps1(), 42, 1, &rampe(Forme::Plate), &Preserves::default()).lignes;
        let p = lignes.iter().find(|l| l.pa == "Petite").expect("plateforme non représentée");
        assert_eq!(p.origine, Origine::Couverture);
        assert_eq!(p.run_num, "R2", "le PREMIER run couvrant le JJ 20");
    }

    #[test]
    fn plancher_de_quota_sert_une_petite_plateforme_sans_recourir_a_la_couverture() {
        // Caractérisation : dès que la cible permet un plancher de 1 par
        // plateforme, la petite est servie par l'allocation NORMALE. La
        // couverture n'est qu'un filet, elle ne doit pas se déclencher ici.
        let mut pool: Vec<CfCandidat> =
            (0..50).map(|i| cand(&format!("G{i:02}"), 5, "Grosse")).collect();
        pool.push(cand("P1", 20, "Petite"));
        let rs = vec![
            RunFacturation { num: "R1".into(), date: d("2026-02-15"), jjs: vec![5], exclu: false },
            RunFacturation { num: "R2".into(), date: d("2026-03-15"), jjs: vec![5, 20], exclu: false },
        ];
        let lignes = allouer(&pool, &rs, &meps1(), 42, 2, &rampe(Forme::Plate), &Preserves::default()).lignes;
        let p = lignes.iter().find(|l| l.pa == "Petite").expect("plateforme non représentée");
        assert_eq!(p.origine, Origine::Auto);
    }

    #[test]
    fn couverture_impossible_avertit_en_nommant_la_plateforme() {
        // Le JJ 20 n'est couvert par aucun run.
        let mut pool: Vec<CfCandidat> =
            (0..5).map(|i| cand(&format!("G{i}"), 5, "Grosse")).collect();
        pool.push(cand("P1", 20, "Orpheline"));
        let rs = runs_jj(2, &[5]);
        let a = allouer(&pool, &rs, &meps1(), 42, 5, &rampe(Forme::Plate), &Preserves::default());
        let (lignes, warns) = (a.lignes, a.avertissements);
        assert!(!lignes.iter().any(|l| l.pa == "Orpheline"));
        assert!(
            warns.iter().any(|w| w.contains("Orpheline")),
            "la plateforme doit être nommée : {warns:?}"
        );
    }

    #[test]
    fn allocation_est_reproductible_a_seed_egal() {
        let pool: Vec<CfCandidat> = (0..40).map(|i| cand(&format!("CF{i:02}"), 5, "PA")).collect();
        let rs = runs_jj(3, &[5]);
        let a = allouer(&pool, &rs, &meps1(), 7, 20, &rampe(Forme::Lineaire), &Preserves::default()).lignes;
        let b = allouer(&pool, &rs, &meps1(), 7, 20, &rampe(Forme::Lineaire), &Preserves::default()).lignes;
        assert_eq!(a, b);
    }

    // ------------------------------------------------------------ params

    fn params_ok() -> PlanParams {
        PlanParams {
            runs: vec![
                RunParam { num: "R2".into(), date: "2026-03-15".into(), jjs: vec![5, 1, 5], exclu: false },
                RunParam { num: "R1".into(), date: "2026-02-15".into(), jjs: vec![5], exclu: true },
            ],
            debut: "2026-01-01".into(),
            fin: "2026-12-31".into(),
            meps: vec!["2026-02-01".into(), "2026-01-15".into(), "2026-02-01".into()],
            mep_count: 2,
            cible: Some(100),
            seed: 42,
            pa_exclues: vec!["Esker".into()],
            rampe: rampe(Forme::Plate),
        }
    }

    #[test]
    fn params_calendrier_trie_dedoublonne_et_conserve_les_exclusions() {
        let (runs, debut, fin, meps) = params_ok().calendrier().unwrap();
        assert_eq!(debut, d("2026-01-01"));
        assert_eq!(fin, d("2026-12-31"));
        assert_eq!(runs[0].num, "R1", "runs triés par date");
        assert!(runs[0].exclu, "l'exclusion est conservée");
        assert_eq!(runs[1].jjs, vec![1, 5], "JJ triés et dédoublonnés");
        assert_eq!(meps, vec![d("2026-01-15"), d("2026-02-01")], "MEP triées, dédoublonnées");
    }

    #[test]
    fn params_refusent_une_fenetre_inversee() {
        let mut p = params_ok();
        p.fin = "2025-01-01".into();
        let err = p.calendrier().unwrap_err();
        assert!(err.contains("fenêtre"), "{err}");
    }

    #[test]
    fn params_refusent_une_date_illisible_en_nommant_le_champ() {
        let mut p = params_ok();
        p.runs[0].date = "15/03/2026".into();
        let err = p.calendrier().unwrap_err();
        assert!(err.contains("R2"), "le run fautif doit être nommé : {err}");
    }

    #[test]
    fn params_aller_retour_yaml() {
        let p = params_ok();
        let back = PlanParams::depuis_yaml(&p.vers_yaml().unwrap()).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn params_yaml_conserve_la_rampe_avec_pilote() {
        let mut p = params_ok();
        p.rampe = Rampe {
            forme: Forme::Geometrique { raison: 1.55 },
            pilote: Some(Pilote { runs: 2, cf_par_run: 10 }),
        };
        let back = PlanParams::depuis_yaml(&p.vers_yaml().unwrap()).unwrap();
        assert_eq!(back.rampe, p.rampe);
    }

    #[test]
    fn params_yaml_conserve_une_rampe_manuelle() {
        let mut p = params_ok();
        p.rampe = rampe(Forme::Manuelle {
            volumes: [("R1".to_string(), 7usize)].into_iter().collect(),
        });
        let back = PlanParams::depuis_yaml(&p.vers_yaml().unwrap()).unwrap();
        assert_eq!(back.rampe, p.rampe);
    }

    // -------------------------------------------------------- régénération

    fn lp(cf: &str, jj: u8, pa: &str, mep: &str, origine: Origine) -> LignePlan {
        LignePlan {
            cf: cf.into(),
            participant: format!("0225:{cf}"),
            jj,
            raison_sociale: "ACME".into(),
            pa: pa.into(),
            mep_id: 1,
            mep_date: d(mep),
            run_num: "R1".into(),
            run_date: d("2026-02-15"),
            origine,
            in_directory: false,
            resolved_at: 0,
            planned_at: 0,
            retire: None,
        }
    }

    #[test]
    fn preserves_repartit_selon_le_sort_a_la_regeneration() {
        let hier = lp("GEL", 5, "PA", "2026-01-01", Origine::Auto);
        let demain = lp("AUTO", 5, "PA", "2026-12-01", Origine::Auto);
        let manuel = lp("MAN", 5, "PA", "2026-12-01", Origine::Manuel);
        let mut retire = lp("RET", 5, "PA", "2026-12-01", Origine::Auto);
        retire.retire = Some(Retrait { le: 1, motif: "m".into() });

        let p = Preserves::depuis(&[hier, demain, manuel, retire], d("2026-06-01"));
        assert_eq!(p.gelees.len(), 1);
        assert_eq!(p.gelees[0].cf, "GEL");
        assert_eq!(p.epinglees.len(), 1);
        assert_eq!(p.epinglees[0].cf, "MAN");
        assert_eq!(p.retirees.len(), 1);
        assert_eq!(p.retirees[0].cf, "RET");
        // « AUTO », future et non retouchée, sera re-tirée : elle n'est nulle part.
        assert!(!p.comptes().contains("AUTO"));
    }

    #[test]
    fn preserves_le_retrait_prime_sur_le_gel() {
        let mut gele_retire = lp("X", 5, "PA", "2026-01-01", Origine::Auto);
        gele_retire.retire = Some(Retrait { le: 1, motif: "m".into() });
        let p = Preserves::depuis(&[gele_retire], d("2026-06-01"));
        assert!(p.gelees.is_empty(), "un compte retiré n'est pas à livrer");
        assert_eq!(p.retirees.len(), 1);
        assert_eq!(p.consomme(), 0, "une ligne retirée ne consomme pas la cible");
    }

    #[test]
    fn cible_auto_sans_preservees_est_le_pool() {
        let pool: Vec<CfCandidat> = (0..5).map(|i| cand(&format!("CF{i}"), 5, "PA")).collect();
        assert_eq!(cible_auto(&pool, &Preserves::default()), 5);
    }

    #[test]
    fn cible_auto_ne_compte_pas_deux_fois_une_preservee_du_pool() {
        // LE test de la correction : une ligne épinglée dont le compte est
        // TOUJOURS au pool occupe une place que le pool fournit déjà. L'ajouter
        // gonflait la cible d'autant, et `regenerer` réclamait ensuite des
        // comptes qui n'existaient pas — « cible non atteinte » sur un plan
        // pourtant complet. Un compteur faux fait mentir tout l'écran.
        let pool: Vec<CfCandidat> = (0..5).map(|i| cand(&format!("CF{i}"), 5, "PA")).collect();
        let p = Preserves {
            epinglees: vec![lp("CF0", 5, "PA", "2026-12-01", Origine::Manuel)],
            ..Preserves::default()
        };
        assert_eq!(cible_auto(&pool, &p), 5, "CF0 est déjà compté par le pool");
    }

    #[test]
    fn cible_auto_ajoute_une_preservee_absente_du_pool() {
        // Cas symétrique : un compte forcé à la main sans être éligible, ou
        // devenu inéligible depuis. Le pool ne le fournit pas, il occupe
        // pourtant une place — sans lui, la cible sous-estimerait le plan.
        let pool: Vec<CfCandidat> = (0..5).map(|i| cand(&format!("CF{i}"), 5, "PA")).collect();
        let p = Preserves {
            epinglees: vec![lp("HORS", 5, "PA", "2026-12-01", Origine::Manuel)],
            ..Preserves::default()
        };
        assert_eq!(cible_auto(&pool, &p), 6);
    }

    #[test]
    fn cible_auto_ignore_les_retirees() {
        // Un retrait est une place qu'on a décidé de ne pas occuper : il ne
        // gonfle pas la cible, exactement comme dans `consomme()`.
        let pool: Vec<CfCandidat> = (0..5).map(|i| cand(&format!("CF{i}"), 5, "PA")).collect();
        let mut retiree = lp("HORS", 5, "PA", "2026-12-01", Origine::Manuel);
        retiree.retire = Some(Retrait { le: 0, motif: "clôturé".into() });
        let p = Preserves { retirees: vec![retiree], ..Preserves::default() };
        assert_eq!(cible_auto(&pool, &p), 5);
    }

    #[test]
    fn cible_auto_laisse_la_regeneration_sans_avertissement() {
        // Le symptôme vécu, de bout en bout : tout le pool est plaçable, une
        // ligne épinglée en fait partie — aucun compte ne manque, donc aucun
        // avertissement ne doit sortir.
        let pool: Vec<CfCandidat> = (0..5).map(|i| cand(&format!("CF{i}"), 5, "PA")).collect();
        let p = Preserves {
            epinglees: vec![lp("CF0", 5, "PA", "2026-12-01", Origine::Manuel)],
            ..Preserves::default()
        };
        let rs = runs_jj(2, &[5]);
        let a = regenerer(&pool, &rs, &meps1(), 42, cible_auto(&pool, &p), &rampe(Forme::Plate), &p)
            .unwrap();
        assert!(a.avertissements.is_empty(), "{:?}", a.avertissements);
        assert_eq!(a.lignes.len(), 5, "les cinq comptes du pool sont au plan");
    }

    #[test]
    fn regeneration_une_ligne_manuelle_survit_a_un_changement_de_rampe() {
        // LE test qui empêche la perte silencieuse : sans épinglage, retoucher
        // le plan puis changer la raison de la rampe effacerait la retouche.
        let pool: Vec<CfCandidat> = (0..10).map(|i| cand(&format!("CF{i}"), 5, "PA")).collect();
        let rs = runs_jj(3, &[5]);
        let manuel = lp("CF7", 5, "PA", "2026-12-01", Origine::Manuel);
        let p = Preserves { epinglees: vec![manuel], ..Preserves::default() };

        let plan = regenerer(&pool, &rs, &meps1(), 42, 10, &rampe(Forme::Lineaire), &p).unwrap().lignes;
        let l = plan.iter().find(|l| l.cf == "CF7").expect("la retouche a disparu");
        assert_eq!(l.origine, Origine::Manuel);
        assert_eq!(l.run_num, "R1", "elle n'a pas été replacée par la rampe");
        assert_eq!(plan.len(), 10, "pas de double affectation");
    }

    #[test]
    fn regeneration_une_ligne_auto_est_bien_retiree() {
        let pool: Vec<CfCandidat> = (0..5).map(|i| cand(&format!("CF{i}"), 5, "PA")).collect();
        let rs = runs_jj(2, &[5]);
        let plan = regenerer(
            &pool, &rs, &meps1(), 42, 5, &rampe(Forme::Plate), &Preserves::default(),
        ).unwrap().lignes;
        assert!(plan.iter().all(|l| l.origine == Origine::Auto));
        assert_eq!(plan.len(), 5);
    }

    #[test]
    fn regeneration_ne_replace_jamais_un_compte_retire() {
        // Sans cet écart du pool, la rampe replacerait le compte au prochain
        // calcul et le retrait ne tiendrait pas.
        let pool: Vec<CfCandidat> = (0..5).map(|i| cand(&format!("CF{i}"), 5, "PA")).collect();
        let rs = runs_jj(2, &[5]);
        let mut retire = lp("CF3", 5, "PA", "2026-12-01", Origine::Auto);
        retire.retire = Some(Retrait { le: 1, motif: "compte clôturé".into() });
        let p = Preserves { retirees: vec![retire], ..Preserves::default() };

        let plan = regenerer(&pool, &rs, &meps1(), 42, 5, &rampe(Forme::Plate), &p).unwrap().lignes;
        let l = plan.iter().find(|l| l.cf == "CF3").expect("la trace doit rester");
        assert!(l.retiree(), "CF3 ne doit pas redevenir actif");
        assert_eq!(plan.iter().filter(|l| l.cf == "CF3").count(), 1);
    }

    #[test]
    fn regeneration_les_preserves_consomment_la_cible_sans_double_compte() {
        let pool: Vec<CfCandidat> = (0..20).map(|i| cand(&format!("CF{i:02}"), 5, "PA")).collect();
        let rs = runs_jj(3, &[5]);
        let p = Preserves {
            gelees: vec![lp("CF00", 5, "PA", "2026-01-01", Origine::Auto)],
            epinglees: vec![lp("CF01", 5, "PA", "2026-12-01", Origine::Manuel)],
            ..Preserves::default()
        };
        let plan = regenerer(&pool, &rs, &meps1(), 42, 10, &rampe(Forme::Plate), &p).unwrap().lignes;
        let actives = plan.iter().filter(|l| !l.retiree()).count();
        assert_eq!(actives, 10, "cible tenue, gelées et épinglées comprises");
        let uniques: HashSet<&str> = plan.iter().map(|l| l.cf.as_str()).collect();
        assert_eq!(uniques.len(), plan.len());
    }

    #[test]
    fn regeneration_refuse_une_mep_gelee_disparue_de_la_configuration() {
        let pool = vec![cand("CF1", 5, "PA")];
        let rs = runs_jj(2, &[5]);
        // La gelée pointe une MEP du 01/03 absente de la liste fournie.
        let p = Preserves {
            gelees: vec![lp("CFG", 5, "PA", "2026-03-01", Origine::Auto)],
            ..Preserves::default()
        };
        let err = regenerer(&pool, &rs, &meps1(), 42, 5, &rampe(Forme::Plate), &p).unwrap_err();
        assert!(err.contains("2026-03-01"), "la MEP doit être nommée : {err}");
    }

    // ------------------------------------------------------------ retouche

    #[test]
    fn runs_compatibles_ne_liste_que_ceux_couvrant_le_jj() {
        let rs = vec![
            RunFacturation { num: "R1".into(), date: d("2026-02-15"), jjs: vec![1, 5], exclu: false },
            RunFacturation { num: "R2".into(), date: d("2026-03-15"), jjs: vec![12], exclu: false },
        ];
        let c = runs_compatibles(12, &rs);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].num, "R2");
        assert!(runs_compatibles(31, &rs).is_empty());
    }

    fn run_r9(jjs: &[u8]) -> RunFacturation {
        RunFacturation { num: "R9".into(), date: d("2026-04-15"), jjs: jjs.to_vec(), exclu: false }
    }

    #[test]
    fn ajout_epingle_la_ligne() {
        let mut plan = vec![];
        let cands = vec![cand("CF1", 5, "PA")];
        ajouter(&mut plan, &cands, &["CF1".into()], &run_r9(&[5]), &meps1(), 123).unwrap();
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].origine, Origine::Manuel);
        assert_eq!(plan[0].run_num, "R9");
        assert_eq!(plan[0].planned_at, 123);
    }

    #[test]
    fn ajout_d_un_compte_non_eligible_est_accepte() {
        // Cas assumé : forcer un compte pilote qu'on sait prêt côté PDP. La
        // liste des candidats est celle du FICHIER, pas du pool.
        let mut plan = vec![];
        let cands = vec![cand("PILOTE", 5, "PA")];
        assert!(ajouter(&mut plan, &cands, &["PILOTE".into()], &run_r9(&[5]), &meps1(), 1).is_ok());
    }

    #[test]
    fn ajout_d_un_compte_absent_du_fichier_est_refuse() {
        let mut plan = vec![];
        let err = ajouter(&mut plan, &[], &["FANTOME".into()], &run_r9(&[5]), &meps1(), 1)
            .unwrap_err();
        assert!(err.contains("FANTOME"), "{err}");
        assert!(plan.is_empty());
    }

    #[test]
    fn ajout_sur_un_run_incompatible_est_refuse() {
        let mut plan = vec![];
        let cands = vec![cand("CF1", 12, "PA")];
        let err = ajouter(&mut plan, &cands, &["CF1".into()], &run_r9(&[5]), &meps1(), 1)
            .unwrap_err();
        assert!(err.contains("12"), "le jour de cycle doit être nommé : {err}");
    }

    #[test]
    fn ajout_d_un_compte_deja_au_plan_est_refuse() {
        let mut plan = vec![lp("CF1", 5, "PA", "2026-01-01", Origine::Auto)];
        let cands = vec![cand("CF1", 5, "PA")];
        let err = ajouter(&mut plan, &cands, &["CF1".into()], &run_r9(&[5]), &meps1(), 1)
            .unwrap_err();
        assert!(err.contains("CF1"), "{err}");
        assert_eq!(plan.len(), 1);
    }

    #[test]
    fn deplacement_epingle_et_change_le_run_et_la_mep() {
        let mut plan = vec![lp("CF1", 5, "PA", "2026-01-01", Origine::Auto)];
        let meps = vec![d("2026-01-01"), d("2026-03-01")];
        deplacer(&mut plan, &["CF1".into()], &run_r9(&[5]), &meps).unwrap();
        assert_eq!(plan[0].origine, Origine::Manuel);
        assert_eq!(plan[0].run_num, "R9");
        assert_eq!(plan[0].mep_id, 2, "R9 (15/04) dépend de la MEP du 01/03");
    }

    #[test]
    fn deplacement_vers_un_run_incompatible_est_refuse() {
        let mut plan = vec![lp("CF1", 12, "PA", "2026-01-01", Origine::Auto)];
        let err = deplacer(&mut plan, &["CF1".into()], &run_r9(&[5]), &meps1()).unwrap_err();
        assert!(err.contains("12"), "{err}");
        assert_eq!(plan[0].run_num, "R1", "rien n'a bougé");
    }

    #[test]
    fn deplacement_d_un_compte_absent_du_plan_est_refuse() {
        let mut plan = vec![];
        let err = deplacer(&mut plan, &["CF1".into()], &run_r9(&[5]), &meps1()).unwrap_err();
        assert!(err.contains("CF1"), "{err}");
    }

    #[test]
    fn retrait_trace_la_date_et_le_motif() {
        let mut plan = vec![lp("CF1", 5, "PA", "2026-12-01", Origine::Auto)];
        retirer(&mut plan, &["CF1".into()], "compte clôturé", 999).unwrap();
        let r = plan[0].retire.as_ref().expect("trace absente");
        assert_eq!(r.le, 999);
        assert_eq!(r.motif, "compte clôturé");
        assert!(plan[0].retiree());
    }

    #[test]
    fn retrait_sans_motif_est_refuse() {
        let mut plan = vec![lp("CF1", 5, "PA", "2026-12-01", Origine::Auto)];
        for motif in ["", "   ", "\t"] {
            assert!(retirer(&mut plan, &["CF1".into()], motif, 1).is_err());
        }
        assert!(!plan[0].retiree(), "aucun retrait n'a eu lieu");
    }

    #[test]
    fn retrait_sur_une_mep_gelee_est_autorise_et_trace() {
        // Décision assumée : le fichier cumulatif déjà transmis changera.
        // L'avertissement est l'affaire de l'IHM ; le moteur, lui, accepte.
        let mut plan = vec![lp("CF1", 5, "PA", "2026-01-01", Origine::Auto)];
        assert!(plan[0].gelee(d("2026-06-01")));
        retirer(&mut plan, &["CF1".into()], "échec connu", 999).unwrap();
        assert!(plan[0].retiree());
    }

    #[test]
    fn annulation_de_retrait_reactive_la_ligne() {
        let mut plan = vec![lp("CF1", 5, "PA", "2026-12-01", Origine::Auto)];
        retirer(&mut plan, &["CF1".into()], "erreur de manip", 1).unwrap();
        annuler_retrait(&mut plan, &["CF1".into()]).unwrap();
        assert!(!plan[0].retiree());
    }

    #[test]
    fn retouche_en_lot_sur_plusieurs_comptes() {
        let mut plan = vec![
            lp("CF1", 5, "PA", "2026-12-01", Origine::Auto),
            lp("CF2", 5, "PA", "2026-12-01", Origine::Auto),
            lp("CF3", 5, "PA", "2026-12-01", Origine::Auto),
        ];
        retirer(&mut plan, &["CF1".into(), "CF3".into()], "lot", 1).unwrap();
        assert!(plan[0].retiree() && plan[2].retiree());
        assert!(!plan[1].retiree());
    }

    #[test]
    fn allocation_signale_un_pilote_infaisable() {
        let pool: Vec<CfCandidat> = (0..30).map(|i| cand(&format!("CF{i:02}"), 5, "PA")).collect();
        let rs = runs_jj(5, &[5]);
        let r = Rampe {
            forme: Forme::Plate,
            pilote: Some(Pilote { runs: 2, cf_par_run: 10 }),
        };
        let warns = allouer(&pool, &rs, &meps1(), 42, 25, &r, &Preserves::default()).avertissements;
        assert!(warns.iter().any(|w| w.contains("pilote")), "{warns:?}");
    }
}
