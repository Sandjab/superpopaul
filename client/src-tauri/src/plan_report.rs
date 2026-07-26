//! Rapport HTML du plan de charge — livrable DISTINCT du rapport de run
//! (`report.rs`), avec lequel il partage style et helpers d'échappement.
//!
//! Contenu : indicateurs de trajectoire, avertissements, courbe du parc
//! facturant et charge par run (`charts`, alimentés par `charge`), table des
//! MEP et Runs de Facturation, répartition par plateforme (plan vs pool), et
//! bande de contrôle du plan.
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
    /// Pool éligible par jour de cycle, pour détecter les comptes hors d'atteinte.
    pub pool_par_jj: &'a BTreeMap<u8, usize>,
    /// Runs **retenus** du calendrier.
    pub runs: &'a [crate::calendrier::RunFacturation],
}

/// Comptes actifs (les retirés sont exclus partout : ils ne sont pas à livrer).
fn actives(lignes: &[LignePlan]) -> Vec<&LignePlan> {
    lignes.iter().filter(|l| !l.retiree()).collect()
}

/// Avertissements que le rapport déduit de son propre contenu.
///
/// Les avertissements de l'allocation ne sont pas persistés dans `PlanMeta` :
/// on ne peut pas les restituer. Ceux-ci portent la même information utile et
/// se recalculent sur des données fraîches, ce que le rapport fait déjà pour
/// le pool.
fn avertissements_derives(
    actifs: &[&LignePlan],
    pool_par_pa: &BTreeMap<String, usize>,
    pool_par_jj: &BTreeMap<u8, usize>,
    runs: &[crate::calendrier::RunFacturation],
) -> Vec<String> {
    let mut out = Vec::new();

    let servies: std::collections::HashSet<&str> = actifs.iter().map(|l| l.pa.as_str()).collect();
    for (pa, n) in pool_par_pa {
        if *n > 0 && !servies.contains(pa.as_str()) {
            out.push(format!(
                "plateforme « {pa} » : aucun compte planifié, alors que {n} comptes \
                 du pool lui appartiennent"
            ));
        }
    }

    let couverts: std::collections::HashSet<u8> =
        runs.iter().flat_map(|r| r.jjs.iter().copied()).collect();
    for (jj, n) in pool_par_jj {
        if *n > 0 && !couverts.contains(jj) {
            out.push(format!(
                "jour de cycle {jj} : {n} comptes hors d'atteinte — aucun run retenu \
                 ne le couvre"
            ));
        }
    }
    out
}

pub fn render(d: &PlanReportData) -> String {
    let actifs = actives(d.lignes);
    let total = actifs.len() as u64;
    let geles = actifs.iter().filter(|l| l.gelee(d.aujourdhui)).count() as u64;
    let manuels = actifs.iter().filter(|l| l.origine == Origine::Manuel).count() as u64;
    let couverture = actifs.iter().filter(|l| l.origine == Origine::Couverture).count() as u64;
    let retires = (d.lignes.len() - actifs.len()) as u64;

    let charge = crate::charge::charge(
        &actifs.iter().map(|l| (*l).clone()).collect::<Vec<_>>(),
        d.runs,
    );
    let pool_total: usize = d.pool_par_pa.values().sum();
    let servies = actifs
        .iter()
        .map(|l| l.pa.as_str())
        .collect::<std::collections::HashSet<_>>();
    let pa_du_pool = d.pool_par_pa.iter().filter(|(_, n)| **n > 0).count();
    let pic = charge.iter().max_by_key(|c| c.total());
    // Fin de montée en charge : dernier run portant une PREMIÈRE facture, non
    // dernier run de la série — sinon l'indicateur mesurerait la longueur du
    // runs.csv, pas celle du déploiement.
    let fin = charge.iter().rev().find(|c| c.premieres > 0);

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

    // ① Trajectoire : où va le plan, pas comment il a été fabriqué.
    html.push_str("<section class=\"kpis\">\n");
    html.push_str(&format!(
        "<div class=\"kpi gold\"><div class=\"v\">{}</div>\
         <div class=\"l\">comptes planifiés</div>\
         <div class=\"abs\">sur <b>{}</b> éligibles · {}</div></div>\n",
        fmt_int(total),
        fmt_int(pool_total as u64),
        pourcent(total, pool_total as u64)
    ));
    if let Some(c) = fin {
        html.push_str(&format!(
            "<div class=\"kpi\"><div class=\"v\">{}</div>\
             <div class=\"l\">fin de montée en charge</div>\
             <div class=\"abs\">dernier run portant un démarrage</div></div>\n",
            c.date.format("%d/%m/%Y")
        ));
    }
    if let Some(p) = pic {
        html.push_str(&format!(
            "<div class=\"kpi amber\"><div class=\"v\">{}</div>\
             <div class=\"l\">pic de charge (un run)</div>\
             <div class=\"abs\">run <b>{}</b> le <b>{}</b></div></div>\n",
            fmt_int(p.total() as u64),
            esc(&p.num),
            p.date.format("%d/%m/%Y")
        ));
    }
    html.push_str(&format!(
        "<div class=\"kpi\"><div class=\"v\">{} <span class=\"unit\">/ {}</span></div>\
         <div class=\"l\">plateformes couvertes</div></div>\n",
        servies.len(),
        pa_du_pool
    ));
    html.push_str("</section>\n");

    let avertissements = avertissements_derives(&actifs, d.pool_par_pa, d.pool_par_jj, d.runs);
    if !avertissements.is_empty() {
        html.push_str("<section class=\"warn\">\n<h2>Avertissements</h2>\n<ul>\n");
        for a in &avertissements {
            html.push_str(&format!("<li>{}</li>\n", esc(a)));
        }
        html.push_str("</ul>\n</section>\n");
    }

    // ③ Parc facturant — cumul en escalier.
    let mut cumul = 0u64;
    let points: Vec<crate::charts::Point> = charge
        .iter()
        .filter(|c| c.premieres > 0)
        .map(|c| {
            cumul += c.premieres as u64;
            crate::charts::Point { date: c.date, valeur: cumul }
        })
        .collect();
    // `mep_id` est déjà 1-basé : le réindexer ferait diverger le libellé du
    // graphe de celui de la table, qui affiche `mep_id` tel quel.
    let jalons: Vec<crate::charts::JalonChart> = meps
        .iter()
        .map(|(id, date)| crate::charts::JalonChart {
            date: chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").unwrap_or(d.aujourdhui),
            label: format!("MEP {id}"),
        })
        .collect();
    let debut = jalons
        .iter()
        .map(|j| j.date)
        .min()
        .into_iter()
        .chain(points.first().map(|p| p.date))
        .min()
        .unwrap_or(d.aujourdhui);
    let fin_axe = charge.last().map(|c| c.date).unwrap_or(debut);
    html.push_str(
        "<h2>Parc facturant</h2>\n<p class=\"h2sub\">Nombre de comptes ayant émis \
         leur première facture, cumulé au fil des runs. Les jalons marquent les mises \
         en production.</p>\n<div class=\"chart\">\n",
    );
    html.push_str(&crate::charts::aire_cumulee(&points, &jalons, debut, fin_axe));
    html.push_str("\n</div>\n");

    // ④ Charge par run — barres empilées.
    let barres: Vec<crate::charts::Barre> = charge
        .iter()
        .map(|c| crate::charts::Barre {
            label: c.num.clone(),
            sous_label: c.date.format("%d/%m").to_string(),
            bas: c.premieres as u64,
            haut: c.recurrences as u64,
        })
        .collect();
    html.push_str(
        "<h2>Charge par run</h2>\n<p class=\"h2sub\">Factures émises à chaque run : \
         premières factures des comptes qui démarrent, et récurrences des comptes déjà \
         en production. Un compte facture une fois par mois civil, au premier run du \
         mois couvrant son jour de cycle.</p>\n<div class=\"chart\">\n",
    );
    html.push_str(&crate::charts::barres_empilees(&barres));
    html.push_str(
        "\n<div class=\"chart-legend\">\
         <span><i style=\"background:var(--gold)\"></i>premières factures</span>\
         <span><i style=\"background:var(--green-later)\"></i>récurrences</span>\
         </div>\n</div>\n",
    );

    // ⑤ MEP et runs. Le fichier de chaque MEP est cumulatif : on affiche le
    // volume propre ET le cumul, sinon le lecteur croit livrer moins.
    html.push_str(
        "<h2>Mises en production et Runs de Facturation</h2>\n\
         <p class=\"h2sub\">Le fichier de chaque MEP est <b>cumulatif</b> : il contient \
         aussi les comptes des MEP précédentes.</p>\n<div class=\"tbl\">\n<table>\n\
         <thead><tr><th>MEP</th><th>Date</th><th>Run</th><th>Date du run</th>\
         <th>Jours de cycle</th><th class=\"num\">Comptes</th>\
         <th class=\"num\">Cumul</th></tr></thead>\n<tbody>\n",
    );
    // Les dates restent des `NaiveDate` dans la clé : elles trient
    // chronologiquement et se formatent en jj/mm/aaaa comme le reste du
    // rapport, là où l'ISO de leur `to_string()` s'affichait tel quel.
    type CleRun = (usize, chrono::NaiveDate, chrono::NaiveDate, String);
    let mut par_run: BTreeMap<CleRun, Vec<&LignePlan>> = BTreeMap::new();
    for l in &actifs {
        par_run
            .entry((l.mep_id, l.mep_date, l.run_date, l.run_num.clone()))
            .or_default()
            .push(l);
    }
    let mut cumul_table = 0u64;
    let mut mep_precedente = usize::MAX;
    for ((mep_id, mep_date, run_date, run_num), lignes) in &par_run {
        cumul_table += lignes.len() as u64;
        let mut jjs: Vec<u8> = lignes.iter().map(|l| l.jj).collect();
        jjs.sort_unstable();
        jjs.dedup();
        let debut_mep = *mep_id != mep_precedente;
        mep_precedente = *mep_id;
        // Le gel est une propriété de la MEP : toutes les lignes du run la
        // partagent, `all` le dit sans supposer que le groupe est homogène.
        let gelee = lignes.iter().all(|l| l.gelee(d.aujourdhui));
        html.push_str(&format!(
            "<tr{}><td class=\"mep-cell\">{mep_id}{}</td><td>{}</td><td>{}</td>\
             <td>{}</td><td class=\"jj\">{}</td><td class=\"num\"><b>{}</b></td>\
             <td class=\"num\">{}</td></tr>\n",
            if debut_mep { " class=\"mep-start\"" } else { "" },
            if gelee { "<span class=\"frozen\">gelée</span>" } else { "" },
            mep_date.format("%d/%m/%Y"),
            esc(run_num),
            run_date.format("%d/%m/%Y"),
            esc(&jjs.iter().map(u8::to_string).collect::<Vec<_>>().join(", ")),
            fmt_int(lignes.len() as u64),
            fmt_int(cumul_table),
        ));
    }
    html.push_str("</tbody>\n</table>\n</div>\n");

    // ⑥ Plan vs pool : le plan doit coller à la distribution du pool, chaque
    // plateforme étant représentée.
    html.push_str(
        "<h2>Répartition par plateforme</h2>\n\
         <p class=\"h2sub\">Part de chaque plateforme dans le plan, comparée à sa part du \
         pool éligible. L'écart en points signale une plateforme sur- ou sous-servie.</p>\n\
         <div class=\"dist\">\n<div class=\"chart-legend\" style=\"margin-bottom:10px\">\
         <span><i style=\"background:var(--gold)\"></i>part du plan</span>\
         <span><i style=\"background:var(--pa-autres)\"></i>part du pool éligible</span>\
         </div>\n",
    );
    let mut plan_par_pa: BTreeMap<&str, u64> = BTreeMap::new();
    for l in &actifs {
        *plan_par_pa.entry(l.pa.as_str()).or_insert(0) += 1;
    }
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
        let part_plan = part(p, total);
        let part_pool = part(e, pool_total as u64);
        let ecart = part_plan - part_pool;
        html.push_str(&format!(
            "<div class=\"dist-row{}\"><div class=\"dist-name\">{}</div>\
             <div class=\"dist-bars\">\
             <div class=\"bar\"><i style=\"width:{part_plan:.1}%\"></i></div>\
             <div class=\"bar pool\"><i style=\"width:{part_pool:.1}%\"></i></div></div>\
             <div class=\"dist-n\"><b>{}</b> · {}<br>{} · {}</div>\
             <div class=\"dist-gap {}\">{}</div></div>\n",
            if p == 0 { " absent" } else { "" },
            esc(nom),
            fmt_int(p),
            pourcent(p, total),
            fmt_int(e),
            pourcent(e, pool_total as u64),
            if ecart >= 0.0 { "over" } else { "under" },
            fmt_ecart(ecart),
        ));
    }
    html.push_str("</div>\n");

    // ⑦ Contrôle du plan : ce qui a été figé ou repris à la main. Ton plus bas
    // que la trajectoire — c'est de la traçabilité, pas de la prévision.
    html.push_str(
        "<h2>Contrôle du plan</h2>\n<p class=\"h2sub\">Ce qui a été figé ou repris à la \
         main. Les comptes retirés sont exclus de tous les chiffres ci-dessus.</p>\n\
         <section class=\"kpis sub\">\n",
    );
    for (valeur, libelle) in [
        (geles, "gelés (MEP passée)"),
        (manuels, "retouches manuelles"),
        (couverture, "placés pour couverture"),
        (retires, "retirés"),
    ] {
        html.push_str(&format!(
            "<div class=\"kpi{}\"><div class=\"v\">{}</div>\
             <div class=\"l\">{libelle}</div></div>\n",
            if valeur > 0 { " on" } else { "" },
            fmt_int(valeur)
        ));
    }
    html.push_str("</section>\n");

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

/// Part en pourcentage pour une largeur de barre. Total nul → 0, pas de NaN.
fn part(x: u64, total: u64) -> f64 {
    if total == 0 {
        0.0
    } else {
        100.0 * x as f64 / total as f64
    }
}

/// Écart en points, signe toujours explicite, virgule française et vrai moins
/// typographique — c'est ce que montre la maquette validée.
fn fmt_ecart(points: f64) -> String {
    format!("{points:+.1} pt").replace('.', ",").replace('-', "−")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Le corps du rapport, feuille de style exclue.
    ///
    /// Le CSS est inliné dans chaque rapport : chercher une sous-chaîne dans le
    /// HTML entier fait matcher `font-size: 12px` ou un commentaire de règle.
    /// Trois tests de ce module s'y sont laissé prendre — assertions vertes sur
    /// une fonction qui ne produisait rien.
    fn corps(html: &str) -> &str {
        html.split("</style>")
            .nth(1)
            .expect("le rapport doit contenir une feuille de style")
    }

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

    fn runs_test() -> Vec<crate::calendrier::RunFacturation> {
        vec![crate::calendrier::RunFacturation {
            num: "R1".into(),
            date: jour("2026-08-11"),
            jjs: vec![5],
            exclu: false,
        }]
    }

    fn data<'a>(
        lignes: &'a [LignePlan],
        pool: &'a BTreeMap<String, usize>,
        pool_par_jj: &'a BTreeMap<u8, usize>,
        runs: &'a [crate::calendrier::RunFacturation],
    ) -> PlanReportData<'a> {
        PlanReportData {
            fichier: "clients.csv",
            date_longue: "25 juillet 2026",
            version: "1.1.0",
            lignes,
            aujourdhui: jour("2026-07-25"),
            pool_par_pa: pool,
            pool_par_jj,
            runs,
        }
    }

    #[test]
    fn avertit_sur_une_plateforme_du_pool_sans_compte_planifie() {
        let lignes = vec![ligne("CF1", "Cegedim", "2026-08-01", Origine::Auto)];
        let pool = BTreeMap::from([("Cegedim".to_string(), 10usize), ("Freedz".to_string(), 4)]);
        let jj = BTreeMap::from([(5u8, 14usize)]);
        let html = render(&data(&lignes, &pool, &jj, &runs_test()));
        // Sur la phrase d'alerte, pas sur le seul nom : la table « Répartition
        // par plateforme » liste déjà les plateformes du pool absentes du plan,
        // donc « Freedz » seul serait présent même sans avertissement.
        assert!(
            html.contains("plateforme « Freedz »"),
            "la plateforme non servie doit être nommée : {html}"
        );
    }

    #[test]
    fn avertit_sur_un_jour_de_cycle_hors_datteinte() {
        let lignes = vec![ligne("CF1", "Cegedim", "2026-08-01", Origine::Auto)];
        let pool = BTreeMap::from([("Cegedim".to_string(), 10usize)]);
        // Le jour 12 pèse 30 comptes mais aucun run retenu ne le couvre.
        let jj = BTreeMap::from([(5u8, 14usize), (12u8, 30usize)]);
        let html = render(&data(&lignes, &pool, &jj, &runs_test()));
        // Sur la phrase d'alerte, pas sur les nombres nus : le CSS inliné
        // contient « 12 » et « 30 » (tailles, marges), qui passeraient toujours.
        assert!(
            html.contains("jour de cycle 12"),
            "le jour de cycle orphelin doit être nommé : {html}"
        );
        assert!(
            html.contains("30 comptes hors d&#39;atteinte"),
            "son effectif aussi : c'est ce qui rend l'alerte actionnable"
        );
    }

    #[test]
    fn aucun_avertissement_quand_le_plan_couvre_tout() {
        let lignes = vec![ligne("CF1", "Cegedim", "2026-08-01", Origine::Auto)];
        let pool = BTreeMap::from([("Cegedim".to_string(), 10usize)]);
        let jj = BTreeMap::from([(5u8, 14usize)]);
        let html = render(&data(&lignes, &pool, &jj, &runs_test()));
        // Sur le titre balisé, pas sur le mot : le CSS inliné contient déjà un
        // commentaire « Avertissements » qui rendrait l'assertion ininterprétable.
        assert!(!html.contains("<h2>Avertissements</h2>"), "pas de section vide : {html}");
    }

    #[test]
    fn les_avertissements_derives_sont_echappes() {
        let lignes = vec![ligne("CF1", "Cegedim", "2026-08-01", Origine::Auto)];
        let pool = BTreeMap::from([
            ("Cegedim".to_string(), 10usize),
            ("<script>alert(1)</script>".to_string(), 4),
        ]);
        let jj = BTreeMap::from([(5u8, 14usize)]);
        let html = render(&data(&lignes, &pool, &jj, &runs_test()));
        assert!(!html.contains("<script>alert"), "injection non échappée");
        // Dans la phrase d'alerte : la table de répartition échappe déjà ce nom
        // de son côté, l'assertion doit viser l'avertissement lui-même.
        assert!(
            html.contains("plateforme « &lt;script&gt;alert(1)&lt;/script&gt; »"),
            "{html}"
        );
    }

    #[test]
    fn rapport_compte_les_actifs_et_exclut_les_retires() {
        let mut r = ligne("CF2", "Cegedim", "2026-08-01", Origine::Auto);
        r.retire = Some(crate::plan::Retrait { le: 1, motif: "clôturé".into() });
        let lignes = vec![ligne("CF1", "Cegedim", "2026-08-01", Origine::Auto), r];
        let pool = BTreeMap::from([("Cegedim".to_string(), 10usize)]);
        let html = render(&data(&lignes, &pool, &BTreeMap::new(), &runs_test()));
        let c = corps(&html);
        assert!(c.contains("comptes planifiés"));
        assert!(c.contains("retirés"));
        // Un seul compte actif malgré deux lignes.
        assert!(c.contains(">1</div><div class=\"l\">comptes planifiés"), "{c}");
    }

    #[test]
    fn rapport_echappe_les_donnees_non_fiables() {
        // Nom de plateforme issu d'un SMP : entrée non fiable.
        let lignes = vec![ligne("CF1", "<script>alert(1)</script>", "2026-08-01", Origine::Auto)];
        let pool = BTreeMap::new();
        let html = render(&data(&lignes, &pool, &BTreeMap::new(), &runs_test()));
        assert!(!html.contains("<script>alert"), "injection non échappée");
        assert!(html.contains("&lt;script&gt;"));
    }

    #[test]
    fn rapport_affiche_le_cumul_par_mep() {
        let mut l2 = ligne("CF2", "Cegedim", "2026-10-01", Origine::Auto);
        l2.mep_id = 2;
        l2.run_num = "R2".into();
        let lignes = vec![ligne("CF1", "Cegedim", "2026-08-01", Origine::Auto), l2];
        let pool = BTreeMap::from([("Cegedim".to_string(), 5usize)]);
        let html = render(&data(&lignes, &pool, &BTreeMap::new(), &runs_test()));
        assert!(html.contains("cumulatif"), "le lecteur doit savoir que les fichiers cumulent");
        assert!(html.contains("<td>R2</td>"));
    }

    #[test]
    fn pourcent_total_nul_ne_fabrique_pas_une_mesure() {
        assert_eq!(pourcent(0, 0), "—");
        assert_eq!(pourcent(1, 4), "25,0 %");
    }

    #[test]
    fn les_indicateurs_de_trajectoire_sont_calcules() {
        let mut l2 = ligne("CF2", "Cegedim", "2026-08-01", Origine::Auto);
        l2.jj = 5;
        let lignes = vec![ligne("CF1", "Cegedim", "2026-08-01", Origine::Auto), l2];
        let pool = BTreeMap::from([("Cegedim".to_string(), 8usize)]);
        let jj = BTreeMap::from([(5u8, 8usize)]);
        let html = render(&data(&lignes, &pool, &jj, &runs_test()));
        let c = corps(&html);
        assert!(c.contains("comptes planifiés"));
        assert!(c.contains("sur <b>8</b> éligibles"), "l'échelle du pool : {c}");
        assert!(c.contains("fin de montée en charge"));
        assert!(c.contains("pic de charge"));
        assert!(c.contains("plateformes couvertes"));
    }

    #[test]
    fn le_rapport_contient_les_deux_graphes() {
        let lignes = vec![ligne("CF1", "Cegedim", "2026-08-01", Origine::Auto)];
        let pool = BTreeMap::from([("Cegedim".to_string(), 8usize)]);
        let jj = BTreeMap::from([(5u8, 8usize)]);
        let html = render(&data(&lignes, &pool, &jj, &runs_test()));
        assert_eq!(corps(&html).matches("<svg").count(), 2, "aire cumulée + barres");
        assert!(corps(&html).contains("Parc facturant"));
        assert!(corps(&html).contains("Charge par run"));
    }

    #[test]
    fn le_rapport_nemet_plus_de_classes_orphelines() {
        // Régression du constat d'ouverture : ces classes n'existaient dans aucune
        // feuille de style, le rapport s'affichait en HTML brut.
        let lignes = vec![ligne("CF1", "Cegedim", "2026-08-01", Origine::Auto)];
        let pool = BTreeMap::from([("Cegedim".to_string(), 8usize)]);
        let jj = BTreeMap::from([(5u8, 8usize)]);
        let html = render(&data(&lignes, &pool, &jj, &runs_test()));
        let c = corps(&html);
        for orpheline in ["class=\"cards\"", "class=\"card\"", "class=\"big\"", "class=\"lbl\""] {
            assert!(!c.contains(orpheline), "classe sans style : {orpheline}");
        }
        assert!(c.contains("class=\"kpis\""), "les cartes du rapport de run");
        assert!(c.contains("class=\"tbl\""), "la table doit être encadrée");
    }

    #[test]
    fn la_table_marque_le_debut_de_mep_et_le_gel() {
        let mut l2 = ligne("CF2", "Cegedim", "2026-10-01", Origine::Auto);
        l2.mep_id = 2;
        l2.run_num = "R2".into();
        // MEP 1 au 01/07, soit avant le 25/07 que `data` donne pour aujourd'hui :
        // elle est donc gelée, et c'est ce badge que le test cherche.
        let lignes = vec![ligne("CF1", "Cegedim", "2026-07-01", Origine::Auto), l2];
        let pool = BTreeMap::from([("Cegedim".to_string(), 8usize)]);
        let jj = BTreeMap::from([(5u8, 8usize)]);
        let html = render(&data(&lignes, &pool, &jj, &runs_test()));
        let c = corps(&html);
        assert!(c.contains("class=\"mep-start\""), "chaque MEP ouvre un groupe");
        assert!(c.contains("class=\"frozen\">gelée"), "la MEP passée est signalée : {c}");
        assert!(c.contains("<tbody>"), "table structurée, pas un empilement de <tr>");
        // Dates en jj/mm/aaaa comme les indicateurs et la maquette, non en ISO :
        // deux formats dans un même rapport se lisent comme deux sources.
        assert!(c.contains("<td>01/07/2026</td>"), "date de MEP en clair : {c}");
        assert!(!c.contains("2026-07-01"), "l'ISO du stockage ne doit pas fuir");
    }

    #[test]
    fn lecart_de_repartition_est_signe_et_en_francais() {
        assert_eq!(fmt_ecart(4.4), "+4,4 pt");
        assert_eq!(fmt_ecart(-1.6), "−1,6 pt");
        assert_eq!(part(0, 0), 0.0, "aucune largeur de barre ne doit valoir NaN");
    }
}
