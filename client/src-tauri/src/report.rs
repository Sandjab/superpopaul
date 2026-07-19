//! Rapport HTML autonome de fin de run — livrable client : agrégats
//! uniquement (jamais de liste d'adressages), identité « Bleu nuit & or »,
//! zéro JavaScript (anneau et barres en SVG/CSS statiques), variante fond
//! clair (impression + écran selon le thème du lecteur). Toute valeur
//! injectée (nom de fichier, nom de PA)
//! est échappée : un nom de PA vient des SMP, c'est une entrée non fiable.

use crate::telemetry::Snapshot;

pub struct ReportData<'a> {
    /// Nom seul du fichier d'entrée (jamais le chemin : il peut révéler
    /// l'arborescence du poste dans un document destiné à un client).
    pub file_name: &'a str,
    /// « 16 juillet 2026 » — en-tête.
    pub date_longue: &'a str,
    /// « 16/07/2026 18:42 » — pied de page.
    pub date_heure: &'a str,
    /// Date du jour : borne « aujourd'hui » des paliers de projection
    /// (ctc::paliers) — injectée pour des rapports testables.
    pub today: chrono::NaiveDate,
    pub version: &'a str,
    pub snapshot: &'a Snapshot,
    /// Pluriel du libellé « ce que représente une ligne » (config
    /// `record_label`) — remplace « lignes » dans le rapport.
    pub record_plural: &'a str,
    /// Couverture déclarative des annuaires (Peppol + PPF). Section masquée si
    /// aucun annuaire n'est chargé (`peppol` et `ppf` tous deux `None`).
    pub coverage: &'a crate::coverage::Coverage,
}

/// CSS du rapport — la maquette validée le 16/07/2026, identité « Bleu nuit
/// & or » (tokens de styles.css), avec variantes fond clair (impression +
/// écran selon le thème du lecteur).
const CSS: &str = r#"
  :root {
    --bg: #0e1524; --card: #172136; --border: #2b3752;
    --fg: #eae9e2; --muted: #939cb4;
    --green: #4cc268; --gold: #d9a83f; --amber: #e0873a; --red: #e5534b;
    --track: #223050;
    --green-later: #2f8050; /* prêt plus tard : vert éteint, pris sur le vert */
    --ppf-l1: #6f6aa8; --ppf-l2: #8a80d4; --ppf-l3: #a892ff; --ppf-l4: #c3b6ff;
  }
  * { box-sizing: border-box; }
  body { margin: 0; background: var(--bg); color: var(--fg);
    font: 15px/1.55 -apple-system, "Segoe UI", system-ui, sans-serif; }
  .page { max-width: 820px; margin: 0 auto; padding: 40px 28px 24px; }
  header { border-bottom: 2px solid var(--gold); padding-bottom: 18px; margin-bottom: 28px; }
  .wordmark { color: var(--gold); font-size: 12px; font-weight: 700; letter-spacing: .18em; }
  h1 { margin: 6px 0 4px; font-size: 26px; font-weight: 600; }
  .meta { color: var(--muted); font-size: 14px; }
  .meta b { color: var(--fg); font-weight: 600; }
  .kpis { display: flex; gap: 14px; flex-wrap: wrap; margin-bottom: 28px; }
  .kpi { flex: 1 1 150px; background: var(--card); border: 1px solid var(--border);
    border-radius: 10px; padding: 14px 16px; }
  .kpi .v { font-size: 27px; font-weight: 700; line-height: 1.15; }
  .kpi .l { color: var(--muted); font-size: 12.5px; margin-top: 3px; }
  .kpi .d { color: var(--muted); font-size: 12.5px; }
  .kpi.gold .v { color: var(--gold); }
  .kpi.green .v { color: var(--green); }
  .kpi.amber .v { color: var(--amber); }
  .kpi.red .v { color: var(--red); }
  .kpi .proj { color: var(--muted); font-size: 12.5px; margin-top: 6px;
    padding-top: 6px; border-top: 1px solid var(--border); }
  .kpi .proj b { color: var(--fg); font-weight: 600; }
  h2 { font-size: 16px; font-weight: 600; margin: 30px 0 14px; }
  h2::after { content: ""; display: block; width: 44px; border-bottom: 2px solid var(--gold); margin-top: 5px; }
  .ring-row { display: flex; gap: 30px; align-items: center; flex-wrap: wrap;
    background: var(--card); border: 1px solid var(--border); border-radius: 10px; padding: 20px; }
  .ring-center { font-size: 30px; font-weight: 700; fill: var(--green); }
  .ring-sub { font-size: 11px; fill: #939cb4; }
  .legend { flex: 1; min-width: 300px; font-size: 14px;
    display: grid; grid-template-columns: auto 1fr max-content max-content;
    gap: 4px 14px; align-items: center; }
  .legend .h { color: var(--muted); font-size: 11px; text-transform: uppercase;
    letter-spacing: .06em; text-align: right; }
  .dot { width: 10px; height: 10px; border-radius: 5px; }
  .legend .n { text-align: right; font-variant-numeric: tabular-nums; color: var(--muted); white-space: nowrap; }
  .legend .n b { color: var(--fg); }
  .proj-card { background: var(--card); border: 1px solid var(--border); border-radius: 10px;
    padding: 16px 20px; font-variant-numeric: tabular-nums; }
  .proj-now { font-size: 15px; font-weight: 600; }
  .proj-now b { color: var(--green); font-size: 17px; }
  .proj-row { display: flex; gap: 10px; align-items: baseline; font-size: 14px; padding: 3px 0 0 14px; }
  .proj-row b { color: var(--fg); }
  .proj-row .plus { color: var(--green-later); font-weight: 700; min-width: 7.5ch; }
  .proj-row .det { color: var(--muted); font-size: 12.5px; }
  .proj-foot { color: var(--muted); font-size: 11.5px; margin: 10px 0 0; }
  .pa { background: var(--card); border: 1px solid var(--border); border-radius: 10px; padding: 18px 20px; }
  .pa-row { display: grid; grid-template-columns: 170px 1fr 90px; gap: 12px; align-items: center; padding: 5px 0; font-size: 14px; }
  .pa-name { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .bar { height: 10px; border-radius: 5px; background: var(--track); overflow: hidden; }
  .bar i { display: block; height: 100%; background: var(--gold); border-radius: 5px; }
  .pa-n { text-align: right; font-variant-numeric: tabular-nums; color: var(--muted); }
  .pa-n b { color: var(--fg); }
  footer { margin-top: 36px; padding-top: 14px; border-top: 1px solid var(--border);
    color: var(--muted); font-size: 12px; display: flex; justify-content: space-between; flex-wrap: wrap; gap: 6px; }
  /* Fond clair à l'écran quand le lecteur a un thème clair (OS/navigateur) —
     même palette que l'impression, toujours sans JS. */
  @media (prefers-color-scheme: light) {
    :root { --bg: #ffffff; --card: #f6f5f1; --border: #d8d5cc; --fg: #1c2333; --muted: #5c6478; --track: #e4e1d8; --ppf-l1: #8b86c4; --ppf-l2: #7a6fd0; --ppf-l3: #6a58c8; --ppf-l4: #5741b0; }
    .ring-sub { fill: #5c6478; }
  }
  @media print {
    :root { --bg: #ffffff; --card: #f6f5f1; --border: #d8d5cc; --fg: #1c2333; --muted: #5c6478; --track: #e4e1d8; --ppf-l1: #8b86c4; --ppf-l2: #7a6fd0; --ppf-l3: #6a58c8; --ppf-l4: #5741b0; }
    .page { padding: 0; }
    .ring-sub { fill: #5c6478; }
  }
  .cov-elig { color: var(--muted); font-size: 13px; margin: 0 0 16px; }
  .cov-elig b { color: var(--fg); }
  .cov-row { display: grid; grid-template-columns: 200px 1fr 128px; gap: 12px;
    align-items: center; padding: 5px 0; font-size: 14px; }
  .cov-name { display: flex; align-items: center; gap: 8px; min-width: 0; }
  .cov-name .tag { color: var(--muted); font-size: 11.5px; }
  .cov-sw { width: 9px; height: 9px; border-radius: 3px; flex: none; }
  .cov-sub .cov-name { position: relative; padding-left: 20px; color: var(--muted); }
  .cov-sub .cov-name::before { content: "├"; position: absolute; left: 4px; color: var(--border); }
  .cov-sub.last .cov-name::before { content: "└"; }
  .cov-n { text-align: right; font-variant-numeric: tabular-nums; color: var(--muted); white-space: nowrap; }
  .cov-n b { color: var(--fg); }
  .cov-sub.last .cov-n b { color: var(--ppf-l4); }
  .cov-gh { color: var(--muted); font-size: 12px; text-transform: uppercase;
    letter-spacing: .06em; margin: 16px 0 4px; }
"#;

pub fn render(d: &ReportData) -> String {
    let s = d.snapshot;
    let file = esc(d.file_name);
    let mut html = String::with_capacity(16 * 1024);
    html.push_str("<!doctype html>\n<html lang=\"fr\">\n<head>\n<meta charset=\"utf-8\">\n");
    html.push_str(&format!(
        "<title>Rapport de résolution Peppol — {file}</title>\n<style>{CSS}</style>\n</head>\n<body>\n<div class=\"page\">\n"
    ));

    // En-tête.
    html.push_str(&format!(
        "<header>\n<div class=\"wordmark\">SUPER POPAUL</div>\n\
         <h1>Rapport de résolution Peppol</h1>\n\
         <p class=\"meta\">Fichier <b>{file}</b> · analysé le <b>{}</b> ·\n\
         <b>{}</b> adressages uniques (<b>{}</b> {})</p>\n</header>\n",
        esc(d.date_longue),
        fmt_int(s.done),
        fmt_int(s.done_lines),
        d.record_plural,
    ));

    // Paliers de projection (3 max, horizon 2 ans) : partagés entre la tuile
    // verte (prochain palier) et la carte Projection.
    let pal = crate::ctc::paliers(&s.ctc_later_dates, d.today);

    // Tuiles KPI — verdict inconnu et non résolus seulement si présents.
    // Double lecture : le grand % est en adressages, le détail donne aussi
    // l'équivalent en lignes de fichier.
    html.push_str("<div class=\"kpis\">\n");
    kpi(&mut html, "gold", s.exists, s.exists_lines, s, "Provisionnés Réseau Peppol", None, d.record_plural);
    kpi(
        &mut html,
        "green",
        s.ctc,
        s.ctc_lines,
        s,
        "France Invoice UBL Extension — prêts aujourd'hui",
        pal.first().map(|p| {
            format!("<b>+{}</b> au {}", fmt_pct(p.addr, s.done), fmt_date_fr(&p.date))
        }),
        d.record_plural,
    );
    if s.no_verdict > 0 {
        kpi(&mut html, "amber", s.no_verdict, s.no_verdict_lines, s, "Verdict inconnu (catalogue illisible)", None, d.record_plural);
    }
    if s.failed > 0 {
        kpi(&mut html, "red", s.failed, s.failed_lines, s, "Non résolus", None, d.record_plural);
    }
    html.push_str("</div>\n");

    // Anneau + légende.
    html.push_str("<h2>Répartition des adressages</h2>\n<div class=\"ring-row\">\n");
    html.push_str(
        "<svg width=\"210\" height=\"210\" viewBox=\"0 0 210 210\" role=\"img\" aria-label=\"Répartition\">\n\
         <g transform=\"rotate(-90 105 105)\">\n\
         <circle cx=\"105\" cy=\"105\" r=\"80\" fill=\"none\" stroke=\"var(--track)\" stroke-width=\"26\"/>\n",
    );
    for (color, len, offset) in ring_segments(s) {
        html.push_str(&format!(
            "<circle cx=\"105\" cy=\"105\" r=\"80\" fill=\"none\" stroke=\"var(--{color})\" stroke-width=\"26\" \
             stroke-dasharray=\"{len:.1} {RING_C:.2}\" stroke-dashoffset=\"{offset:.1}\"/>\n"
        ));
    }
    html.push_str(&format!(
        "</g>\n<text x=\"105\" y=\"102\" text-anchor=\"middle\" class=\"ring-center\">{}</text>\n\
         <text x=\"105\" y=\"122\" text-anchor=\"middle\" class=\"ring-sub\">France Invoice UBL Ext.</text>\n\
         <text x=\"105\" y=\"135\" text-anchor=\"middle\" class=\"ring-sub\">prêts aujourd'hui</text>\n</svg>\n",
        fmt_pct(s.ctc, s.done)
    ));
    html.push_str(&format!(
        "<div class=\"legend\">\n<span></span><span></span>\
         <span class=\"h\">adressages</span><span class=\"h\">{}</span>\n",
        d.record_plural,
    ));
    for (color, label, addr, lignes) in legend_rows(s) {
        html.push_str(&format!(
            "<span class=\"dot\" style=\"background:var(--{color})\"></span><span>{label}</span>\
             <span class=\"n\"><b>{}</b> · {}</span><span class=\"n\"><b>{}</b> · {}</span>\n",
            fmt_int(addr),
            fmt_pct(addr, s.done),
            fmt_int(lignes),
            fmt_pct(lignes, s.done_lines)
        ));
    }
    html.push_str("</div>\n</div>\n");

    // Carte Projection : paliers cumulés — chaque ligne est une vérité
    // exacte (« au JJ/MM, +x % seront prêts »), les dates omises sont
    // absorbées par le palier suivant. Absente s'il n'y a rien à projeter
    // sous l'horizon.
    if !pal.is_empty() {
        html.push_str(&format!(
            "<h2>Projection</h2>\n<div class=\"proj-card\">\n\
             <div class=\"proj-now\">Prêts aujourd'hui : <b>{}</b></div>\n",
            fmt_pct(s.ctc, s.done)
        ));
        let mut prev = 0u64;
        for p in &pal {
            let delta = p.addr - prev;
            let mut det = if prev == 0 {
                format!("{} adressages", fmt_int(delta))
            } else {
                format!("cumul, {} adressages de plus", fmt_int(delta))
            };
            if p.date == crate::ctc::ECHEANCE_REFORME {
                det.push_str(" — échéance réglementaire");
            }
            html.push_str(&format!(
                "<div class=\"proj-row\"><span class=\"plus\">+{}</span>\
                 <span>au <b>{}</b></span><span class=\"det\">{det}</span></div>\n",
                fmt_pct(p.addr, s.done),
                fmt_date_fr(&p.date)
            ));
            prev = p.addr;
        }
        html.push_str(
            "<p class=\"proj-foot\">Paliers cumulés d'après les dates d'activation \
             annoncées dans Peppol · horizon 2 ans · l'échéance du 01/09/2026 reste \
             affichée tant qu'elle est à venir.</p>\n</div>\n",
        );
    }

    // Plateformes constatées — top 5, le reste agrégé.
    let (shown, autres) = pa_rows(s);
    if !shown.is_empty() || autres.is_some() {
        html.push_str("<h2>Plateformes de dématérialisation constatées</h2>\n<div class=\"pa\">\n");
        let max = shown
            .iter()
            .map(|(_, c)| *c)
            .chain(autres.iter().map(|(_, c)| *c))
            .max()
            .unwrap_or(1)
            .max(1);
        for (name, count) in shown.iter().cloned().chain(autres.clone()) {
            html.push_str(&format!(
                "<div class=\"pa-row\"><span class=\"pa-name\">{}</span>\
                 <span class=\"bar\"><i style=\"width:{:.0}%\"></i></span>\
                 <span class=\"pa-n\"><b>{}</b> · {}</span></div>\n",
                esc(&name),
                count as f64 * 100.0 / max as f64,
                fmt_int(count),
                fmt_pct(count, s.exists)
            ));
        }
        html.push_str("</div>\n");
    }

    // Présence déclarative en annuaire (Peppol + PPF) — après le réseau, dont
    // elle est explicitement distincte.
    coverage_section(&mut html, d.coverage, d.record_plural);

    // Pied de page.
    html.push_str(&format!(
        "<footer>\n<span>Généré par Super Popaul v{} · {}</span>\n\
         <span>Données du réseau Peppol (SML/SMP) au moment de l'analyse</span>\n</footer>\n",
        esc(d.version),
        esc(d.date_heure)
    ));
    html.push_str("</div>\n</body>\n</html>\n");
    html
}

// Section « Présence déclarative en annuaire » : barres (pas anneaux) pour la
// distinguer du réseau. Rendue seulement si au moins un annuaire est chargé.
// Dénominateur = éligibles 0225 ; « lignes » remplacé par le libellé record.
fn coverage_section(html: &mut String, c: &crate::coverage::Coverage, record_plural: &str) {
    if c.peppol.is_none() && c.ppf.is_none() {
        return;
    }
    let denom = c.eligible_0225;
    let pct = |n: usize| -> String {
        match (n * 100 + denom / 2).checked_div(denom) {
            Some(v) => format!("{v}\u{202F}%"),
            None => "—".to_string(),
        }
    };
    let width = |n: usize| -> f64 {
        if denom == 0 { 0.0 } else { n as f64 * 100.0 / denom as f64 }
    };
    let row = |html: &mut String, sub: bool, last: bool, color: &str, name: &str, tag: &str, n: usize, bold_name: bool| {
        let cls = if sub { if last { "cov-row cov-sub last" } else { "cov-row cov-sub" } } else { "cov-row" };
        let name_html = if bold_name { format!("<b>{name}</b>") } else { name.to_string() };
        let tag_html = if tag.is_empty() { String::new() } else { format!(" <span class=\"tag\">{tag}</span>") };
        html.push_str(&format!(
            "<div class=\"{cls}\"><span class=\"cov-name\">\
             <span class=\"cov-sw\" style=\"background:var(--{color})\"></span>{name_html}{tag_html}</span>\
             <span class=\"bar\"><i style=\"width:{:.0}%;background:var(--{color})\"></i></span>\
             <span class=\"cov-n\"><b>{}</b> / {} · {}</span></div>\n",
            width(n), fmt_int(n as u64), fmt_int(denom as u64), pct(n)
        ));
    };

    html.push_str("<h2>Présence déclarative en annuaire</h2>\n<div class=\"pa\">\n");
    html.push_str(&format!(
        "<p class=\"cov-elig\"><b>{}</b> éligibles 0225 / <b>{}</b> {} · <b>{}</b> non applicables — \
         présence déclarée dans les annuaires chargés, distincte du « Provisionnés Réseau Peppol » ci-dessus.</p>\n",
        fmt_int(denom as u64),
        fmt_int(c.total_lines as u64),
        record_plural,
        fmt_int(c.non_applicable as u64),
    ));
    if let Some(p) = c.peppol {
        row(html, false, false, "green", "Annuaire Peppol", "", p.present, false);
    }
    if let Some(p) = c.ppf {
        html.push_str("<div class=\"cov-gh\">Annuaire PPF — présent → utilisable</div>\n");
        row(html, false, false, "ppf-l1", "Annuaire PPF", "", p.present, true);
        row(html, true, false, "ppf-l2", "PPF actif", "motif C/P", p.active, false);
        row(html, true, false, "ppf-l3", "PDP définie", "réelle", p.pdp_definie, false);
        row(html, true, true, "ppf-l4", "PPF utilisable", "actif + PDP réelle", p.usable, false);
    }
    html.push_str("</div>\n");
}

// Chaque paramètre est une donnée distincte de la tuile (cible, couleur, deux
// comptes, snapshot, libellé, projection, libellé pluriel) — pas un défaut de
// conception ; on tolère la liste longue comme `output::generate`.
#[allow(clippy::too_many_arguments)]
fn kpi(
    html: &mut String,
    color: &str,
    addr: u64,
    lignes: u64,
    s: &Snapshot,
    label: &str,
    proj: Option<String>,
    record_plural: &str,
) {
    // `proj` : ligne de projection sous filet (tuile verte : prochain palier).
    // HTML déjà formé côté appelant — uniquement à partir de valeurs internes.
    let proj = proj
        .map(|p| format!("<div class=\"proj\">{p}</div>"))
        .unwrap_or_default();
    html.push_str(&format!(
        "<div class=\"kpi {color}\"><div class=\"v\">{}</div><div class=\"l\">{label}</div>\
         <div class=\"d\">{} adressages</div><div class=\"d\">{} {} ({})</div>{proj}</div>\n",
        fmt_pct(addr, s.done),
        fmt_int(addr),
        fmt_int(lignes),
        record_plural,
        fmt_pct(lignes, s.done_lines)
    ));
}

/// « 2026-09-01 » → « 01/09/2026 » (clé ISO produite par ctc::activation_day).
fn fmt_date_fr(iso: &str) -> String {
    let mut it = iso.splitn(3, '-');
    match (it.next(), it.next(), it.next()) {
        (Some(y), Some(m), Some(j)) => format!("{j}/{m}/{y}"),
        _ => iso.to_string(),
    }
}

/// Lignes de la légende, dans l'ordre de l'anneau, segments vides omis
/// (au sens adressages : les lignes suivent).
fn legend_rows(s: &Snapshot) -> Vec<(&'static str, &'static str, u64, u64)> {
    ring_parts(s)
        .into_iter()
        .filter(|(_, _, n, _)| *n > 0)
        .collect()
}

/// Une ligne du tableau des PA : libellé et nombre d'adressages.
type PaRow = (String, u64);

/// Top 5 des PA + agrégat « Autres (n plateformes) » au-delà de 6 (à 6
/// exactement, tout est affiché : un agrégat d'une seule PA n'aide pas).
fn pa_rows(s: &Snapshot) -> (Vec<PaRow>, Option<PaRow>) {
    let pa = &s.pa;
    if pa.len() > 6 {
        let rest: u64 = pa[5..].iter().map(|p| p.count).sum();
        (
            pa[..5].iter().map(|p| (p.name.clone(), p.count)).collect(),
            Some((format!("Autres ({} plateformes)", pa.len() - 5), rest)),
        )
    } else {
        (pa.iter().map(|p| (p.name.clone(), p.count)).collect(), None)
    }
}

/// « 16 juillet 2026 » — chrono n'embarque pas de locale.
pub fn date_fr_longue(d: &chrono::DateTime<chrono::Local>) -> String {
    use chrono::Datelike;
    const MOIS: [&str; 12] = [
        "janvier", "février", "mars", "avril", "mai", "juin", "juillet",
        "août", "septembre", "octobre", "novembre", "décembre",
    ];
    format!("{} {} {}", d.day(), MOIS[d.month0() as usize], d.year())
}

/// Échappement HTML minimal — toute valeur dynamique passe par là.
fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// Milliers séparés par une espace fine insécable (typographie française).
fn fmt_int(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push('\u{202F}');
        }
        out.push(c);
    }
    out
}

/// « 38,1 % » — virgule décimale, espace fine insécable avant le %.
/// Total nul (run vide) : 0,0 %.
fn fmt_pct(part: u64, total: u64) -> String {
    let p = if total == 0 {
        0.0
    } else {
        part as f64 * 100.0 / total as f64
    };
    format!("{p:.1}\u{202F}%").replace('.', ",")
}

/// Les six familles de l'anneau, dans l'ordre : (couleur, libellé, compte
/// en adressages, compte en lignes de fichier). Les comptes dérivés sont en
/// soustraction saturante : un snapshot incohérent ne doit jamais faire
/// paniquer un rapport.
fn ring_parts(s: &Snapshot) -> [(&'static str, &'static str, u64, u64); 6] {
    // Le gap « sans l'extension » est net des prêts plus tard (comptés à
    // part) — il inclut les supports expirés (dégradation simple).
    let sans_ext = s
        .exists
        .saturating_sub(s.ctc)
        .saturating_sub(s.ctc_later)
        .saturating_sub(s.no_verdict);
    let sans_ext_l = s
        .exists_lines
        .saturating_sub(s.ctc_lines)
        .saturating_sub(s.ctc_later_lines)
        .saturating_sub(s.no_verdict_lines);
    let absents = s.done.saturating_sub(s.exists).saturating_sub(s.failed);
    let absents_l = s
        .done_lines
        .saturating_sub(s.exists_lines)
        .saturating_sub(s.failed_lines);
    [
        ("green", "Prêts aujourd'hui (France Invoice UBL Extension)", s.ctc, s.ctc_lines),
        ("green-later", "Prêts plus tard (activation à venir)", s.ctc_later, s.ctc_later_lines),
        ("gold", "Provisionnés, sans l'extension", sans_ext, sans_ext_l),
        ("amber", "Provisionnés, verdict inconnu", s.no_verdict, s.no_verdict_lines),
        ("track", "Absents Réseau Peppol", absents, absents_l),
        ("red", "Non résolus", s.failed, s.failed_lines),
    ]
}

/// Segments de l'anneau : (classe de couleur, longueur, offset) sur la
/// circonférence `RING_C`, dans l'ordre de `ring_parts`. Segments vides omis.
const RING_C: f64 = 502.6548; // 2π × r=80
fn ring_segments(s: &Snapshot) -> Vec<(&'static str, f64, f64)> {
    let parts = ring_parts(s);
    let total: u64 = parts.iter().map(|(_, _, n, _)| n).sum();
    if total == 0 {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut cum = 0.0;
    for (color, _, n, _) in parts {
        if n == 0 {
            continue;
        }
        let len = n as f64 / total as f64 * RING_C;
        out.push((color, len, -cum));
        cum += len;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coverage::{Coverage, PeppolCoverage, PpfCoverage};
    use crate::telemetry::NamedCount;
    use std::collections::BTreeMap;

    // Couverture « vide » (aucun annuaire chargé) : référence 'static, permet de
    // garder `data()` inchangé et laisse la section absente du rendu.
    static EMPTY_COV: Coverage = Coverage {
        total_lines: 0,
        eligible_0225: 0,
        non_applicable: 0,
        peppol: None,
        ppf: None,
    };

    fn cov_full() -> Coverage {
        Coverage {
            total_lines: 1000,
            eligible_0225: 900,
            non_applicable: 100,
            peppol: Some(PeppolCoverage { present: 812 }),
            ppf: Some(PpfCoverage { present: 640, active: 590, pdp_definie: 610, usable: 570 }),
        }
    }

    fn data_with<'a>(s: &'a Snapshot, cov: &'a Coverage) -> ReportData<'a> {
        ReportData { coverage: cov, ..data(s) }
    }

    fn named(pairs: &[(&str, u64)]) -> Vec<NamedCount> {
        pairs
            .iter()
            .map(|(n, c)| NamedCount { name: n.to_string(), count: *c })
            .collect()
    }

    fn snap() -> Snapshot {
        Snapshot {
            done: 11_942,
            total: 11_942,
            exists: 7_452,
            ctc: 4_550,
            ctc_later: 0,
            ctc_later_lines: 0,
            ctc_later_dates: Vec::new(),
            no_verdict: 609,
            failed: 143,
            done_lines: 12_480,
            exists_lines: 7_800,
            ctc_lines: 4_700,
            no_verdict_lines: 620,
            failed_lines: 150,
            http: BTreeMap::new(),
            pa: named(&[
                ("Docaposte", 2_310),
                ("Esalink", 1_480),
                ("Generix", 990),
                ("Pennylane", 640),
                ("B2Brouter", 410),
                ("Qweeby", 300),
                ("Tradeshift", 200),
                ("Unimaze", 122),
            ]),
            errors: Vec::new(),
            latency: None,
            latency_hist: Vec::new(),
            concurrency_allowed: 0,
            concurrency_max: 0,
            req_per_s: 0.0,
            addr_per_s: 0.0,
            eta_s: None,
            active_s: 120.0,
            halted: false,
        }
    }

    fn data(s: &Snapshot) -> ReportData<'_> {
        ReportData {
            file_name: "clients_2026.csv",
            date_longue: "16 juillet 2026",
            date_heure: "16/07/2026 18:42",
            today: "2026-07-16".parse().unwrap(),
            version: "0.3.4",
            snapshot: s,
            record_plural: "lignes",
            coverage: &EMPTY_COV,
        }
    }

    #[test]
    fn rapport_utilise_le_libelle_record_partout() {
        // Le libellé configuré (record_label.plural()) remplace « lignes » dans
        // les trois zones du rapport : en-tête, en-tête de légende, tuiles KPI.
        let s = snap();
        let mut d = data(&s);
        d.record_plural = "abonnés";
        let html = render(&d);
        assert!(
            html.matches("abonnés").count() >= 3,
            "le libellé doit remplacer « lignes » (en-tête + légende + ≥1 KPI)"
        );
        assert!(
            !html.contains("lignes"),
            "aucun « lignes » codé en dur ne doit subsister dans le rendu"
        );
    }

    #[test]
    fn libelles_provisionnes_reseau() {
        // Le rapport parle de « Provisionnés » (constaté sur le réseau Peppol),
        // plus de « Inscrits » : distinction avec la présence déclarative dans
        // l'annuaire. snap() a du verdict inconnu et des absents, donc les deux
        // lignes de légende correspondantes sont présentes.
        let html = render(&data(&snap()));
        assert!(html.contains("Provisionnés Réseau Peppol"), "KPI réseau");
        assert!(html.contains("Provisionnés, sans l'extension"), "légende sans extension");
        assert!(html.contains("Provisionnés, verdict inconnu"), "légende verdict inconnu");
        assert!(html.contains("Absents Réseau Peppol"), "légende absents");
        assert!(!html.contains("Inscrits"), "plus aucun « Inscrits » codé en dur");
        assert!(!html.contains("Absents de Peppol"), "ancien libellé absents");
    }

    #[test]
    fn rapport_s_adapte_au_theme_clair_du_lecteur() {
        // Le rapport bascule en fond clair si l'OS/navigateur du lecteur est en
        // thème clair (@media prefers-color-scheme) — sans le moindre JS.
        let html = render(&data(&snap()));
        assert!(
            html.contains("prefers-color-scheme: light"),
            "variante écran clair absente du CSS"
        );
        assert!(!html.contains("<script"), "le rapport doit rester sans JS");
    }

    /// snap() + 535 « prêts plus tard » (498 au 01/09, 37 au 22/09 — le cas
    /// réel de la base du 16/07/2026), retranchés du vert.
    fn snap_avec_futurs() -> Snapshot {
        let mut s = snap();
        s.ctc -= 535;
        s.ctc_lines -= 552;
        s.ctc_later = 535;
        s.ctc_later_lines = 552;
        s.ctc_later_dates = vec![
            crate::ctc::DateCount { date: "2026-09-01".into(), addr: 498, lines: 514 },
            crate::ctc::DateCount { date: "2026-09-22".into(), addr: 37, lines: 38 },
        ];
        s
    }

    #[test]
    fn rapport_nominal_kpis_meta_et_pied() {
        let s = snap();
        let html = render(&data(&s));
        // Méta : fichier, date, volumes (séparateur de milliers insécable fin).
        assert!(html.contains("clients_2026.csv"), "nom du fichier absent");
        assert!(html.contains("16 juillet 2026"));
        assert!(html.contains("11\u{202F}942"), "volume adressages non formaté");
        assert!(html.contains("12\u{202F}480"), "volume lignes non formaté");
        // KPIs : pourcentages base adressages, virgule décimale.
        assert!(html.contains("62,4\u{202F}%"), "KPI inscrits absent");
        assert!(html.contains("38,1\u{202F}%"), "KPI extension absent");
        // Double lecture : équivalents lignes dans les tuiles…
        assert!(html.contains("7\u{202F}800 lignes (62,5\u{202F}%)"), "tuile inscrits sans lignes");
        assert!(html.contains("4\u{202F}700 lignes (37,7\u{202F}%)"), "tuile extension sans lignes");
        // … et légende en deux colonnes chiffrées avec en-têtes.
        assert!(html.contains(">adressages<"), "en-tête de colonne adressages");
        assert!(html.contains(">lignes<"), "en-tête de colonne lignes");
        // Absents en lignes : 12 480 − 7 800 − 150 = 4 530.
        assert!(html.contains("4\u{202F}530"), "absents de Peppol en lignes");
        // Pied de page.
        assert!(html.contains("Super Popaul v0.3.4"));
        assert!(html.contains("16/07/2026 18:42"));
        // Autonome et sans script.
        assert!(html.starts_with("<!doctype html>"));
        assert!(!html.contains("<script"), "le rapport ne doit porter aucun JS");
    }

    #[test]
    fn verdict_temporel_tuile_legende_et_projection() {
        let s = snap_avec_futurs();
        let html = render(&data(&s));
        // Tuile verte : grand % = prêts aujourd'hui (4 015/11 942 = 33,6 %),
        // projection (prochain palier) sous le filet.
        assert!(html.contains("prêts aujourd'hui"), "libellé tuile/centre");
        assert!(html.contains("33,6\u{202F}%"), "grand % recalculé sans les futurs");
        assert!(
            html.contains("+4,2\u{202F}%</b> au 01/09/2026"),
            "projection de la tuile"
        );
        // Anneau : segment vert éteint entre le vert et l'or.
        assert!(
            html.contains("stroke=\"var(--green-later)\""),
            "segment prêts plus tard"
        );
        // Légende : ligne dédiée, double lecture.
        assert!(html.contains("Prêts plus tard"), "ligne de légende");
        assert!(html.contains(">535<"), "adressages plus tard");
        assert!(html.contains(">552<"), "lignes plus tard");
        // Gap « sans l'extension » : exists − ctc − later − nv
        // = 7 452 − 4 015 − 535 − 609 = 2 293.
        assert!(html.contains("2\u{202F}293"), "gap net des prêts plus tard");
        // Carte Projection : paliers cumulés (+4,2 % puis +4,5 %), détails.
        assert!(html.contains("Projection"), "carte projection");
        assert!(html.contains("+4,5\u{202F}%"), "palier cumulé");
        assert!(html.contains("<b>22/09/2026</b>"), "date du palier cumulé");
        assert!(html.contains("498 adressages"), "détail du premier palier");
        assert!(html.contains("échéance réglementaire"), "mention échéance");
        assert!(html.contains("cumul, 37 adressages de plus"), "détail cumulé");
        assert!(html.contains("horizon 2 ans"), "pied de carte");
    }

    #[test]
    fn sans_activation_future_ni_segment_ni_projection() {
        // snap() : ctc_later = 0 — rien de nouveau dans le rapport.
        let html = render(&data(&snap()));
        assert!(!html.contains("Prêts plus tard"));
        assert!(!html.contains("Projection"));
        assert!(!html.contains("stroke=\"var(--green-later)\""));
    }

    #[test]
    fn projection_bornee_par_l_horizon_meme_avec_des_futurs() {
        // Un seul futur, à 10 ans (bruit déclaratif) : le segment et la
        // légende comptent l'adressage, mais aucune carte Projection.
        let mut s = snap_avec_futurs();
        s.ctc_later_dates =
            vec![crate::ctc::DateCount { date: "2036-05-18".into(), addr: 535, lines: 552 }];
        let html = render(&data(&s));
        assert!(html.contains("Prêts plus tard"), "toujours comptés");
        assert!(!html.contains("Projection"), "rien à projeter sous 2 ans");
    }

    #[test]
    fn nom_de_pa_hostile_echappe() {
        let mut s = snap();
        s.pa = named(&[("<script>alert(1)</script>", 100)]);
        let html = render(&data(&s));
        assert!(!html.contains("<script"), "PA non échappée : XSS possible");
        assert!(html.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
    }

    #[test]
    fn blocs_conditionnels_absents_sur_run_sain() {
        let mut s = snap();
        s.no_verdict = 0;
        s.failed = 0;
        let html = render(&data(&s));
        assert!(!html.contains("Verdict inconnu"), "tuile inutile sur run sain");
        assert!(!html.contains("Non résolus"), "tuile inutile sur run sain");
        // Et présents quand le cas existe.
        let html = render(&data(&snap()));
        assert!(html.contains("Verdict inconnu"));
        assert!(html.contains("Non résolus"));
    }

    #[test]
    fn pa_top5_et_autres_agreges() {
        let s = snap(); // 8 PA
        let html = render(&data(&s));
        assert!(html.contains("Docaposte"));
        assert!(html.contains("B2Brouter"));
        assert!(!html.contains("Qweeby"), "au-delà du top 5 : agrégé");
        // Autres = Qweeby + Tradeshift + Unimaze = 622.
        assert!(html.contains("Autres (3 plateformes)"), "agrégat absent");
        assert!(html.contains(">622<") || html.contains("622"), "somme des Autres absente");
        // ≤ 6 PA : toutes affichées, pas d'agrégat.
        let mut s6 = snap();
        s6.pa.truncate(6);
        let html = render(&data(&s6));
        assert!(html.contains("Qweeby"));
        assert!(!html.contains("Autres ("));
    }

    #[test]
    fn anneau_couvre_le_cercle_et_omet_les_segments_vides() {
        let s = snap();
        let segs = ring_segments(&s);
        let total: f64 = segs.iter().map(|(_, len, _)| len).sum();
        assert!((total - RING_C).abs() < 0.5, "somme {total} ≠ circonférence");
        // Offsets cumulés : chaque segment démarre où finit le précédent.
        let mut cum = 0.0;
        for (_, len, offset) in &segs {
            assert!((offset + cum).abs() < 0.01, "offset {offset} attendu {}", -cum);
            cum += len;
        }
        // Un run sans échec ni verdict inconnu : 3 segments seulement.
        let mut sain = snap();
        sain.no_verdict = 0;
        sain.failed = 0;
        assert_eq!(ring_segments(&sain).len(), 3);
    }

    #[test]
    fn run_vide_ne_panique_pas() {
        let mut s = snap();
        (s.done, s.exists, s.ctc, s.no_verdict, s.failed, s.done_lines) = (0, 0, 0, 0, 0, 0);
        s.pa = Vec::new();
        let html = render(&data(&s));
        assert!(html.contains("0,0\u{202F}%"), "pourcentages sur run vide");
    }

    #[test]
    fn date_francaise() {
        use chrono::TimeZone;
        let d = chrono::Local.with_ymd_and_hms(2026, 7, 16, 18, 42, 0).unwrap();
        assert_eq!(date_fr_longue(&d), "16 juillet 2026");
        let d = chrono::Local.with_ymd_and_hms(2027, 1, 1, 0, 0, 0).unwrap();
        assert_eq!(date_fr_longue(&d), "1 janvier 2027");
    }

    #[test]
    fn formats_francais() {
        assert_eq!(fmt_int(0), "0");
        assert_eq!(fmt_int(999), "999");
        assert_eq!(fmt_int(11_942), "11\u{202F}942");
        assert_eq!(fmt_int(1_234_567), "1\u{202F}234\u{202F}567");
        assert_eq!(fmt_pct(4_550, 11_942), "38,1\u{202F}%");
        assert_eq!(fmt_pct(0, 0), "0,0\u{202F}%");
        assert_eq!(fmt_pct(1, 1), "100,0\u{202F}%");
    }

    #[test]
    fn couverture_section_rendue_et_distincte_du_reseau() {
        let s = snap();
        let cov = cov_full();
        let html = render(&data_with(&s, &cov));
        assert!(html.contains("Présence déclarative en annuaire"), "titre section");
        assert!(html.contains("Annuaire Peppol"), "ligne Peppol");
        assert!(html.contains("PPF utilisable"), "ligne usable");
        assert!(html.contains("812"), "présent Peppol");
        assert!(html.contains("570"), "usable PPF");
        // Coexiste avec le réseau, sans le remplacer.
        assert!(html.contains("Provisionnés Réseau Peppol"), "KPI réseau conservé");
    }

    #[test]
    fn couverture_absente_si_aucun_annuaire() {
        // data() utilise EMPTY_COV (les deux annuaires None) → pas de section.
        let html = render(&data(&snap()));
        assert!(!html.contains("Présence déclarative en annuaire"));
    }

    #[test]
    fn couverture_variables_ppf_dans_les_trois_themes() {
        // --ppf-l4 défini pour dark (racine), clair (@media) et impression.
        let html = render(&data(&snap()));
        assert!(
            html.matches("--ppf-l4").count() >= 3,
            "variable PPF absente d'un contexte de thème"
        );
    }
}
