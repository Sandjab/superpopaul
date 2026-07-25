//! Calendrier de facturation : Runs de Facturation et mises en production.
//!
//! Vocabulaire (à ne jamais confondre) : un **Run de Facturation** est une date
//! à laquelle la facturation tourne pour une liste de jours de cycle (JJ) ; un
//! **Run de Résolution** est un lot d'appels API — le sens historique du mot
//! « run » dans le reste du projet.
//!
//! Module PUR : aucune DB, aucune UI, aucun accès disque (le contenu de
//! `runs.csv` est passé en texte). Testable sans rien monter.

use chrono::NaiveDate;

/// Un Run de Facturation : date d'exécution + jours de cycle qu'il traite.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunFacturation {
    pub num: String,
    pub date: NaiveDate,
    /// Jours de cycle facturés (1..=31), triés et dédoublonnés.
    pub jjs: Vec<u8>,
    pub exclu: bool,
}

impl RunFacturation {
    /// Un CF n'est atteignable par ce run que si son JJ y figure : c'est une
    /// contrainte arithmétique, pas une préférence.
    pub fn couvre(&self, jj: u8) -> bool {
        self.jjs.contains(&jj)
    }
}

/// Parse le `runs.csv` fourni par l'équipe facturation.
///
/// Contrat : en-tête `DATE_RUN;NUM_RUN;JJS`, date en **JJ/MM/AAAA** stricte,
/// JJ séparés par des tirets (`1-5-15`). Le séparateur de colonnes est `;`.
///
/// **Fail-loud sans abandon** : toutes les erreurs sont collectées et rendues
/// ensemble, les lignes valides sont conservées. Corriger un fichier erreur
/// après erreur serait insupportable ; l'appelant affiche la liste telle
/// quelle. Les runs rendus sont triés par date puis par numéro.
pub fn parse_runs_csv(texte: &str) -> (Vec<RunFacturation>, Vec<String>) {
    let mut runs: Vec<RunFacturation> = Vec::new();
    let mut erreurs: Vec<String> = Vec::new();

    // On garde le numéro de ligne PHYSIQUE : filtrer les lignes vides avant de
    // numéroter ferait pointer les messages d'erreur à côté dans le fichier
    // que l'utilisateur a sous les yeux.
    let sans_cr = texte.replace('\r', "");
    let lignes: Vec<(usize, &str)> = sans_cr
        .split('\n')
        .enumerate()
        .map(|(i, l)| (i + 1, l.trim()))
        .filter(|(_, l)| !l.is_empty())
        .collect();

    let Some(&(_, entete)) = lignes.first() else {
        return (runs, vec!["fichier vide".into()]);
    };
    let attendu = ["DATE_RUN", "NUM_RUN", "JJS"];
    let cols: Vec<String> = decouper(entete)
        .into_iter()
        .map(|c| c.to_uppercase())
        .collect();
    if cols.len() < 3 || cols[..3] != attendu {
        return (
            runs,
            vec![format!(
                "en-tête invalide : « {entete} » — attendu « DATE_RUN;NUM_RUN;JJS »"
            )],
        );
    }

    let mut vus_num: Vec<String> = Vec::new();
    let mut vues_date: Vec<NaiveDate> = Vec::new();
    for &(n, ligne) in lignes.iter().skip(1) {
        let champs = decouper(ligne);
        if champs.len() < 3 {
            erreurs.push(format!(
                "ligne {n} : {} colonne(s), 3 attendues",
                champs.len()
            ));
            continue;
        }
        let (brut_date, brut_num, brut_jjs) = (&champs[0], &champs[1], &champs[2]);

        let Some(date) = parse_date_fr(brut_date) else {
            erreurs.push(format!(
                "ligne {n} : date « {brut_date} » invalide (JJ/MM/AAAA attendu)"
            ));
            continue;
        };
        let num = brut_num.trim().to_string();
        if num.is_empty() {
            erreurs.push(format!("ligne {n} : numéro de run vide"));
            continue;
        }
        let Some(jjs) = parse_jjs(brut_jjs) else {
            erreurs.push(format!(
                "ligne {n} : liste de JJ invalide « {brut_jjs} » (entiers 1 à 31 séparés par des tirets)"
            ));
            continue;
        };
        // Doublons : signalés, et la ligne fautive n'est pas retenue — deux
        // runs de même numéro rendraient les volumes de rampe ambigus.
        if vus_num.contains(&num) {
            erreurs.push(format!("ligne {n} : numéro de run « {num} » en double"));
            continue;
        }
        if vues_date.contains(&date) {
            erreurs.push(format!("ligne {n} : deux runs le {brut_date}"));
            continue;
        }
        vus_num.push(num.clone());
        vues_date.push(date);
        runs.push(RunFacturation { num, date, jjs, exclu: false });
    }

    runs.sort_by(|a, b| a.date.cmp(&b.date).then_with(|| a.num.cmp(&b.num)));
    (runs, erreurs)
}

/// Découpe une ligne sur `;` et retire espaces et guillemets encadrants.
fn decouper(ligne: &str) -> Vec<String> {
    ligne
        .split(';')
        .map(|c| c.trim().trim_matches('"').trim().to_string())
        .collect()
}

/// JJ/MM/AAAA STRICT. Le format ISO est refusé : l'intrant réel est en
/// JJ/MM/AAAA, et l'accepter en silence ouvrirait la porte à une inversion
/// jour/mois indétectable. `NaiveDate` rejette seul les dates inexistantes.
fn parse_date_fr(brut: &str) -> Option<NaiveDate> {
    let p: Vec<&str> = brut.split('/').collect();
    if p.len() != 3 || p[0].len() != 2 || p[1].len() != 2 || p[2].len() != 4 {
        return None;
    }
    if !p.iter().all(|s| s.chars().all(|c| c.is_ascii_digit())) {
        return None;
    }
    NaiveDate::from_ymd_opt(p[2].parse().ok()?, p[1].parse().ok()?, p[0].parse().ok()?)
}

/// Entiers 1..=31 séparés par des tirets, triés et dédoublonnés.
/// `None` dès qu'un élément est invalide : une liste de JJ à moitié comprise
/// donnerait un run qui facture autre chose que ce que le fichier annonce.
fn parse_jjs(brut: &str) -> Option<Vec<u8>> {
    let mut out: Vec<u8> = Vec::new();
    let brut = brut.trim();
    if brut.is_empty() {
        return None;
    }
    for part in brut.split('-') {
        let jj: u8 = part.trim().parse().ok()?;
        if !(1..=31).contains(&jj) {
            return None;
        }
        if !out.contains(&jj) {
            out.push(jj);
        }
    }
    out.sort_unstable();
    Some(out)
}

/// Runs éligibles aux premières factures : non exclus, dans `[debut, fin]`, et
/// **strictement** postérieurs à la première MEP. Le « strictement » n'est pas
/// un détail : un run qui tombe le jour même d'une MEP ne peut pas facturer ce
/// qu'elle vient de déclarer.
///
/// Sans aucune MEP, rien n'est utilisable (il n'y a rien à facturer).
pub fn runs_utilisables(
    runs: &[RunFacturation],
    debut: NaiveDate,
    fin: NaiveDate,
    meps: &[NaiveDate],
) -> Vec<RunFacturation> {
    let Some(premiere) = meps.iter().min() else {
        return Vec::new();
    };
    runs.iter()
        .filter(|r| !r.exclu && r.date >= debut && r.date <= fin && r.date > *premiere)
        .cloned()
        .collect()
}

/// Complète les MEP fournies jusqu'à `voulu`, par équirépartition sur
/// `[debut, fin)`. Les dates fournies sont toujours conservées.
///
/// Une MEP **calculée** qui n'aurait aucun run utilisable après elle est
/// ramenée à la veille du dernier run candidat — sinon elle ne sert à rien.
/// Une MEP **fournie** dans ce cas est conservée telle quelle : c'est un choix
/// de l'utilisateur, on l'avertit sans le corriger.
///
/// Renvoie les MEP triées et dédoublonnées, plus les avertissements.
pub fn completer_meps(
    runs: &[RunFacturation],
    debut: NaiveDate,
    fin: NaiveDate,
    fournies: &[NaiveDate],
    voulu: usize,
) -> (Vec<NaiveDate>, Vec<String>) {
    let mut avertissements: Vec<String> = Vec::new();
    let mut donnees: Vec<NaiveDate> = fournies.to_vec();
    donnees.sort_unstable();
    donnees.dedup();
    let vise = voulu.max(donnees.len());

    let candidats: Vec<&RunFacturation> = runs
        .iter()
        .filter(|r| !r.exclu && r.date >= debut && r.date <= fin)
        .collect();
    let dernier_run = candidats.iter().map(|r| r.date).max();

    let mut out = donnees.clone();
    if out.len() < vise && vise > 0 {
        let etendue = (fin - debut).num_days().max(0);
        for i in 0..vise {
            if out.len() >= vise {
                break;
            }
            // Arithmétique entière (arrondi au plus proche) : pas de flottant,
            // donc pas de dépendance au mode d'arrondi de la plateforme.
            let decalage = (i as i64 * etendue + vise as i64 / 2) / vise as i64;
            let mut s = debut + chrono::Duration::days(decalage);
            if let Some(dr) = dernier_run {
                if s >= dr {
                    // Aucune facture possible après cette date : on la ramène
                    // au dernier jour utile.
                    s = dr - chrono::Duration::days(1);
                    if out.contains(&s) || s < debut {
                        continue;
                    }
                }
            }
            if s >= fin {
                continue; // invariant [debut, fin)
            }
            if !out.contains(&s) {
                out.push(s);
            }
        }
    }
    out.sort_unstable();
    out.dedup();

    if out.len() < vise {
        let manque = vise - out.len();
        avertissements.push(format!(
            "{manque} MEP(s) non planifiable(s) — fenêtre trop courte ou dernier \
             Run de Facturation trop proche"
        ));
    }
    // Les MEP FOURNIES sans run après elles sont conservées (choix de
    // l'utilisateur) mais signalées une par une.
    for m in &donnees {
        if dernier_run.is_none_or(|dr| *m >= dr) {
            avertissements.push(format!(
                "MEP {m} : aucun Run de Facturation utilisable strictement après \
                 cette date dans la fenêtre"
            ));
        }
    }
    if candidats.is_empty() {
        avertissements.push("aucun Run de Facturation utilisable dans la fenêtre".into());
    }
    (out, avertissements)
}

/// MEP de rattachement d'un run : la dernière **strictement** antérieure.
/// Renvoie `(numéro 1-basé, date)`, ou `None` si le run précède toute MEP.
/// `meps` doit être trié croissant.
pub fn mep_de(run: NaiveDate, meps: &[NaiveDate]) -> Option<(usize, NaiveDate)> {
    meps.iter()
        .enumerate()
        .rfind(|(_, m)| **m < run)
        .map(|(i, m)| (i + 1, *m))
}

/// Dimanche de Pâques, computus de Meeus (forme grégorienne anonyme).
fn paques(annee: i32) -> NaiveDate {
    let a = annee % 19;
    let b = annee / 100;
    let c = annee % 100;
    let d = b / 4;
    let e = b % 4;
    let f = (b + 8) / 25;
    let g = (b - f + 1) / 3;
    let h = (19 * a + b - d - g + 15) % 30;
    let i = c / 4;
    let k = c % 4;
    let l = (32 + 2 * e + 2 * i - h - k) % 7;
    let m = (a + 11 * h + 22 * l) / 451;
    let n = h + l - 7 * m + 114;
    NaiveDate::from_ymd_opt(annee, (n / 31) as u32, (n % 31 + 1) as u32)
        .expect("le computus ne produit que des dates de mars ou d'avril")
}

fn fixe(annee: i32, mois: u32, jour: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(annee, mois, jour).expect("date fixe valide toute année")
}

/// Les onze jours fériés nationaux français de l'année, triés par date, avec
/// leur nom. Pas de particularisme d'Alsace-Moselle : parité avec peppolstat.
///
/// Purement décoratifs : aucun calcul du plan ne les lit. Ils servent à
/// comprendre un calendrier de runs, pas à le corriger.
pub fn feries(annee: i32) -> Vec<(NaiveDate, &'static str)> {
    let p = paques(annee);
    let mut out = vec![
        (fixe(annee, 1, 1), "Jour de l'an"),
        (fixe(annee, 5, 1), "Fête du Travail"),
        (fixe(annee, 5, 8), "Victoire 1945"),
        (fixe(annee, 7, 14), "Fête nationale"),
        (fixe(annee, 8, 15), "Assomption"),
        (fixe(annee, 11, 1), "Toussaint"),
        (fixe(annee, 11, 11), "Armistice"),
        (fixe(annee, 12, 25), "Noël"),
        (p + chrono::Duration::days(1), "Lundi de Pâques"),
        (p + chrono::Duration::days(39), "Ascension"),
        (p + chrono::Duration::days(50), "Lundi de Pentecôte"),
    ];
    out.sort_by_key(|(d, _)| *d);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(iso: &str) -> NaiveDate {
        NaiveDate::parse_from_str(iso, "%Y-%m-%d").expect("date de test valide")
    }

    /// Runs de facturation de test, aux dates données (num dérivé de l'index).
    fn runs(dates: &[&str]) -> Vec<RunFacturation> {
        dates
            .iter()
            .enumerate()
            .map(|(i, s)| RunFacturation {
                num: format!("R{}", i + 1),
                date: d(s),
                jjs: vec![1],
                exclu: false,
            })
            .collect()
    }

    #[test]
    fn utilisables_ecarte_un_run_exclu() {
        let mut rs = runs(&["2026-08-11", "2026-08-25"]);
        rs[0].exclu = true;
        let out = runs_utilisables(&rs, d("2026-06-01"), d("2026-12-31"), &[d("2026-06-15")]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].num, "R2");
    }

    #[test]
    fn utilisables_bornes_de_fenetre_incluses() {
        let rs = runs(&["2026-05-31", "2026-06-01", "2026-12-31", "2027-01-01"]);
        let out = runs_utilisables(&rs, d("2026-06-01"), d("2026-12-31"), &[d("2026-05-01")]);
        let nums: Vec<&str> = out.iter().map(|r| r.num.as_str()).collect();
        assert_eq!(nums, vec!["R2", "R3"], "les deux bornes sont incluses");
    }

    #[test]
    fn utilisables_ecarte_un_run_le_jour_meme_de_la_premiere_mep() {
        // Le « strictement postérieur » de la spec : un run le jour de la MEP
        // ne peut pas facturer ce qu'elle vient de déclarer.
        let rs = runs(&["2026-06-15", "2026-06-16"]);
        let out = runs_utilisables(&rs, d("2026-06-01"), d("2026-12-31"), &[d("2026-06-15")]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].date, d("2026-06-16"));
    }

    #[test]
    fn utilisables_prend_la_premiere_mep_meme_non_triee() {
        let rs = runs(&["2026-06-20"]);
        // La plus ancienne MEP fait foi, quel que soit l'ordre reçu.
        let out = runs_utilisables(
            &rs,
            d("2026-06-01"),
            d("2026-12-31"),
            &[d("2026-10-01"), d("2026-06-15")],
        );
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn utilisables_vide_sans_mep() {
        let rs = runs(&["2026-08-11"]);
        assert!(runs_utilisables(&rs, d("2026-06-01"), d("2026-12-31"), &[]).is_empty());
    }

    #[test]
    fn meps_fournies_conservees_sans_completion() {
        // voulu <= nombre de dates fournies : rien n'est ajouté.
        let rs = runs(&["2026-08-11"]);
        let (meps, warns) = completer_meps(
            &rs,
            d("2026-06-01"),
            d("2026-12-31"),
            &[d("2026-06-15"), d("2026-07-01")],
            0,
        );
        assert_eq!(meps, vec![d("2026-06-15"), d("2026-07-01")]);
        assert!(warns.is_empty(), "{warns:?}");
    }

    #[test]
    fn meps_equireparties_sur_la_fenetre() {
        // Fenêtre 01/06 → 31/12 (213 jours), 3 MEP, dernier run le 10/11 :
        // décalages 0, 71 et 142 jours.
        let rs = runs(&["2026-11-10"]);
        let (meps, warns) = completer_meps(&rs, d("2026-06-01"), d("2026-12-31"), &[], 3);
        assert_eq!(meps, vec![d("2026-06-01"), d("2026-08-11"), d("2026-10-21")]);
        assert!(warns.is_empty(), "{warns:?}");
    }

    #[test]
    fn meps_calculee_sans_run_apres_est_ramenee_a_la_veille_du_dernier_run() {
        // Fenêtre 01/06 → 30/06, dernier run le 10/06 : les slots 11/06 et
        // 20/06 n'ont aucun run après eux, ils sont ramenés au 09/06 — et le
        // second devient un doublon, donc une MEP manque.
        let rs = runs(&["2026-06-10"]);
        let (meps, warns) = completer_meps(&rs, d("2026-06-01"), d("2026-06-30"), &[], 3);
        assert_eq!(meps, vec![d("2026-06-01"), d("2026-06-09")]);
        assert!(
            warns.iter().any(|w| w.contains("1 MEP")),
            "le manque doit être signalé : {warns:?}"
        );
    }

    #[test]
    fn mep_fournie_sans_run_apres_est_conservee_avec_avertissement() {
        // Choix de l'utilisateur : on avertit, on ne corrige pas.
        let rs = runs(&["2026-06-10"]);
        let (meps, warns) = completer_meps(
            &rs,
            d("2026-06-01"),
            d("2026-12-31"),
            &[d("2026-08-01")],
            0,
        );
        assert_eq!(meps, vec![d("2026-08-01")], "la date fournie est conservée");
        assert!(
            warns.iter().any(|w| w.contains("2026-08-01")),
            "la MEP concernée doit être nommée : {warns:?}"
        );
    }

    #[test]
    fn meps_aucun_run_candidat_averti() {
        let (_meps, warns) = completer_meps(&[], d("2026-06-01"), d("2026-12-31"), &[], 2);
        assert!(
            warns.iter().any(|w| w.contains("aucun Run de Facturation")),
            "{warns:?}"
        );
    }

    #[test]
    fn meps_jamais_a_la_borne_de_fin() {
        // Invariant [debut, fin) : une MEP le dernier jour n'aurait aucun run
        // après elle par construction.
        let rs = runs(&["2026-06-20"]);
        let (meps, _) = completer_meps(&rs, d("2026-06-01"), d("2026-06-30"), &[], 5);
        assert!(meps.iter().all(|m| *m < d("2026-06-30")), "{meps:?}");
    }

    #[test]
    fn meps_triees_et_dedoublonnees() {
        let rs = runs(&["2026-12-01"]);
        let (meps, _) = completer_meps(
            &rs,
            d("2026-06-01"),
            d("2026-12-31"),
            &[d("2026-08-01"), d("2026-06-15"), d("2026-08-01")],
            0,
        );
        assert_eq!(meps, vec![d("2026-06-15"), d("2026-08-01")]);
    }

    #[test]
    fn mep_de_prend_la_derniere_anterieure_pas_la_premiere() {
        let meps = vec![d("2026-06-15"), d("2026-08-01"), d("2026-10-01")];
        assert_eq!(mep_de(d("2026-09-10"), &meps), Some((2, d("2026-08-01"))));
    }

    #[test]
    fn mep_de_est_strictement_anterieure() {
        let meps = vec![d("2026-06-15"), d("2026-08-01")];
        // Run le jour même d'une MEP : rattaché à la précédente.
        assert_eq!(mep_de(d("2026-08-01"), &meps), Some((1, d("2026-06-15"))));
    }

    #[test]
    fn mep_de_none_avant_toute_mep() {
        let meps = vec![d("2026-06-15")];
        assert_eq!(mep_de(d("2026-06-15"), &meps), None);
        assert_eq!(mep_de(d("2026-01-01"), &meps), None);
    }

    #[test]
    fn mep_de_numerote_a_partir_de_un() {
        let meps = vec![d("2026-06-15"), d("2026-08-01")];
        assert_eq!(mep_de(d("2026-07-01"), &meps).map(|(i, _)| i), Some(1));
    }

    #[test]
    fn parse_nominal_trie_les_runs_et_les_jj() {
        let (runs, errs) = parse_runs_csv(
            "DATE_RUN;NUM_RUN;JJS\n\
             25/09/2026;R-0927;5-1\n\
             11/08/2026;R-0826;1-5\n",
        );
        assert!(errs.is_empty(), "erreurs inattendues : {errs:?}");
        assert_eq!(runs.len(), 2);
        // Runs triés par date, pas dans l'ordre du fichier.
        assert_eq!(runs[0].num, "R-0826");
        assert_eq!(runs[0].date, d("2026-08-11"));
        // JJ triés, pas dans l'ordre de la cellule.
        assert_eq!(runs[1].jjs, vec![1, 5]);
        assert!(!runs[0].exclu);
    }

    #[test]
    fn parse_dedoublonne_les_jj() {
        let (runs, errs) = parse_runs_csv("DATE_RUN;NUM_RUN;JJS\n11/08/2026;R1;5-1-5\n");
        assert!(errs.is_empty(), "{errs:?}");
        assert_eq!(runs[0].jjs, vec![1, 5]);
    }

    #[test]
    fn parse_entete_absent_premiere_ligne_donnees() {
        // Sans en-tête, la première ligne de données serait avalée en silence.
        let (runs, errs) = parse_runs_csv("11/08/2026;R1;1-5\n25/08/2026;R2;10\n");
        assert!(runs.is_empty(), "aucun run ne doit être retenu");
        assert_eq!(errs.len(), 1);
        assert!(errs[0].contains("en-tête"), "message: {}", errs[0]);
    }

    #[test]
    fn parse_entete_mal_forme() {
        let (runs, errs) = parse_runs_csv("DATE,NUM,JJS\n11/08/2026;R1;1-5\n");
        assert!(runs.is_empty());
        assert_eq!(errs.len(), 1);
        assert!(errs[0].contains("en-tête"), "message: {}", errs[0]);
    }

    #[test]
    fn parse_fichier_vide() {
        let (runs, errs) = parse_runs_csv("");
        assert!(runs.is_empty());
        assert_eq!(errs.len(), 1);
    }

    #[test]
    fn parse_refuse_une_date_inexistante() {
        let (runs, errs) = parse_runs_csv("DATE_RUN;NUM_RUN;JJS\n31/02/2026;R1;1\n");
        assert!(runs.is_empty());
        assert_eq!(errs.len(), 1);
        assert!(errs[0].contains("31/02/2026"), "message: {}", errs[0]);
        assert!(errs[0].contains("ligne 2"), "la ligne doit être nommée : {}", errs[0]);
    }

    #[test]
    fn parse_refuse_le_format_iso() {
        // L'intrant réel est en JJ/MM/AAAA ; accepter l'ISO en silence
        // ouvrirait la porte à une inversion jour/mois indétectable.
        let (runs, errs) = parse_runs_csv("DATE_RUN;NUM_RUN;JJS\n2026-08-11;R1;1\n");
        assert!(runs.is_empty());
        assert_eq!(errs.len(), 1);
        assert!(errs[0].contains("JJ/MM/AAAA"), "message: {}", errs[0]);
    }

    #[test]
    fn parse_refuse_les_jj_hors_bornes() {
        let (runs, errs) = parse_runs_csv(
            "DATE_RUN;NUM_RUN;JJS\n11/08/2026;R1;0\n12/08/2026;R2;32\n13/08/2026;R3;x\n",
        );
        assert!(runs.is_empty());
        assert_eq!(errs.len(), 3, "les trois lignes doivent être signalées : {errs:?}");
    }

    #[test]
    fn parse_refuse_un_numero_vide() {
        let (runs, errs) = parse_runs_csv("DATE_RUN;NUM_RUN;JJS\n11/08/2026; ;1-5\n");
        assert!(runs.is_empty());
        assert_eq!(errs.len(), 1);
        assert!(errs[0].contains("numéro"), "message: {}", errs[0]);
    }

    #[test]
    fn parse_refuse_un_numero_en_double() {
        let (_runs, errs) = parse_runs_csv(
            "DATE_RUN;NUM_RUN;JJS\n11/08/2026;R1;1\n25/08/2026;R1;10\n",
        );
        assert_eq!(errs.len(), 1);
        assert!(errs[0].contains("R1"), "message: {}", errs[0]);
    }

    #[test]
    fn parse_refuse_deux_runs_le_meme_jour() {
        let (_runs, errs) = parse_runs_csv(
            "DATE_RUN;NUM_RUN;JJS\n11/08/2026;R1;1\n11/08/2026;R2;10\n",
        );
        assert_eq!(errs.len(), 1);
        assert!(errs[0].contains("11/08/2026"), "message: {}", errs[0]);
    }

    #[test]
    fn parse_colonnes_manquantes() {
        let (runs, errs) = parse_runs_csv("DATE_RUN;NUM_RUN;JJS\n11/08/2026;R1\n");
        assert!(runs.is_empty());
        assert_eq!(errs.len(), 1);
        assert!(errs[0].contains("colonne"), "message: {}", errs[0]);
    }

    #[test]
    fn parse_collecte_toutes_les_erreurs_sans_perdre_les_lignes_valides() {
        // Le cœur du contrat fail-loud : on ne s'arrête pas à la première
        // erreur, et une ligne saine au milieu du bruit est conservée.
        let (runs, errs) = parse_runs_csv(
            "DATE_RUN;NUM_RUN;JJS\n\
             31/02/2026;R1;1\n\
             11/08/2026;R2;1-5\n\
             12/08/2026;R3;99\n\
             13/08/2026; ;1\n",
        );
        assert_eq!(runs.len(), 1, "la ligne valide doit survivre : {runs:?}");
        assert_eq!(runs[0].num, "R2");
        assert_eq!(errs.len(), 3, "{errs:?}");
    }

    #[test]
    fn parse_tolere_crlf_guillemets_et_lignes_vides() {
        let (runs, errs) = parse_runs_csv(
            "DATE_RUN;NUM_RUN;JJS\r\n\"11/08/2026\";\"R1\";\"1-5\"\r\n\r\n",
        );
        assert!(errs.is_empty(), "{errs:?}");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].num, "R1");
        assert_eq!(runs[0].jjs, vec![1, 5]);
    }

    #[test]
    fn couvre_repond_sur_le_jj() {
        let (runs, _) = parse_runs_csv("DATE_RUN;NUM_RUN;JJS\n11/08/2026;R1;1-5\n");
        assert!(runs[0].couvre(5));
        assert!(!runs[0].couvre(12));
    }

    #[test]
    fn feries_2026_les_onze_dates() {
        let f = feries(2026);
        let dates: Vec<NaiveDate> = f.iter().map(|(d, _)| *d).collect();
        assert_eq!(dates.len(), 11, "onze fériés nationaux, pas un de plus");
        assert_eq!(
            dates,
            vec![
                d("2026-01-01"),
                d("2026-04-06"), // lundi de Pâques (Pâques le 5 avril)
                d("2026-05-01"),
                d("2026-05-08"),
                d("2026-05-14"), // Ascension
                d("2026-05-25"), // lundi de Pentecôte
                d("2026-07-14"),
                d("2026-08-15"),
                d("2026-11-01"),
                d("2026-11-11"),
                d("2026-12-25"),
            ],
            "les fériés sortent triés par date"
        );
        let noms: Vec<&str> = f.iter().map(|(_, n)| *n).collect();
        assert!(
            noms.contains(&"Ascension") && noms.contains(&"Lundi de Pentecôte"),
            "les trois fériés mobiles sont nommés, pas laissés sous un « férié » générique"
        );
    }

    #[test]
    fn feries_annee_bissextile_propagent_le_decalage() {
        // 2024 est bissextile : si le décalage de février n'était pas propagé,
        // les fériés mobiles glisseraient d'un jour.
        let dates: Vec<NaiveDate> = feries(2024).iter().map(|(d, _)| *d).collect();
        assert!(dates.contains(&d("2024-04-01")), "lundi de Pâques (Pâques le 31 mars)");
        assert!(dates.contains(&d("2024-05-09")), "Ascension");
        assert!(dates.contains(&d("2024-05-20")), "lundi de Pentecôte");
    }
}
