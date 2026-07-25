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
    Mep { rang: usize },
}

/// Pourquoi un run ne compte pas. Miroir exact des trois filtres de
/// `calendrier::runs_utilisables`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Ecart {
    Exclu,
    HorsFenetre,
    MepNonPassee,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct RunJour {
    pub num: String,
    pub jjs: Vec<u8>,
    pub exclu: bool,
    pub ecart: Option<Ecart>,
    /// Présent si et seulement si `ecart` est `None`.
    pub detail: Option<DetailRun>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct JourTimeline {
    /// ISO, comme le reste de la charge utile envoyée au JS.
    pub date: String,
    /// « lun » … « dim ».
    pub jour_semaine: &'static str,
    pub weekend: bool,
    pub ferie: Option<&'static str>,
    pub jalons: Vec<Jalon>,
    /// Une liste, pas un `Option` : rien n'interdit deux runs à la même date,
    /// et un run perdu en silence est ce que ce lot corrige.
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
    let lo = runs.iter().map(|r| r.date).min().unwrap_or(debut).min(debut);
    let hi = runs.iter().map(|r| r.date).max().unwrap_or(fin).max(fin);

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
            "sans calendrier chargé, l'étendue se réduit à la fenêtre et aucun jour ne porte de run"
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
