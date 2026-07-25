//! Rapport HTML du plan de charge — livrable DISTINCT du rapport de run
//! (`report.rs`), avec lequel il partage style et helpers d'échappement.
//!
//! Contenu v1 : indicateurs, avertissements, table des MEP et Runs de
//! Facturation, répartition par plateforme (plan vs pool). Les courbes
//! (cumulée, charge par jour civil) sont hors périmètre v1.
//!
//! Toute valeur d'origine CSV ou SMP passe par `esc` : ce sont des entrées non
//! fiables, comme dans le rapport de run.

use crate::plan::{LignePlan, Origine};
use crate::report::{esc, fmt_int, CSS};
use std::collections::BTreeMap;

pub struct PlanReportData<'a> {
    pub fichier: &'a str,
    pub date_longue: &'a str,
    pub version: &'a str,
    pub lignes: &'a [LignePlan],
    pub aujourdhui: chrono::NaiveDate,
    /// Pool éligible par plateforme, pour la comparaison plan vs pool.
    pub pool_par_pa: &'a BTreeMap<String, usize>,
    pub avertissements: &'a [String],
}

/// Comptes actifs (les retirés sont exclus partout : ils ne sont pas à livrer).
fn actives(lignes: &[LignePlan]) -> Vec<&LignePlan> {
    lignes.iter().filter(|l| !l.retiree()).collect()
}

pub fn render(d: &PlanReportData) -> String {
    let actifs = actives(d.lignes);
    let total = actifs.len() as u64;
    let geles = actifs.iter().filter(|l| l.gelee(d.aujourdhui)).count() as u64;
    let manuels = actifs.iter().filter(|l| l.origine == Origine::Manuel).count() as u64;
    let couverture = actifs.iter().filter(|l| l.origine == Origine::Couverture).count() as u64;
    let retires = (d.lignes.len() - actifs.len()) as u64;

    let mut meps: Vec<(usize, String)> = actifs
        .iter()
        .map(|l| (l.mep_id, l.mep_date.to_string()))
        .collect();
    meps.sort();
    meps.dedup();

    let fichier = esc(d.fichier);
    let mut html = String::with_capacity(16 * 1024);
    html.push_str("<!doctype html>\n<html lang=\"fr\">\n<head>\n<meta charset=\"utf-8\">\n");
    html.push_str(&format!(
        "<title>Plan de charge — {fichier}</title>\n<style>{CSS}</style>\n</head>\n\
         <body>\n<div class=\"page\">\n"
    ));
    html.push_str(&format!(
        "<header>\n<div class=\"wordmark\">SUPER POPAUL</div>\n\
         <h1>Plan de charge — montée en charge de la facturation</h1>\n\
         <p class=\"meta\">Fichier <b>{fichier}</b> · établi le <b>{}</b> · \
         <b>{}</b> comptes de facturation sur <b>{}</b> mise(s) en production</p>\n</header>\n",
        esc(d.date_longue),
        fmt_int(total),
        meps.len(),
    ));

    // Indicateurs.
    html.push_str("<section class=\"cards\">\n");
    for (valeur, libelle) in [
        (fmt_int(total), "comptes planifiés"),
        (fmt_int(geles), "gelés (MEP passée)"),
        (fmt_int(manuels), "retouches manuelles"),
        (fmt_int(couverture), "placés pour couverture"),
        (fmt_int(retires), "retirés"),
    ] {
        html.push_str(&format!(
            "<div class=\"card\"><div class=\"big\">{valeur}</div>\
             <div class=\"lbl\">{libelle}</div></div>\n"
        ));
    }
    html.push_str("</section>\n");

    if !d.avertissements.is_empty() {
        html.push_str("<section><h2>Avertissements</h2>\n<ul>\n");
        for a in d.avertissements {
            html.push_str(&format!("<li>{}</li>\n", esc(a)));
        }
        html.push_str("</ul>\n</section>\n");
    }

    // MEP et runs. Le fichier de chaque MEP est cumulatif : on affiche le
    // volume propre ET le cumul, sinon le lecteur croit livrer moins.
    html.push_str(
        "<section><h2>Mises en production et Runs de Facturation</h2>\n\
         <p class=\"meta\">Le fichier de chaque MEP est <b>cumulatif</b> : il contient \
         aussi les comptes des MEP précédentes.</p>\n\
         <table><tr><th>MEP</th><th>Date</th><th>Run</th><th>Date du run</th>\
         <th>JJ</th><th>Comptes</th><th>Cumul</th></tr>\n",
    );
    let mut par_run: BTreeMap<(usize, String, String, String), Vec<&LignePlan>> = BTreeMap::new();
    for l in &actifs {
        par_run
            .entry((l.mep_id, l.mep_date.to_string(), l.run_date.to_string(), l.run_num.clone()))
            .or_default()
            .push(l);
    }
    let mut cumul = 0u64;
    for ((mep_id, mep_date, run_date, run_num), lignes) in &par_run {
        cumul += lignes.len() as u64;
        let mut jjs: Vec<u8> = lignes.iter().map(|l| l.jj).collect();
        jjs.sort_unstable();
        jjs.dedup();
        html.push_str(&format!(
            "<tr><td>{mep_id}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td>\
             <td>{}</td><td>{}</td></tr>\n",
            esc(mep_date),
            esc(run_num),
            esc(run_date),
            esc(&jjs.iter().map(u8::to_string).collect::<Vec<_>>().join(", ")),
            fmt_int(lignes.len() as u64),
            fmt_int(cumul),
        ));
    }
    html.push_str("</table>\n</section>\n");

    // Plan vs pool : le plan doit coller à la distribution du pool, chaque
    // plateforme étant représentée.
    html.push_str(
        "<section><h2>Répartition par plateforme</h2>\n\
         <p class=\"meta\">Part de chaque plateforme dans le plan, comparée à sa part \
         du pool éligible.</p>\n\
         <table><tr><th>Plateforme</th><th>Plan</th><th>Pool éligible</th></tr>\n",
    );
    let mut plan_par_pa: BTreeMap<&str, u64> = BTreeMap::new();
    for l in &actifs {
        *plan_par_pa.entry(l.pa.as_str()).or_insert(0) += 1;
    }
    let pool_total: usize = d.pool_par_pa.values().sum();
    let mut noms: Vec<&str> = plan_par_pa.keys().copied().collect();
    for n in d.pool_par_pa.keys() {
        if !plan_par_pa.contains_key(n.as_str()) {
            noms.push(n.as_str());
        }
    }
    noms.sort_by_key(|n| std::cmp::Reverse(d.pool_par_pa.get(*n).copied().unwrap_or(0)));
    for nom in noms {
        let p = plan_par_pa.get(nom).copied().unwrap_or(0);
        let e = d.pool_par_pa.get(nom).copied().unwrap_or(0) as u64;
        html.push_str(&format!(
            "<tr><td>{}</td><td>{} ({})</td><td>{} ({})</td></tr>\n",
            esc(nom),
            fmt_int(p),
            pourcent(p, total),
            fmt_int(e),
            pourcent(e, pool_total as u64),
        ));
    }
    html.push_str("</table>\n</section>\n");

    html.push_str(&format!(
        "<footer class=\"meta\">Super Popaul {} — plan de charge</footer>\n\
         </div>\n</body>\n</html>\n",
        esc(d.version)
    ));
    html
}

/// Pourcentage à une décimale, virgule française. Total nul → « — » plutôt
/// qu'un « 0,0 % » qui laisserait croire à une mesure.
fn pourcent(part: u64, total: u64) -> String {
    if total == 0 {
        return "—".into();
    }
    format!("{:.1} %", 100.0 * part as f64 / total as f64).replace('.', ",")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jour(iso: &str) -> chrono::NaiveDate {
        chrono::NaiveDate::parse_from_str(iso, "%Y-%m-%d").unwrap()
    }

    fn ligne(cf: &str, pa: &str, mep: &str, origine: Origine) -> LignePlan {
        LignePlan {
            cf: cf.into(),
            participant: "0225:1".into(),
            jj: 5,
            raison_sociale: "ACME".into(),
            pa: pa.into(),
            mep_id: 1,
            mep_date: jour(mep),
            run_num: "R1".into(),
            run_date: jour("2026-08-11"),
            origine,
            in_directory: true,
            resolved_at: 0,
            planned_at: 0,
            retire: None,
        }
    }

    fn data<'a>(
        lignes: &'a [LignePlan],
        pool: &'a BTreeMap<String, usize>,
        warns: &'a [String],
    ) -> PlanReportData<'a> {
        PlanReportData {
            fichier: "clients.csv",
            date_longue: "25 juillet 2026",
            version: "1.1.0",
            lignes,
            aujourdhui: jour("2026-07-25"),
            pool_par_pa: pool,
            avertissements: warns,
        }
    }

    #[test]
    fn rapport_compte_les_actifs_et_exclut_les_retires() {
        let mut r = ligne("CF2", "Cegedim", "2026-08-01", Origine::Auto);
        r.retire = Some(crate::plan::Retrait { le: 1, motif: "clôturé".into() });
        let lignes = vec![ligne("CF1", "Cegedim", "2026-08-01", Origine::Auto), r];
        let pool = BTreeMap::from([("Cegedim".to_string(), 10usize)]);
        let html = render(&data(&lignes, &pool, &[]));
        assert!(html.contains("comptes planifiés"));
        assert!(html.contains("retirés"));
        // Un seul compte actif malgré deux lignes.
        assert!(html.contains(">1</div><div class=\"lbl\">comptes planifiés"), "{html}");
    }

    #[test]
    fn rapport_echappe_les_donnees_non_fiables() {
        // Nom de plateforme issu d'un SMP : entrée non fiable.
        let lignes = vec![ligne("CF1", "<script>alert(1)</script>", "2026-08-01", Origine::Auto)];
        let pool = BTreeMap::new();
        let html = render(&data(&lignes, &pool, &[]));
        assert!(!html.contains("<script>alert"), "injection non échappée");
        assert!(html.contains("&lt;script&gt;"));
    }

    #[test]
    fn rapport_echappe_aussi_les_avertissements() {
        let lignes = vec![ligne("CF1", "PA", "2026-08-01", Origine::Auto)];
        let pool = BTreeMap::new();
        let warns = vec!["plateforme <b>X</b> : rien".to_string()];
        let html = render(&data(&lignes, &pool, &warns));
        assert!(html.contains("&lt;b&gt;X&lt;/b&gt;"), "{html}");
    }

    #[test]
    fn rapport_affiche_le_cumul_par_mep() {
        let mut l2 = ligne("CF2", "Cegedim", "2026-10-01", Origine::Auto);
        l2.mep_id = 2;
        l2.run_num = "R2".into();
        let lignes = vec![ligne("CF1", "Cegedim", "2026-08-01", Origine::Auto), l2];
        let pool = BTreeMap::from([("Cegedim".to_string(), 5usize)]);
        let html = render(&data(&lignes, &pool, &[]));
        assert!(html.contains("cumulatif"), "le lecteur doit savoir que les fichiers cumulent");
        assert!(html.contains("<td>R2</td>"));
    }

    #[test]
    fn pourcent_total_nul_ne_fabrique_pas_une_mesure() {
        assert_eq!(pourcent(0, 0), "—");
        assert_eq!(pourcent(1, 4), "25,0 %");
    }
}
