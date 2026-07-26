//! Graphes SVG du rapport — aire cumulée et barres empilées.
//!
//! Module PUR : il reçoit des nombres et des étiquettes, il rend une chaîne
//! SVG. Il ne connaît ni plan, ni run, ni facture. Aucun JS, aucune dépendance
//! graphique : les couleurs viennent des variables CSS déjà définies dans la
//! constante `CSS` de `report`, qui s'appliquent au SVG inline.
//!
//! Les étiquettes proviennent de fichiers fournis par des tiers (numéros de run
//! du `runs.csv`) : elles passent toutes par `esc`.

use crate::report::{esc, fmt_int};
use chrono::NaiveDate;

/// Borne haute « ronde » d'un axe et pas de ses graduations, visant au plus
/// quatre intervalles. Un maximum nul rend `(1, 1)` : jamais de division par
/// zéro, jamais d'axe sans graduation.
fn echelle(max: u64) -> (u64, u64) {
    if max == 0 {
        return (1, 1);
    }
    let mut p = 1u64;
    loop {
        for mult in [1u64, 2, 5] {
            let pas = mult * p;
            if pas.saturating_mul(4) >= max {
                return (max.div_ceil(pas) * pas, pas);
            }
        }
        match p.checked_mul(10) {
            Some(n) => p = n,
            None => return (max, max),
        }
    }
}

/// Une barre du graphe de charge : `bas` empilé sous `haut`.
pub struct Barre {
    pub label: String,
    pub sous_label: String,
    pub bas: u64,
    pub haut: u64,
}

const W: f64 = 760.0;
const X0: f64 = 52.0;
const X1: f64 = 748.0;
const Y0: f64 = 16.0;
const Y1: f64 = 194.0;

/// Barres empilées : `bas` (premières factures) sous `haut` (récurrences).
pub fn barres_empilees(barres: &[Barre]) -> String {
    let mut s = String::with_capacity(4 * 1024);
    s.push_str(&format!(
        "<svg viewBox=\"0 0 {W} 240\" role=\"img\" aria-label=\"Charge par run\">"
    ));
    if barres.is_empty() {
        s.push_str(&format!(
            "<text class=\"tick mid\" x=\"{}\" y=\"120\">Aucun run retenu</text></svg>",
            W / 2.0
        ));
        return s;
    }

    let max = barres.iter().map(|b| b.bas + b.haut).max().unwrap_or(0);
    let (haut, pas) = echelle(max);
    let y = |v: u64| Y1 - (v as f64 / haut as f64) * (Y1 - Y0);

    let mut v = 0u64;
    while v <= haut {
        s.push_str(&format!(
            "<line class=\"grid\" x1=\"{X0}\" y1=\"{0:.1}\" x2=\"{X1}\" y2=\"{0:.1}\"></line>\
             <text class=\"tick end\" x=\"44\" y=\"{1:.1}\">{2}</text>",
            y(v),
            y(v) + 4.0,
            fmt_int(v)
        ));
        v += pas;
    }

    let bande = (X1 - X0) / barres.len() as f64;
    let largeur = (bande * 0.54).min(46.0);
    for (i, b) in barres.iter().enumerate() {
        let centre = X0 + bande * (i as f64 + 0.5);
        let x = centre - largeur / 2.0;
        if b.bas > 0 {
            s.push_str(&format!(
                "<rect class=\"b-first\" x=\"{x:.1}\" y=\"{:.1}\" width=\"{largeur:.1}\" height=\"{:.1}\"></rect>",
                y(b.bas),
                Y1 - y(b.bas)
            ));
        }
        if b.haut > 0 {
            s.push_str(&format!(
                "<rect class=\"b-rec\" x=\"{x:.1}\" y=\"{:.1}\" width=\"{largeur:.1}\" height=\"{:.1}\"></rect>",
                y(b.bas + b.haut),
                y(b.bas) - y(b.bas + b.haut)
            ));
        }
        s.push_str(&format!(
            "<text class=\"tick mid\" x=\"{centre:.1}\" y=\"212\">{}</text>\
             <text class=\"tick mid\" x=\"{centre:.1}\" y=\"226\">{}</text>",
            esc(&b.label),
            esc(&b.sous_label)
        ));
    }

    // Repère du pic : la barre la plus haute est celle qui dimensionne.
    if max > 0 {
        s.push_str(&format!(
            "<line class=\"b-peak\" x1=\"{X0}\" y1=\"{0:.1}\" x2=\"{X1}\" y2=\"{0:.1}\"></line>\
             <text class=\"tick end\" x=\"{X1}\" y=\"{1:.1}\">pic {2}</text>",
            y(max),
            y(max) - 5.0,
            fmt_int(max)
        ));
    }

    s.push_str(&format!(
        "<line class=\"axis\" x1=\"{X0}\" y1=\"{Y1}\" x2=\"{X1}\" y2=\"{Y1}\"></line></svg>"
    ));
    s
}

/// Un palier de la courbe cumulée : à cette date, le cumul vaut cette valeur.
pub struct Point {
    pub date: NaiveDate,
    pub valeur: u64,
}

/// Un repère vertical daté (mise en production).
pub struct JalonChart {
    pub date: NaiveDate,
    pub label: String,
}

/// Aire cumulée **en escalier** : le parc facturant saute à chaque run, il ne
/// croît pas continûment. Une courbe lissée suggérerait une progression
/// quotidienne qui n'existe pas.
pub fn aire_cumulee(
    points: &[Point],
    jalons: &[JalonChart],
    debut: NaiveDate,
    fin: NaiveDate,
) -> String {
    let mut s = String::with_capacity(4 * 1024);
    s.push_str(&format!(
        "<svg viewBox=\"0 0 {W} 240\" role=\"img\" aria-label=\"Parc facturant cumulé\">"
    ));
    if points.is_empty() {
        s.push_str(&format!(
            "<text class=\"tick mid\" x=\"{}\" y=\"120\">Aucune première facture planifiée</text></svg>",
            W / 2.0
        ));
        return s;
    }

    let jours = (fin - debut).num_days().max(1) as f64;
    let x = |d: NaiveDate| X0 + ((d - debut).num_days() as f64 / jours) * (X1 - X0);
    let max = points.iter().map(|p| p.valeur).max().unwrap_or(0);
    let (haut, pas) = echelle(max);
    let y = |v: u64| Y1 + 6.0 - (v as f64 / haut as f64) * (Y1 + 6.0 - Y0);

    let mut v = 0u64;
    while v <= haut {
        s.push_str(&format!(
            "<line class=\"grid\" x1=\"{X0}\" y1=\"{0:.1}\" x2=\"{X1}\" y2=\"{0:.1}\"></line>\
             <text class=\"tick end\" x=\"44\" y=\"{1:.1}\">{2}</text>",
            y(v),
            y(v) + 4.0,
            fmt_int(v)
        ));
        v += pas;
    }

    for jl in jalons {
        let jx = x(jl.date);
        s.push_str(&format!(
            "<line class=\"mep\" x1=\"{jx:.1}\" y1=\"{Y0}\" x2=\"{jx:.1}\" y2=\"{:.1}\"></line>\
             <text class=\"mep-lbl\" x=\"{:.1}\" y=\"27\">{}</text>",
            y(0),
            jx + 22.0,
            esc(&jl.label)
        ));
    }

    // Escalier : on avance à l'horizontale jusqu'à la date du run, puis on
    // monte d'un coup (H puis V, jamais de segment oblique).
    let base = y(0);
    let mut d = format!("M {X0:.1},{base:.1}");
    for p in points {
        d.push_str(&format!(" H {:.1} V {:.1}", x(p.date), y(p.valeur)));
    }
    d.push_str(&format!(" H {X1:.1}"));
    s.push_str(&format!("<path class=\"area\" d=\"{d} V {base:.1} Z\"></path>"));
    s.push_str(&format!("<path class=\"line\" d=\"{d}\"></path>"));

    s.push_str(&format!(
        "<line class=\"axis\" x1=\"{X0}\" y1=\"{base:.1}\" x2=\"{X1}\" y2=\"{base:.1}\"></line></svg>"
    ));
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn echelle_arrondie_au_palier_superieur() {
        assert_eq!(echelle(1800), (2000, 500), "0 500 1000 1500 2000");
        assert_eq!(echelle(3340), (4000, 1000), "0 1000 2000 3000 4000");
        assert_eq!(echelle(7), (8, 2));
    }

    #[test]
    fn echelle_dun_maximum_nul_ne_divise_pas_par_zero() {
        assert_eq!(echelle(0), (1, 1));
    }

    fn barre(label: &str, sous: &str, bas: u64, haut: u64) -> Barre {
        Barre { label: label.into(), sous_label: sous.into(), bas, haut }
    }

    #[test]
    fn barres_empilees_rend_une_barre_par_serie() {
        let b = vec![barre("R1", "11/08", 420, 0), barre("R2", "08/09", 610, 420)];
        let svg = barres_empilees(&b);
        assert!(svg.starts_with("<svg"), "{svg}");
        assert!(svg.ends_with("</svg>"));
        assert_eq!(svg.matches("class=\"b-first\"").count(), 2);
        // La barre sans récurrence ne doit pas produire de rectangle de hauteur nulle.
        assert_eq!(svg.matches("class=\"b-rec\"").count(), 1);
        assert!(svg.contains(">R1<") && svg.contains(">11/08<"));
    }

    #[test]
    fn barres_empilees_marque_le_pic() {
        // Le repère relie l'indicateur de tête à la figure.
        let b = vec![barre("R1", "11/08", 420, 0), barre("R2", "08/09", 610, 420)];
        let svg = barres_empilees(&b);
        assert!(svg.contains("class=\"b-peak\""), "{svg}");
        // \u{202F} : espace fine insécable, le séparateur de milliers de `fmt_int`.
        assert!(
            svg.contains("pic 1\u{202F}030"),
            "le pic vaut 610 + 420, séparateur de milliers compris"
        );
    }

    #[test]
    fn barres_empilees_serie_vide_rend_un_svg_valide() {
        let svg = barres_empilees(&[]);
        assert!(svg.starts_with("<svg") && svg.ends_with("</svg>"), "{svg}");
        assert!(svg.contains("Aucun run"), "l'absence doit se dire, pas se taire");
    }

    #[test]
    fn barres_empilees_valeurs_toutes_nulles_ne_divisent_pas_par_zero() {
        let svg = barres_empilees(&[barre("R1", "11/08", 0, 0)]);
        assert!(svg.starts_with("<svg"));
    }

    #[test]
    fn les_etiquettes_de_run_sont_echappees() {
        // Le numéro de run vient du runs.csv : entrée non fiable.
        let svg = barres_empilees(&[barre("<script>alert(1)</script>", "11/08", 1, 0)]);
        assert!(!svg.contains("<script>alert"), "injection non échappée : {svg}");
        assert!(svg.contains("&lt;script&gt;"));
    }

    fn j(iso: &str) -> NaiveDate {
        NaiveDate::parse_from_str(iso, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn aire_cumulee_trace_un_escalier_et_ses_jalons() {
        let pts = vec![
            Point { date: j("2026-08-11"), valeur: 420 },
            Point { date: j("2026-09-08"), valeur: 1030 },
        ];
        let jalons = vec![JalonChart { date: j("2026-08-01"), label: "MEP 1".into() }];
        let svg = aire_cumulee(&pts, &jalons, j("2026-08-01"), j("2026-09-30"));
        assert!(svg.starts_with("<svg") && svg.ends_with("</svg>"));
        assert!(svg.contains("class=\"area\"") && svg.contains("class=\"line\""));
        // Escalier : le tracé n'utilise que des segments horizontaux et verticaux.
        assert!(svg.contains(" H ") && svg.contains(" V "), "{svg}");
        assert!(!svg.contains(" C "), "aucune courbe de Bézier : le parc saute, il ne glisse pas");
        assert!(svg.contains("MEP 1"));
    }

    #[test]
    fn aire_cumulee_sans_point_rend_un_svg_valide() {
        let svg = aire_cumulee(&[], &[], j("2026-08-01"), j("2026-09-30"));
        assert!(svg.starts_with("<svg") && svg.ends_with("</svg>"), "{svg}");
        assert!(svg.contains("Aucune"));
    }

    #[test]
    fn aire_cumulee_fenetre_dun_seul_jour_ne_divise_pas_par_zero() {
        let pts = vec![Point { date: j("2026-08-01"), valeur: 5 }];
        let svg = aire_cumulee(&pts, &[], j("2026-08-01"), j("2026-08-01"));
        assert!(svg.starts_with("<svg"));
    }

    #[test]
    fn les_libelles_de_jalon_sont_echappes() {
        let jalons = vec![JalonChart { date: j("2026-08-01"), label: "<script>x</script>".into() }];
        let pts = vec![Point { date: j("2026-08-11"), valeur: 1 }];
        let svg = aire_cumulee(&pts, &jalons, j("2026-08-01"), j("2026-09-30"));
        assert!(!svg.contains("<script>x"), "{svg}");
        assert!(svg.contains("&lt;script&gt;"));
    }
}
