//! Assemblage de la timeline calendaire de l'écran Plan de charge.
//!
//! Module PUR : aucune DB, aucune UI, aucun accès disque. Il ne décide rien —
//! il met bout à bout ce que `calendrier` et `plan` ont déjà établi, pour que
//! l'UI n'ait qu'à rendre des lignes.

use crate::calendrier::RunFacturation;
use crate::plan::DetailRun;
use chrono::{Datelike, NaiveDate, Weekday};
use std::collections::HashMap;

/// Ce qui coupe le calendrier. Plusieurs peuvent tomber le même jour.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "sorte", rename_all = "snake_case")]
pub enum Jalon {
    DebutFenetre,
    FinFenetre,
    /// 1-basé, comme `DetailRun.mep_id`.
    Mep { rang: usize },
}

/// Pourquoi un run ne compte pas. Miroir des trois filtres de
/// `calendrier::runs_utilisables`, qui les enchaîne par `&&` : un run peut
/// en échouer plusieurs. Priorité retenue — `Exclu`, puis `HorsFenetre`,
/// puis `MepNonPassee` : l'exclusion est le seul motif que l'utilisateur
/// pilote, elle doit rester lisible même sur un run par ailleurs écarté.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Ecart {
    Exclu,
    HorsFenetre,
    MepNonPassee,
}

/// Un Run de Facturation tel qu'il s'affiche sur son jour civil.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct RunJour {
    pub num: String,
    pub jjs: Vec<u8>,
    /// Redondant avec `Ecart::Exclu`, qui prime : la case à cocher de l'écran
    /// lit un booléen. Invariant : `exclu` ⟺ `ecart == Some(Ecart::Exclu)`.
    pub exclu: bool,
    pub ecart: Option<Ecart>,
    /// Présent si et seulement si `ecart` est `None`.
    pub detail: Option<DetailRun>,
}

/// Un jour civil de la timeline, avec tout ce qui peut y être accroché.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct JourTimeline {
    /// ISO, comme le reste de la charge utile envoyée au JS.
    pub date: String,
    /// « lun » … « dim ».
    pub jour_semaine: &'static str,
    pub weekend: bool,
    pub ferie: Option<&'static str>,
    pub jalons: Vec<Jalon>,
    /// Une liste, pas un `Option` : `parse_runs_csv` refuse deux runs à la même
    /// date, mais `PlanParams::calendrier` ne le revérifie pas — et un run perdu
    /// en silence est exactement ce que ce lot corrige.
    pub runs: Vec<RunJour>,
}

const JOURS: [&str; 7] = ["lun", "mar", "mer", "jeu", "ven", "sam", "dim"];

pub fn timeline(
    runs: &[RunFacturation],
    debut: NaiveDate,
    fin: NaiveDate,
    _meps: &[NaiveDate],
    _details: &[DetailRun],
) -> Vec<JourTimeline> {
    let lo = runs.iter().map(|r| r.date).fold(debut, NaiveDate::min);
    let hi = runs.iter().map(|r| r.date).fold(fin, NaiveDate::max);

    let mut feries: HashMap<NaiveDate, &'static str> = HashMap::new();
    for annee in lo.year()..=hi.year() {
        feries.extend(crate::calendrier::feries(annee));
    }

    let mut out = Vec::new();
    let mut jour = lo;
    while jour <= hi {
        out.push(JourTimeline {
            date: jour.to_string(),
            jour_semaine: JOURS[jour.weekday().num_days_from_monday() as usize],
            weekend: matches!(jour.weekday(), Weekday::Sat | Weekday::Sun),
            ferie: feries.get(&jour).copied(),
            jalons: Vec::new(),
            runs: Vec::new(),
        });
        jour += chrono::Duration::days(1);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(iso: &str) -> NaiveDate {
        NaiveDate::parse_from_str(iso, "%Y-%m-%d").unwrap()
    }

    fn run(num: &str, date: &str, jjs: &[u8]) -> RunFacturation {
        RunFacturation { num: num.into(), date: d(date), jjs: jjs.to_vec(), exclu: false }
    }

    #[test]
    fn couvre_tous_les_jours_sans_trou() {
        let t = timeline(&[], d("2026-07-01"), d("2026-07-05"), &[], &[]);
        assert_eq!(
            t.iter().map(|j| j.date.as_str()).collect::<Vec<_>>(),
            vec!["2026-07-01", "2026-07-02", "2026-07-03", "2026-07-04", "2026-07-05"],
            "un jour manquant serait un trou dans le calendrier affiché"
        );
        assert!(
            t.iter().all(|j| j.runs.is_empty()),
            "aucun run chargé : aucun jour ne doit porter de run"
        );
    }

    #[test]
    fn etendue_deborde_la_fenetre_pour_montrer_les_runs_hors_fenetre() {
        // Le run du 20 est hors fenêtre. S'il sortait de l'étendue, l'écran ne
        // pourrait pas expliquer pourquoi il ne compte pas — le défaut même
        // que ce lot corrige.
        let t = timeline(
            &[run("3326", "2026-07-20", &[17])],
            d("2026-07-10"),
            d("2026-07-15"),
            &[],
            &[],
        );
        assert_eq!(t.first().unwrap().date, "2026-07-10");
        assert_eq!(t.last().unwrap().date, "2026-07-20");
    }

    #[test]
    fn etendue_ne_se_referme_pas_avant_la_fin_de_fenetre() {
        // Le dernier run tombe avant la fin de fenêtre. Si l'étendue
        // s'arrêtait au dernier run, les jours sans run de fin de fenêtre
        // disparaîtraient de l'écran — l'inverse du trou couvert plus haut,
        // mais tout aussi trompeur.
        let t = timeline(
            &[run("3326", "2026-07-08", &[17])],
            d("2026-07-05"),
            d("2026-07-20"),
            &[],
            &[],
        );
        assert_eq!(t.last().unwrap().date, "2026-07-20");
    }

    #[test]
    fn feries_couverts_a_cheval_sur_deux_annees_civiles() {
        // Une fenêtre FUT à cheval sur le changement d'année est banale ; si
        // la boucle sur les années ne prenait que `lo.year()`, tous les
        // fériés de janvier disparaîtraient sans bruit.
        let t = timeline(&[], d("2026-12-24"), d("2027-01-02"), &[], &[]);
        assert_eq!(
            t.iter().find(|j| j.date == "2026-12-25").unwrap().ferie,
            Some("Noël")
        );
        assert_eq!(
            t.iter().find(|j| j.date == "2027-01-01").unwrap().ferie,
            Some("Jour de l'an")
        );
    }

    #[test]
    fn jours_de_week_end_marques() {
        // 4 et 5 juillet 2026 : samedi et dimanche.
        let t = timeline(&[], d("2026-07-01"), d("2026-07-06"), &[], &[]);
        let we: Vec<&str> =
            t.iter().filter(|j| j.weekend).map(|j| j.date.as_str()).collect();
        assert_eq!(we, vec!["2026-07-04", "2026-07-05"]);
        assert_eq!(t[0].jour_semaine, "mer", "1er juillet 2026 est un mercredi");
    }

    #[test]
    fn feries_portes_par_le_jour() {
        let t = timeline(&[], d("2026-07-13"), d("2026-07-15"), &[], &[]);
        assert_eq!(t[1].ferie, Some("Fête nationale"), "le 14 juillet");
        assert_eq!(t[0].ferie, None);
    }
}
