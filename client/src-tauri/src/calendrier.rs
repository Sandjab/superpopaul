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

#[cfg(test)]
mod tests {
    use super::*;

    fn d(iso: &str) -> NaiveDate {
        NaiveDate::parse_from_str(iso, "%Y-%m-%d").expect("date de test valide")
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
}
