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
}
