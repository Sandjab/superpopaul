//! Assemblage de la timeline calendaire de l'écran Plan de charge.
//!
//! Module PUR : aucune DB, aucune UI, aucun accès disque. Il ne décide rien —
//! il met bout à bout ce que `calendrier` et `plan` ont déjà établi, pour que
//! l'UI n'ait qu'à rendre des lignes. Une seule exception : le motif d'écart
//! (`ecart_de`), qui rejoue le filtre de `calendrier::runs_utilisables` faute
//! de pouvoir l'appeler — celui-ci ne rend pas de motif.

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

/// Pourquoi un run ne compte pas. Quatre motifs pour quatre situations,
/// vécues différemment par l'utilisateur bien qu'`AucuneMep` et
/// `MepNonPassee` viennent tous deux du filtre MEP de
/// `calendrier::runs_utilisables` — l'un de son retour anticipé quand `meps`
/// est vide, l'autre de la comparaison de dates. Les quatre motifs ne
/// s'excluent pas mutuellement au niveau des filtres eux-mêmes (enchaînés
/// par `&&`, un run peut en échouer plusieurs), d'où la priorité retenue —
/// `Exclu`, puis `HorsFenetre`, puis les deux motifs de MEP : l'exclusion
/// est le seul motif que l'utilisateur pilote, elle doit rester lisible
/// même sur un run par ailleurs écarté.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Ecart {
    Exclu,
    HorsFenetre,
    /// Des MEP existent, mais aucune ne précède ce run : l'action utile est
    /// de décaler la MEP ou le run.
    MepNonPassee,
    /// Aucune MEP n'est définie — l'état initial normal de l'écran, avant
    /// toute saisie. L'action utile est d'en créer une, pas de décaler une
    /// date qui n'existe pas : c'est pourquoi ce motif est distinct de
    /// `MepNonPassee` plutôt que d'y être replié.
    AucuneMep,
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
    /// Présent si et seulement si `ecart` est `None`. Redouble `run_num`,
    /// `run_date` et `jjs` du `RunJour` porteur : le rendu lit toujours ceux
    /// du `RunJour`, jamais leurs équivalents dans `DetailRun`.
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

const JOURS_SEMAINE: [&str; 7] = ["lun", "mar", "mer", "jeu", "ven", "sam", "dim"];

/// Pourquoi `r` ne compte pas, s'il ne compte pas. C'est l'unique endroit du
/// module qui décide plutôt que d'assembler — nommé pour que l'exception à
/// l'en-tête du module reste visible, et pour miroiter à l'œil nu la
/// signature de `calendrier::runs_utilisables(runs, debut, fin, meps)`.
fn ecart_de(
    r: &RunFacturation,
    debut: NaiveDate,
    fin: NaiveDate,
    premiere_mep: Option<NaiveDate>,
) -> Option<Ecart> {
    // Ordre délibéré : l'exclusion prime, c'est le seul motif que
    // l'utilisateur pilote depuis l'écran.
    if r.exclu {
        Some(Ecart::Exclu)
    } else if r.date < debut || r.date > fin {
        Some(Ecart::HorsFenetre)
    } else {
        match premiere_mep {
            None => Some(Ecart::AucuneMep),
            Some(p) if r.date <= p => Some(Ecart::MepNonPassee),
            Some(_) => None,
        }
    }
}

pub fn timeline(
    runs: &[RunFacturation],
    debut: NaiveDate,
    fin: NaiveDate,
    meps: &[NaiveDate],
    details: &[DetailRun],
) -> Vec<JourTimeline> {
    let lo = runs.iter().map(|r| r.date).fold(debut, NaiveDate::min);
    let hi = runs.iter().map(|r| r.date).fold(fin, NaiveDate::max);

    let mut feries: HashMap<NaiveDate, &'static str> = HashMap::new();
    for annee in lo.year()..=hi.year() {
        feries.extend(crate::calendrier::feries(annee));
    }

    let premiere_mep = meps.iter().min().copied();
    let mut par_date: HashMap<NaiveDate, Vec<RunJour>> = HashMap::new();
    for r in runs {
        let ecart = ecart_de(r, debut, fin, premiere_mep);
        let detail = match ecart {
            None => details.iter().find(|d| d.run_num == r.num).cloned(),
            Some(_) => None,
        };
        par_date.entry(r.date).or_default().push(RunJour {
            num: r.num.clone(),
            jjs: r.jjs.clone(),
            exclu: r.exclu,
            ecart,
            detail,
        });
    }
    for v in par_date.values_mut() {
        v.sort_by(|a, b| a.num.cmp(&b.num));
    }

    let mut out = Vec::new();
    let mut jour = lo;
    while jour <= hi {
        out.push(JourTimeline {
            date: jour.to_string(),
            jour_semaine: JOURS_SEMAINE[jour.weekday().num_days_from_monday() as usize],
            weekend: matches!(jour.weekday(), Weekday::Sat | Weekday::Sun),
            ferie: feries.get(&jour).copied(),
            jalons: Vec::new(),
            runs: par_date.remove(&jour).unwrap_or_default(),
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

    fn detail(num: &str, vise: usize, place: usize) -> DetailRun {
        DetailRun {
            run_num: num.into(),
            run_date: "2026-07-09".into(),
            jjs: vec![8],
            mep_id: 1,
            mep_date: "2026-07-08".into(),
            vise,
            report_entrant: 0,
            stock: 240,
            place,
            reliquat: 0,
        }
    }

    #[test]
    fn run_hors_fenetre_reste_visible_avec_son_motif() {
        // Sans motif affiché, une cible non atteinte reste inexplicable :
        // c'est le défaut de la v1 que ce lot corrige.
        let t = timeline(
            &[run("3327", "2026-07-22", &[19])],
            d("2026-07-10"),
            d("2026-07-20"),
            &[d("2026-07-11")],
            &[],
        );
        let j = t.iter().find(|j| j.date == "2026-07-22").unwrap();
        assert_eq!(j.runs[0].ecart, Some(Ecart::HorsFenetre));
        assert_eq!(j.runs[0].detail, None, "un run écarté n'a pas de chiffres");
    }

    #[test]
    fn run_le_jour_meme_de_la_premiere_mep_est_ecarte() {
        // Le filtre de runs_utilisables est STRICT (`r.date > premiere`) : un
        // run tombant le jour de la MEP est écarté lui aussi. C'est ce cas qui
        // interdit le libellé « avant la première MEP ».
        let t = timeline(
            &[run("3319", "2026-07-08", &[6])],
            d("2026-07-01"),
            d("2026-07-20"),
            &[d("2026-07-08")],
            &[],
        );
        let j = t.iter().find(|j| j.date == "2026-07-08").unwrap();
        assert_eq!(j.runs[0].ecart, Some(Ecart::MepNonPassee));
    }

    #[test]
    fn runs_sur_les_bornes_de_fenetre_sont_retenus() {
        // `r.date < debut || r.date > fin` compare en strict : les bornes
        // elles-mêmes appartiennent à la fenêtre. Muter l'une en `<=`/`>=`
        // ferait basculer à tort en `HorsFenetre` un run posé pile sur
        // `debut` ou `fin`. La non-divergence avec `runs_utilisables` est,
        // elle, du ressort des tests de miroir.
        let t = timeline(
            &[run("3330", "2026-07-01", &[1]), run("3331", "2026-07-20", &[20])],
            d("2026-07-01"),
            d("2026-07-20"),
            &[d("2026-06-20")],
            &[],
        );
        assert_eq!(
            t.iter().find(|j| j.date == "2026-07-01").unwrap().runs[0].ecart,
            None,
            "le premier jour de la fenêtre est retenu"
        );
        assert_eq!(
            t.iter().find(|j| j.date == "2026-07-20").unwrap().runs[0].ecart,
            None,
            "le dernier jour de la fenêtre est retenu"
        );
    }

    #[test]
    fn run_hors_fenetre_et_avant_la_premiere_mep_affiche_hors_fenetre() {
        // Le motif affiché est un conseil d'action déguisé. Un run du 15 juin,
        // avec une fenêtre qui commence en juillet et une première MEP le
        // 5 juillet, échoue aux deux filtres à la fois. Afficher
        // `MepNonPassee` laisserait croire qu'avancer la MEP suffirait — c'est
        // faux, ce run reste hors fenêtre quoi qu'il arrive sur les MEP.
        // `HorsFenetre` est le seul motif qui décrit un blocage qui tient
        // indépendamment des MEP : c'est le bon conseil.
        let t = timeline(
            &[run("3300", "2026-06-15", &[15])],
            d("2026-07-01"),
            d("2026-07-20"),
            &[d("2026-07-05")],
            &[],
        );
        let j = t.iter().find(|j| j.date == "2026-06-15").unwrap();
        assert_eq!(j.runs[0].ecart, Some(Ecart::HorsFenetre));
    }

    #[test]
    fn exclusion_manuelle_prime_sur_les_autres_motifs() {
        // L'exclusion est le seul motif que l'utilisateur pilote depuis
        // l'écran : elle doit rester lisible même sur un run par ailleurs
        // écarté pour une autre raison, sinon décocher la case n'a aucun
        // effet visible. Deux cas, un par motif concurrent.

        // Cas 1 : le run est aussi hors fenêtre → Exclu > HorsFenetre.
        let mut r = run("3321", "2026-07-30", &[9]);
        r.exclu = true;
        let t = timeline(&[r], d("2026-07-01"), d("2026-07-20"), &[d("2026-07-05")], &[]);
        let j = t.iter().find(|j| j.date == "2026-07-30").unwrap();
        assert_eq!(j.runs[0].ecart, Some(Ecart::Exclu));
        assert!(j.runs[0].exclu);

        // Cas 2 : le run est dans la fenêtre, avant la MEP → Exclu > MepNonPassee.
        let mut r2 = run("3322", "2026-07-04", &[4]);
        r2.exclu = true;
        let t2 = timeline(&[r2], d("2026-07-01"), d("2026-07-20"), &[d("2026-07-05")], &[]);
        let j2 = t2.iter().find(|j| j.date == "2026-07-04").unwrap();
        assert_eq!(j2.runs[0].ecart, Some(Ecart::Exclu));
    }

    #[test]
    fn run_retenu_porte_ses_chiffres() {
        let t = timeline(
            &[run("3320", "2026-07-09", &[8])],
            d("2026-07-01"),
            d("2026-07-20"),
            &[d("2026-07-08")],
            &[detail("3320", 143, 143)],
        );
        let j = t.iter().find(|j| j.date == "2026-07-09").unwrap();
        assert_eq!(j.runs[0].ecart, None);
        assert_eq!(j.runs[0].detail.as_ref().unwrap().vise, 143);
    }

    #[test]
    fn run_ecarte_ignore_le_detail_fourni_pour_lui() {
        // Les tests de runs écartés passent tous `details: &[]` : l'assertion
        // `detail == None` y est vacante puisqu'aucun détail n'existe pour
        // personne. Ici un détail EXISTE bel et bien pour le run écarté — il
        // ne doit quand même pas lui être attaché.
        let t = timeline(
            &[run("3327", "2026-07-22", &[19])],
            d("2026-07-10"),
            d("2026-07-20"),
            &[d("2026-07-11")],
            &[detail("3327", 50, 50)],
        );
        let j = t.iter().find(|j| j.date == "2026-07-22").unwrap();
        assert_eq!(j.runs[0].ecart, Some(Ecart::HorsFenetre));
        assert_eq!(
            j.runs[0].detail, None,
            "un détail existant ne doit pas être attaché à un run écarté"
        );
    }

    #[test]
    fn deux_runs_le_meme_jour_sont_rendus_tries_par_numero() {
        // `parse_runs_csv` refuse deux runs à la même date, mais
        // `PlanParams::calendrier` ne le revérifie pas en reconstruisant les
        // runs depuis les paramètres persistés — et c'est ce chemin-là qui
        // alimente l'écran. En perdre un en silence serait la faute que ce
        // lot corrige. Insérés en ordre inverse (3321 avant 3320) : sans le
        // tri par numéro, l'ordre resterait celui d'insertion.
        let t = timeline(
            &[run("3321", "2026-07-09", &[9]), run("3320", "2026-07-09", &[8])],
            d("2026-07-01"),
            d("2026-07-20"),
            &[d("2026-07-08")],
            &[],
        );
        let j = t.iter().find(|j| j.date == "2026-07-09").unwrap();
        assert_eq!(j.runs.len(), 2);
        assert_eq!(j.runs[0].num, "3320");
        assert_eq!(j.runs[1].num, "3321");
    }

    #[test]
    fn sans_aucune_mep_tout_run_est_ecarte() {
        // `runs_utilisables` ne rend rien sans MEP : il n'y a rien à facturer.
        // C'est l'état initial normal de l'écran, avant toute saisie — pas un
        // cas limite — d'où un motif distinct de `MepNonPassee` : l'action
        // utile est de créer une MEP, pas de décaler une date.
        let t = timeline(
            &[run("3320", "2026-07-09", &[8])],
            d("2026-07-01"),
            d("2026-07-20"),
            &[],
            &[],
        );
        let j = t.iter().find(|j| j.date == "2026-07-09").unwrap();
        assert_eq!(j.runs[0].ecart, Some(Ecart::AucuneMep));
    }

    #[test]
    fn aucune_mep_se_distingue_de_mep_non_passee() {
        // Les deux motifs viennent du même filtre MEP de `runs_utilisables`
        // (`meps` vide, ou run non postérieur à la première), mais pas du
        // même conseil : créer une MEP n'est pas décaler une date. Sans ce
        // test, rien n'empêcherait de les refondre en un seul motif.
        let sans_mep = timeline(
            &[run("3320", "2026-07-09", &[8])],
            d("2026-07-01"),
            d("2026-07-20"),
            &[],
            &[],
        );
        assert_eq!(
            sans_mep.iter().find(|j| j.date == "2026-07-09").unwrap().runs[0].ecart,
            Some(Ecart::AucuneMep)
        );

        let avec_mep = timeline(
            &[run("3320", "2026-07-09", &[8])],
            d("2026-07-01"),
            d("2026-07-20"),
            &[d("2026-07-10")],
            &[],
        );
        assert_eq!(
            avec_mep.iter().find(|j| j.date == "2026-07-09").unwrap().runs[0].ecart,
            Some(Ecart::MepNonPassee)
        );
    }

    #[test]
    fn les_runs_sans_ecart_sont_exactement_ceux_que_retient_le_moteur() {
        // `ecart_de` rejoue à la main le filtre de `calendrier::runs_utilisables`
        // au lieu de l'appeler — il faut bien un motif, que le filtre ne rend
        // pas. Deux implémentations de la même règle vivent donc dans deux
        // modules : sans ce test, déplacer une borne dans `calendrier` ferait
        // afficher « retenu » des runs que le plan a écartés, sans un bruit.
        // L'échantillon pose un run pile sur la première MEP elle-même (C) et
        // un pile sur `fin` (E) : ce sont deux des bornes exactes que
        // `runs_utilisables` compare, et un glissement de l'une resterait
        // invisible sans un run posé dessus. Le run exclu (B) referme le
        // quatrième filtre (`!r.exclu`) ; sa priorité sur MepNonPassee est,
        // elle, vérifiée par un cas dédié dans
        // `exclusion_manuelle_prime_sur_les_autres_motifs`, pas ici. La borne
        // `debut` ne peut pas rejoindre cet échantillon : elle exigerait une
        // première MEP antérieure à `debut`, incompatible avec C qui doit
        // rester dans la fenêtre — elle est couverte séparément par
        // `le_run_du_debut_de_fenetre_suit_le_moteur`.
        let mut exclu = run("B", "2026-07-04", &[4]);
        exclu.exclu = true;
        let rs = vec![
            run("A", "2026-07-03", &[3]),  // avant la première MEP
            exclu,                          // exclu, dans la fenêtre, avant la MEP
            run("C", "2026-07-05", &[5]),  // écarté, pile sur la première MEP
            run("D", "2026-07-15", &[15]), // retenu, au milieu de la fenêtre
            run("E", "2026-07-20", &[20]), // retenu, pile sur la fin de fenêtre
            run("F", "2026-07-25", &[25]), // hors fenêtre
        ];
        let (debut, fin) = (d("2026-07-01"), d("2026-07-20"));
        let meps = vec![d("2026-07-05")];

        let t = timeline(&rs, debut, fin, &meps, &[]);

        let mut affiches: Vec<String> = t
            .iter()
            .flat_map(|j| &j.runs)
            .filter(|r| r.ecart.is_none())
            .map(|r| r.num.clone())
            .collect();
        let mut retenus: Vec<String> =
            crate::calendrier::runs_utilisables(&rs, debut, fin, &meps)
                .iter()
                .map(|r| r.num.clone())
                .collect();
        // `runs_utilisables` préserve l'ordre d'entrée, `timeline` sort en
        // ordre chronologique puis par numéro : les deux ordres coïncident en
        // production (`PlanParams::calendrier` trie déjà ainsi), mais ici on
        // veut comparer l'ENSEMBLE retenu, pas son ordre.
        affiches.sort();
        retenus.sort();

        assert_eq!(affiches, retenus, "l'écran et le moteur doivent retenir le même ensemble de runs");
        assert_eq!(affiches, vec!["D", "E"], "D et E sont les deux seuls à passer les trois filtres");
    }

    #[test]
    fn le_run_du_debut_de_fenetre_suit_le_moteur() {
        // Séparé du test miroir principal : y ajouter un run pile sur `debut`
        // exigerait une première MEP antérieure à `debut`, incompatible avec
        // le run C posé sur la MEP elle-même là-bas (qui doit, lui, rester
        // dans la fenêtre) — les deux bornes ne peuvent pas être observées
        // avec la même première MEP. Referme la borne manquante : si
        // `runs_utilisables` mutait `r.date >= debut` en `r.date > debut`,
        // ce run pile sur `debut` serait exclu par le moteur réel tout en
        // restant retenu par la copie de `timeline` — divergence détectée.
        let rs = vec![run("H", "2026-07-01", &[1])];
        let (debut, fin) = (d("2026-07-01"), d("2026-07-20"));
        let meps = vec![d("2026-06-20")];

        let affiches: Vec<String> = timeline(&rs, debut, fin, &meps, &[])
            .iter()
            .flat_map(|j| &j.runs)
            .filter(|r| r.ecart.is_none())
            .map(|r| r.num.clone())
            .collect();
        let retenus: Vec<String> = crate::calendrier::runs_utilisables(&rs, debut, fin, &meps)
            .iter()
            .map(|r| r.num.clone())
            .collect();

        assert_eq!(affiches, retenus, "l'écran et le moteur doivent retenir le même ensemble de runs");
        assert_eq!(affiches, vec!["H"], "le run pile sur le début de fenêtre est retenu");
    }
}
