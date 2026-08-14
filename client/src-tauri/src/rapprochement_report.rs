//! Rapport HTML d'un rapprochement appliqué.
//!
//! Module **pur** : aucune I/O, aucune horloge, aucune dépendance à Tauri ni à
//! `commands`. Ce qui vient du disque (noms de fichiers, empreinte) ou de
//! l'horloge (date longue) est fourni tout prêt par l'appelant, comme pour
//! `plan_report`.

use crate::rapprochement::{Action, Ecart, Nature, Rapprochement};
use crate::report::{esc, fmt_int, CSS};

/// Un fichier de livraison, tel que le rapport en parle : par son nom.
///
/// Vue propre à ce module plutôt que `commands::FichierMep` : le rapport ne
/// connaît pas de chemins — ceux de la machine qui a produit le lot n'ont
/// aucun sens pour qui le reçoit — et un module pur n'a pas à dépendre de
/// `commands`.
pub struct FichierLivre<'a> {
    pub nom: &'a str,
    pub mep_id: usize,
    pub mep_date: &'a str,
    pub comptes: usize,
}

/// Où se trouvait une ligne avant le rapprochement.
///
/// `Action::Deplacer` ne porte que la **destination**, et `appliquer` mute les
/// lignes en place : après application, le run d'origine n'existe plus nulle
/// part. La commande capture donc ces positions AVANT d'appliquer.
pub struct PositionAvant {
    pub run_num: String,
    /// ISO, comme partout ailleurs dans le projet.
    pub run_date: String,
    pub mep_id: usize,
}

/// Un compte retiré **à la main**, tel que le rapport en parle.
///
/// **Propriétaire de ses chaînes**, comme `PositionAvant` et contrairement à
/// `FichierLivre` : la date est CALCULÉE par la commande (conversion d'un
/// horodatage stocké), elle n'existe donc nulle part où l'emprunter.
pub struct RetraitManuel {
    pub cf: String,
    /// Date du retrait, **en ISO** — comme `PositionAvant::run_date` et
    /// `FichierLivre::mep_date`. Le rendu la met en forme via `date_fr`.
    pub le: String,
    /// Saisi par l'utilisateur. **Texte libre**, échappé au point d'insertion.
    pub motif: String,
    /// La MEP de la ligne est passée : le compte figure dans un fichier déjà
    /// transmis. Même conséquence qu'un `Ecart.gelee`, même traitement.
    pub gelee: bool,
}

/// Ce que la liste des retraits manuels prend pour origine. Le document écrit
/// la date ET ce qu'elle désigne : « depuis le 28/07 » ne dit pas au lecteur
/// pourquoi la liste commence là.
pub enum Depuis {
    /// ISO. Le plan a déjà été rapproché : la liste part de cette date-là.
    DernierRapprochement(String),
    /// ISO. Le plan n'a jamais été rapproché : elle part de sa génération.
    GenerationDuPlan(String),
}

pub struct RapprochementReportData<'a> {
    /// Nom du fichier qui a produit le plan, capturé AVANT réalignement.
    pub fichier_avant: &'a str,
    pub fichier_apres: &'a str,
    /// SHA-256 du fichier rapproché : le destinataire le compare.
    pub empreinte: &'a str,
    /// Déjà formatée par `report::date_fr_longue` — ce module n'a pas d'horloge.
    pub date_longue: &'a str,
    pub version: &'a str,
    pub rapprochement: &'a Rapprochement,
    pub fichiers: &'a [FichierLivre<'a>],
    /// Fichiers de MEP supprimés parce que leur MEP s'est vidée. Noms nus.
    pub obsoletes: &'a [String],
    /// Position d'origine des lignes, capturée AVANT `appliquer`. Clé : n° de CF.
    pub origines: &'a std::collections::BTreeMap<String, PositionAvant>,
    /// Retraits faits à la main depuis `depuis`, **triés par la commande**
    /// (date puis n° de CF) : le rendu ne réordonne pas. Le tri appartient au
    /// producteur, qui seul connaît les horodatages bruts.
    pub retraits_manuels: &'a [RetraitManuel],
    pub depuis: &'a Depuis,
    /// Avertissement d'annuaire PPF incomplet, s'il y a lieu.
    pub annuaire_incomplet: Option<&'a str>,
}

/// Écarts portant l'action demandée.
fn par_action<'a>(r: &'a Rapprochement, f: impl Fn(&Action) -> bool) -> Vec<&'a Ecart> {
    r.ecarts.iter().filter(|e| f(&e.action)).collect()
}

/// Écarts retirés dont la nature est celle-ci. `disparus` sépare les deux
/// motifs de retrait : ils n'appellent pas la même vérification côté
/// destinataire.
fn retraits_de_nature(r: &Rapprochement, disparus: bool) -> Vec<&Ecart> {
    r.ecarts
        .iter()
        .filter(|e| matches!(e.action, Action::Retirer { .. }))
        .filter(|e| matches!(e.nature, Nature::DisparuDuFichier) == disparus)
        .collect()
}

/// Une date ISO rendue en jour/mois/année. Le rapport est lu par des humains.
fn date_fr(iso: &str) -> String {
    match (iso.get(0..4), iso.get(5..7), iso.get(8..10)) {
        (Some(a), Some(m), Some(j)) => format!("{j}/{m}/{a}"),
        _ => iso.to_string(),
    }
}

/// Un jour de cycle, ou son absence. `0` est la sentinelle de
/// `rapprochement::calculer` pour un jour que le fichier ne permet pas de
/// lire : hors du domaine 1–31, il ne s'affiche jamais tel quel.
fn jour(j: u8) -> String {
    if j == 0 {
        "illisible".into()
    } else {
        j.to_string()
    }
}

/// Cellule « avant → après ». L'ancienne valeur reste lisible : c'est ce que
/// le destinataire a sous les yeux dans le lot précédent.
fn chg(avant: &str, apres: &str) -> String {
    format!(
        "<td class=\"chg\"><span class=\"old\">{}</span>\
         <span class=\"arr\">→</span>{}</td>",
        esc(avant),
        esc(apres)
    )
}

/// Le run d'arrivée d'un déplacement : son numéro nu (pour comparer), son
/// libellé lisible, et sa MEP.
///
/// **Rien n'est échappé ici** : `chg` échappe ses deux arguments, et échapper
/// en amont produirait `&amp;amp;`. L'échappement se fait au point d'insertion.
fn destination(a: &Action) -> Option<(String, String, usize)> {
    match a {
        Action::Deplacer { run_num, run_date, mep_id, .. } => Some((
            run_num.clone(),
            format!("Run {} — {}", run_num, date_fr(&run_date.to_string())),
            *mep_id,
        )),
        _ => None,
    }
}

/// Le motif d'un retrait, ou une chaîne vide si l'action n'en est pas un.
fn motif(a: &Action) -> &str {
    match a {
        Action::Retirer { motif } => motif.as_str(),
        _ => "",
    }
}

/// Ouvre une section titrée. **Rien n'est écrit si la liste est vide** : un
/// tableau à en-tête seul dit « rien à signaler », pas « sans objet ».
fn section(html: &mut String, titre: &str, sous_titre: &str, entetes: &[&str], vide: bool) {
    if vide {
        return;
    }
    html.push_str(&format!("<h2>{}</h2>\n", esc(titre)));
    html.push_str(&format!("<p class=\"h2sub\">{}</p>\n", esc(sous_titre)));
    html.push_str("<div class=\"tbl\">\n<table>\n<thead><tr>");
    for e in entetes {
        html.push_str(&format!("<th>{}</th>", esc(e)));
    }
    html.push_str("</tr></thead>\n<tbody>\n");
}

fn fin_section(html: &mut String, vide: bool) {
    if !vide {
        html.push_str("</tbody>\n</table>\n</div>\n");
    }
}

pub fn render(d: &RapprochementReportData) -> String {
    let r = d.rapprochement;
    let retires = par_action(r, |a| matches!(a, Action::Retirer { .. }));
    let inelig = retraits_de_nature(r, false).len();
    let disparus = retraits_de_nature(r, true).len();
    let deplaces = par_action(r, |a| matches!(a, Action::Deplacer { .. })).len();
    let rafraichis = par_action(r, |a| matches!(a, Action::Rafraichir)).len();
    let actives = r.inchangees + r.ecarts.len();

    let avant = esc(d.fichier_avant);
    let apres = esc(d.fichier_apres);

    let mut html = String::with_capacity(16 * 1024);
    html.push_str("<!doctype html>\n<html lang=\"fr\">\n<head>\n<meta charset=\"utf-8\">\n");
    html.push_str(&format!(
        "<title>Rapprochement du plan — {apres}</title>\n<style>{CSS}{CSS_RAPPRO}</style>\n\
         </head>\n<body>\n<div class=\"page\">\n"
    ));
    html.push_str(&format!(
        "<header>\n<div class=\"wordmark\">SUPER POPAUL</div>\n\
         <h1>Rapprochement du plan de charge</h1>\n\
         <p class=\"meta\">Plan établi sur <b>{avant}</b> · rapproché de <b>{apres}</b> \
         · appliqué le <b>{}</b></p>\n\
         <p class=\"hash\">Empreinte du fichier rapproché : {}</p>\n</header>\n",
        esc(d.date_longue),
        esc(d.empreinte),
    ));

    html.push_str("<section class=\"kpis\">\n");
    html.push_str(&format!(
        "<div class=\"kpi red\"><div class=\"v\">{}</div>\
         <div class=\"l\">comptes retirés</div>\
         <div class=\"abs\"><b>{}</b> non éligibles · <b>{}</b> disparu{}</div></div>\n",
        fmt_int(retires.len() as u64),
        fmt_int(inelig as u64),
        fmt_int(disparus as u64),
        if disparus > 1 { "s" } else { "" },
    ));
    html.push_str(&format!(
        "<div class=\"kpi gold\"><div class=\"v\">{}</div>\
         <div class=\"l\">comptes déplacés</div>\
         <div class=\"abs\">jour de cycle changé</div></div>\n",
        fmt_int(deplaces as u64),
    ));
    html.push_str(&format!(
        "<div class=\"kpi amber\"><div class=\"v\">{}</div>\
         <div class=\"l\">plateformes corrigées</div>\
         <div class=\"abs\">la ligne ne bouge pas</div></div>\n",
        fmt_int(rafraichis as u64),
    ));
    html.push_str(&format!(
        "<div class=\"kpi green\"><div class=\"v\">{}</div>\
         <div class=\"l\">lignes inchangées</div>\
         <div class=\"abs\">sur <b>{}</b> actives</div></div>\n",
        fmt_int(r.inchangees as u64),
        fmt_int(actives as u64),
    ));
    // Cinquième tuile, rendue seulement si elle est non nulle — les quatre
    // autres décrivent ce que le rapprochement a EXAMINÉ, et « 0 déplacé » est
    // un constat ; celle-ci parlerait d'un geste qui n'a pas eu lieu. NEUTRE :
    // elle est hors de l'arithmétique des autres, lui donner du rouge en
    // ferait un second total de retraits calculés.
    if !d.retraits_manuels.is_empty() {
        html.push_str(&format!(
            "<div class=\"kpi hors\"><div class=\"v\">{}</div>\
             <div class=\"l\">retirés à la main</div>\
             <div class=\"abs\">hors du calcul</div></div>\n",
            fmt_int(d.retraits_manuels.len() as u64),
        ));
    }
    html.push_str("</section>\n");

    // Avertissements DÉRIVÉS DU CALCUL : ils décrivent tous ce que le
    // rapprochement va faire.
    if !r.avertissements.is_empty() {
        html.push_str("<section class=\"warn\">\n<h2>Avertissements</h2>\n<ul>\n");
        for a in &r.avertissements {
            html.push_str(&format!("<li>{}</li>\n", esc(a)));
        }
        html.push_str("</ul>\n</section>\n");
    }
    // Séparé des précédents, et jamais fondu avec eux : celui-ci ne décrit pas
    // ce que le rapprochement fait, il prévient qu'il est INCOMPLET. Dans un
    // document transmis, sans lui, « 0 éligibilité perdue » se lit comme un
    // constat au lieu d'un aveu d'aveuglement.
    if let Some(a) = d.annuaire_incomplet {
        html.push_str(&format!(
            "<section class=\"warn\">\n<h2>Annuaire PPF incomplet</h2>\n<ul>\n\
             <li>{}<br><b>Les éligibilités PPF perdues ne sont pas visibles</b> — \
             le compte « 0 éligibilité perdue » ne vaut que pour le verdict CTC.</li>\n\
             </ul>\n</section>\n",
            esc(a)
        ));
    }

    // Retraits portant sur une MEP déjà transmise. AVANT tout le reste : les
    // fichiers étant cumulatifs, le destinataire a une version antérieure
    // entre les mains, et c'est la seule information du rapport qui l'oblige
    // à agir sur ce qu'il a déjà reçu.
    let geles: Vec<&&Ecart> = retires.iter().filter(|e| e.gelee).collect();
    // Même conséquence, même section : le destinataire tient une version
    // antérieure du fichier, que le retrait vienne du calcul ou d'une décision.
    // La section dit ce qui arrive au lecteur, pas d'où vient la cause.
    let geles_manuels: Vec<&RetraitManuel> =
        d.retraits_manuels.iter().filter(|m| m.gelee).collect();
    if !geles.is_empty() || !geles_manuels.is_empty() {
        html.push_str(
            "<section class=\"warn danger\">\n\
             <h2>Retrait portant sur une mise en production déjà transmise</h2>\n<ul>\n",
        );
        for e in &geles {
            html.push_str(&format!(
                "<li>Le compte <b>{}</b> figurait dans un fichier qui vous a déjà été \
                 transmis. Les fichiers étant cumulatifs, il ne figure plus dans aucun \
                 fichier de ce lot. Motif : <b>{}</b>.</li>\n",
                esc(&e.cf),
                esc(motif(&e.action)),
            ));
        }
        // Les manuels à la suite des calculés — ordre déterministe. Le motif
        // est ici la phrase de l'utilisateur, qui dit la décision mieux qu'un
        // libellé généré.
        for m in &geles_manuels {
            html.push_str(&format!(
                "<li>Le compte <b>{}</b> figurait dans un fichier qui vous a déjà été \
                 transmis. Les fichiers étant cumulatifs, il ne figure plus dans aucun \
                 fichier de ce lot. Motif : <b>{}</b>.</li>\n",
                esc(&m.cf),
                esc(&m.motif),
            ));
        }
        html.push_str("</ul>\n</section>\n");
    }

    // ① Éligibilité perdue.
    let inelig_l = retraits_de_nature(r, false);
    section(
        &mut html,
        "Comptes retirés — éligibilité perdue",
        "Le compte est au plan mais le fichier ne le déclare plus éligible.",
        &["N° de CF", "Éligibilité", "Motif"],
        inelig_l.is_empty(),
    );
    for e in &inelig_l {
        let (av, ap) = match &e.nature {
            Nature::EligibilitePerdue { avant, apres } => (avant.as_str(), apres.as_str()),
            _ => ("", ""),
        };
        html.push_str(&format!(
            "<tr><td>{}</td>{}<td>{}</td></tr>\n",
            esc(&e.cf),
            chg(av, ap),
            esc(motif(&e.action)),
        ));
    }
    fin_section(&mut html, inelig_l.is_empty());

    // ② Disparus du fichier.
    let disparus_l = retraits_de_nature(r, true);
    section(
        &mut html,
        "Comptes retirés — disparus du fichier",
        "Le compte était au plan et n'apparaît plus dans le fichier rapproché.",
        &["N° de CF", "Motif"],
        disparus_l.is_empty(),
    );
    for e in &disparus_l {
        html.push_str(&format!(
            "<tr><td>{}</td><td>{}</td></tr>\n",
            esc(&e.cf),
            esc(motif(&e.action)),
        ));
    }
    fin_section(&mut html, disparus_l.is_empty());

    // ③ Retirés à la main. Groupé avec ① et ② : le document range par
    // CONSÉQUENCE pour le destinataire, et les trois répondent à « quels
    // comptes ne sont plus dans mes fichiers ». La colonne « Retiré le » est ce
    // qui distingue ce tableau — ses comptes ont pu quitter le plan des
    // semaines avant ce rapprochement, là où les deux autres sont retirés à
    // l'instant.
    let sous_titre_manuels = match d.depuis {
        Depuis::DernierRapprochement(iso) => format!(
            "Retirés à la main depuis le {}, dernier rapprochement. \
             Ces comptes ne figurent dans aucun fichier de ce lot.",
            date_fr(iso)
        ),
        Depuis::GenerationDuPlan(iso) => format!(
            "Retirés à la main depuis la génération du plan, le {}. \
             Ces comptes ne figurent dans aucun fichier de ce lot.",
            date_fr(iso)
        ),
    };
    section(
        &mut html,
        "Comptes retirés — décision manuelle",
        &sous_titre_manuels,
        &["N° de CF", "Retiré le", "Motif"],
        d.retraits_manuels.is_empty(),
    );
    for m in d.retraits_manuels {
        html.push_str(&format!(
            "<tr><td>{}</td><td class=\"date\">{}</td><td>{}</td></tr>\n",
            esc(&m.cf),
            esc(&date_fr(&m.le)),
            esc(&m.motif),
        ));
    }
    fin_section(&mut html, d.retraits_manuels.is_empty());

    // ④ Déplacés.
    let deplaces_l = par_action(r, |a| matches!(a, Action::Deplacer { .. }));
    section(
        &mut html,
        "Comptes déplacés — jour de cycle changé",
        "Le fichier annonce un autre jour de facturation : la ligne suit son cycle.",
        &["N° de CF", "Jour", "Run", "MEP"],
        deplaces_l.is_empty(),
    );
    for e in &deplaces_l {
        let (javant, japres) = match &e.nature {
            Nature::JourChange { avant, apres } => (*avant, *apres),
            _ => (0, 0),
        };
        // Un `Deplacer` a toujours une destination ; la branche par défaut
        // n'existe que pour ne pas paniquer si un variant s'y glisse un jour.
        let (run_num, run_lbl, mep_id) =
            destination(&e.action).unwrap_or_else(|| (String::new(), "—".into(), 0));
        let avant = d.origines.get(&e.cf);
        let cell_run = match avant {
            // Le jour a changé sans changer de run : ne pas écrire deux fois
            // la même valeur, dire que le déplacement a été évalué.
            Some(p) if p.run_num == run_num => format!(
                "<td>{} <span class=\"same\">même run</span></td>",
                esc(&run_lbl)
            ),
            Some(p) => chg(
                &format!("Run {} — {}", p.run_num, date_fr(&p.run_date)),
                &run_lbl,
            ),
            None => format!("<td>{}</td>", esc(&run_lbl)),
        };
        let cell_mep = match avant {
            Some(p) if p.mep_id != mep_id => chg(&p.mep_id.to_string(), &mep_id.to_string()),
            _ => format!("<td>{mep_id}</td>"),
        };
        html.push_str(&format!(
            "<tr><td>{}</td>{}{}{}</tr>\n",
            esc(&e.cf),
            chg(&jour(javant), &jour(japres)),
            cell_run,
            cell_mep,
        ));
    }
    fin_section(&mut html, deplaces_l.is_empty());

    // ⑤ Plateformes corrigées.
    let plat_l = par_action(r, |a| matches!(a, Action::Rafraichir));
    section(
        &mut html,
        "Plateformes corrigées",
        "Le champ est mis à jour, la ligne ne change ni de run ni de MEP.",
        &["N° de CF", "Plateforme"],
        plat_l.is_empty(),
    );
    for e in &plat_l {
        let (av, ap) = match &e.nature {
            Nature::PlateformeChangee { avant, apres } => (avant.as_str(), apres.as_str()),
            _ => ("", ""),
        };
        html.push_str(&format!("<tr><td>{}</td>{}</tr>\n", esc(&e.cf), chg(av, ap)));
    }
    fin_section(&mut html, plat_l.is_empty());

    // Ce que le rapprochement n'a pas tranché. Ni vert ni rouge : en attente.
    let signales = par_action(r, |a| matches!(a, Action::Signaler));
    if !signales.is_empty() {
        html.push_str("<section class=\"todo\">\n<h2>À traiter à la main</h2>\n<ul>\n");
        for e in &signales {
            let quoi = match &e.nature {
                Nature::JourChange { avant, apres } if *apres == 0 => format!(
                    "annonce un jour de cycle <b>illisible</b> dans le fichier rapproché \
                     (il était à <b>{}</b>). La ligne n'a pas été déplacée.",
                    jour(*avant)
                ),
                Nature::JourChange { avant, apres } => format!(
                    "voit son jour de cycle passer de <b>{}</b> à <b>{}</b>{}. \
                     La ligne n'a pas été déplacée.",
                    jour(*avant),
                    jour(*apres),
                    if e.gelee {
                        " alors qu'il est gelé (mise en production déjà transmise)"
                    } else {
                        ", sans run disponible pour l'accueillir"
                    },
                ),
                Nature::EligibilitePerdue { avant, apres } => {
                    format!("passe de <b>{}</b> à <b>{}</b>.", esc(avant), esc(apres))
                }
                Nature::PlateformeChangee { avant, apres } => format!(
                    "change de plateforme : <b>{}</b> → <b>{}</b>.",
                    esc(avant),
                    esc(apres)
                ),
                Nature::DisparuDuFichier => {
                    "n'apparaît plus dans le fichier rapproché.".to_string()
                }
            };
            html.push_str(&format!("<li>Le compte <b>{}</b> {}</li>\n", esc(&e.cf), quoi));
        }
        html.push_str("</ul>\n</section>\n");
    }

    // Ce que le destinataire a effectivement en main après ce lot.
    if !d.fichiers.is_empty() || !d.obsoletes.is_empty() {
        html.push_str("<h2>Fichiers de livraison produits</h2>\n");
        html.push_str(
            "<p class=\"h2sub\">Les fichiers sont cumulatifs : celui de la MEP <i>n</i> \
             contient les comptes des MEP 1 à <i>n</i>. Ils remplacent ceux du lot \
             précédent.</p>\n",
        );
        html.push_str(
            "<div class=\"tbl\">\n<table>\n<thead><tr><th>Fichier</th><th>MEP</th>\
             <th>Date de MEP</th><th class=\"num\">Comptes</th></tr></thead>\n<tbody>\n",
        );
        for f in d.fichiers {
            html.push_str(&format!(
                "<tr><td>{}</td><td>{}</td><td>{}</td>\
                 <td class=\"num\"><b>{}</b></td></tr>\n",
                esc(f.nom),
                f.mep_id,
                date_fr(f.mep_date),
                fmt_int(f.comptes as u64),
            ));
        }
        for o in d.obsoletes {
            html.push_str(&format!(
                "<tr class=\"gone\"><td>{}</td><td>—</td><td>—</td>\
                 <td class=\"num why\">supprimé — plus aucun compte</td></tr>\n",
                esc(o),
            ));
        }
        html.push_str("</tbody>\n</table>\n</div>\n");
    }

    html.push_str(&format!(
        "<footer>\n<span>Le détail compte par compte figure dans le classeur \
         du périmètre.</span>\n<span>Super Popaul {}</span>\n</footer>\n\
         </div>\n</body>\n</html>\n",
        esc(d.version),
    ));
    html
}

/// Ajouts de style propres à ce rapport. Repris tels quels de la maquette
/// validée du 28/07/2026 ; rien de `report::CSS` n'est modifié.
const CSS_RAPPRO: &str = r#"
  .warn.danger { border-left-color: var(--red); }
  .warn.danger h2 { color: var(--red); }
  .warn.danger li::marker { color: var(--red); }
  .chg .old { color: var(--muted); }
  .chg .arr { color: var(--muted); padding: 0 5px; }
  .same { color: var(--muted); font-size: 12px; padding-left: 6px; }
  .hash { font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
    font-size: 11.5px; color: var(--muted); word-break: break-all; }
  .todo { background: var(--card); border: 1px solid var(--border);
    border-left: 3px solid var(--pa-autres); border-radius: 8px; padding: 14px 18px; }
  .todo h2 { margin: 0 0 8px; font-size: 13px; text-transform: uppercase;
    letter-spacing: .08em; color: var(--fg); }
  .todo h2::after { display: none; }
  tbody tr.gone td { color: var(--muted); }
  tbody tr.gone .why { font-size: 12px; }
  .kpi.hors { border-left: 3px solid var(--pa-autres); }
  .kpi.hors .v { color: var(--fg); }
  td.date { white-space: nowrap; color: var(--muted); }
"#;

#[cfg(test)]
mod tests {
    use super::*;

    /// Le corps du rapport, feuille de style exclue.
    ///
    /// Le CSS est inliné : chercher une sous-chaîne dans le HTML entier fait
    /// matcher une règle ou un commentaire de style. Le module `plan_report`
    /// s'y est laissé prendre trois fois — assertions vertes sur une fonction
    /// qui ne produisait rien.
    fn corps(html: &str) -> &str {
        html.split("</style>")
            .nth(1)
            .expect("le rapport doit contenir une feuille de style")
    }

    fn vide() -> Rapprochement {
        Rapprochement::default()
    }

    /// Table d'origines vide, partagée : `donnees()` rend une struct
    /// empruntante, elle ne peut pas emprunter un temporaire local.
    fn origines_vides() -> &'static std::collections::BTreeMap<String, PositionAvant> {
        static VIDE: std::sync::OnceLock<std::collections::BTreeMap<String, PositionAvant>> =
            std::sync::OnceLock::new();
        VIDE.get_or_init(Default::default)
    }

    /// `Depuis` par défaut, partagé — même motif que `origines_vides()`.
    fn depuis_defaut() -> &'static Depuis {
        static D: std::sync::OnceLock<Depuis> = std::sync::OnceLock::new();
        D.get_or_init(|| Depuis::DernierRapprochement("2026-07-28".into()))
    }

    fn retrait_manuel(cf: &str, le: &str, motif: &str, gelee: bool) -> RetraitManuel {
        RetraitManuel { cf: cf.into(), le: le.into(), motif: motif.into(), gelee }
    }

    fn donnees(r: &Rapprochement) -> RapprochementReportData<'_> {
        RapprochementReportData {
            fichier_avant: "brm2606.csv",
            fichier_apres: "brm2607.csv",
            empreinte: "9f3c1ab27de4508bb6a1e0f47c25d9836ea15b0c7d42f98e3a6b5c0197de24af",
            date_longue: "mardi 28 juillet 2026",
            version: "1.6.0",
            rapprochement: r,
            fichiers: &[],
            obsoletes: &[],
            origines: origines_vides(),
            retraits_manuels: &[],
            depuis: depuis_defaut(),
            annuaire_incomplet: None,
        }
    }

    #[test]
    fn une_cinquieme_tuile_compte_les_retraits_manuels() {
        let r = vide();
        let manuels = vec![
            retrait_manuel("4100238091", "2026-07-31", "Périmètre repoussé à 2027", false),
            retrait_manuel("4100243662", "2026-07-31", "Périmètre repoussé à 2027", false),
        ];
        let mut d = donnees(&r);
        d.retraits_manuels = &manuels;
        let html = render(&d);
        let c = corps(&html);
        assert!(c.contains("retirés à la main"), "libellé de la tuile absent");
        assert!(c.contains("hors du calcul"), "la tuile doit dire qu'elle est hors du compte");
        assert!(c.contains("<div class=\"v\">2</div>"), "compte des retraits manuels absent");
    }

    #[test]
    fn sans_retrait_manuel_la_cinquieme_tuile_n_existe_pas() {
        // Les quatre autres tuiles décrivent ce que le rapprochement a
        // EXAMINÉ : « 0 déplacé » est un constat. Une tuile à zéro sur des
        // retraits manuels parlerait d'un geste qui n'a pas eu lieu.
        let r = vide();
        let html = render(&donnees(&r));
        assert!(!corps(&html).contains("retirés à la main"));
    }

    #[test]
    fn les_retraits_manuels_ne_faussent_pas_l_arithmetique_du_resume() {
        // `inchangées + retirés + déplacés + rafraîchis + signalés = actives`.
        // Les retraits manuels sont HORS de cette somme — `calculer` les a
        // sautés, ils ne sont ni dans `ecarts` ni dans `inchangees`. Les
        // compter dans « comptes retirés » ferait dépasser le total dans un
        // document transmis.
        let mut r = vide();
        r.inchangees = 143;
        r.ecarts = vec![ecart_eligibilite("4100000001")];
        let manuels = vec![
            retrait_manuel("4100238091", "2026-07-31", "décision métier", false),
            retrait_manuel("4100243662", "2026-07-31", "décision métier", false),
            retrait_manuel("4100247788", "2026-08-06", "litige", false),
        ];
        let mut d = donnees(&r);
        d.retraits_manuels = &manuels;
        let html = render(&d);
        let c = corps(&html);
        assert!(c.contains("<div class=\"v\">1</div>"), "un seul retrait CALCULÉ");
        assert!(
            c.contains("sur <b>144</b> actives"),
            "les actives restent 143 inchangées + 1 écart : les manuels n'y entrent pas"
        );
        assert!(c.contains("<div class=\"v\">3</div>"), "les 3 manuels ont leur propre tuile");
    }

    fn ecart_eligibilite(cf: &str) -> Ecart {
        Ecart {
            cf: cf.into(),
            nature: Nature::EligibilitePerdue {
                avant: "CTC prêt".into(),
                apres: "CTC non prêt".into(),
            },
            action: Action::Retirer {
                motif: "2026-07-28 — CTC non prêt".into(),
            },
            gelee: false,
        }
    }

    fn ecart_disparu(cf: &str) -> Ecart {
        Ecart {
            cf: cf.into(),
            nature: Nature::DisparuDuFichier,
            action: Action::Retirer {
                motif: "absent du fichier rapproché".into(),
            },
            gelee: false,
        }
    }

    fn ecart_deplace(
        cf: &str,
        avant: u8,
        apres: u8,
        run: &str,
        run_date: &str,
        mep_id: usize,
    ) -> Ecart {
        let d = |iso: &str| chrono::NaiveDate::parse_from_str(iso, "%Y-%m-%d").unwrap();
        Ecart {
            cf: cf.into(),
            nature: Nature::JourChange { avant, apres },
            action: Action::Deplacer {
                run_num: run.into(),
                run_date: d(run_date),
                mep_id,
                mep_date: d(run_date),
            },
            gelee: false,
        }
    }

    fn ecart_plateforme(cf: &str, avant: &str, apres: &str) -> Ecart {
        Ecart {
            cf: cf.into(),
            nature: Nature::PlateformeChangee {
                avant: avant.into(),
                apres: apres.into(),
            },
            action: Action::Rafraichir,
            gelee: false,
        }
    }

    fn origines_de(couples: &[(&str, &str, &str, usize)]) -> std::collections::BTreeMap<String, PositionAvant> {
        couples
            .iter()
            .map(|(cf, run_num, run_date, mep_id)| {
                (
                    (*cf).to_string(),
                    PositionAvant {
                        run_num: (*run_num).into(),
                        run_date: (*run_date).into(),
                        mep_id: *mep_id,
                    },
                )
            })
            .collect()
    }

    /// Les titres de section, **avec leur balise**.
    ///
    /// Chercher « jour de cycle changé » nu passerait sans qu'aucune section
    /// existe : le libellé du KPI « comptes déplacés » porte déjà ces mots.
    /// Le premier jet de ces tests s'y est laissé prendre.
    const T_MANUELS: &str = "<h2>Comptes retirés — décision manuelle</h2>";

    #[test]
    fn le_tableau_des_retraits_manuels_donne_date_et_motif() {
        let r = vide();
        let manuels = vec![
            retrait_manuel(
                "4100238091",
                "2026-07-31",
                "Exclusion décidée en comité — périmètre repoussé à 2027",
                false,
            ),
            retrait_manuel("4100247788", "2026-08-06", "Litige commercial en cours", true),
        ];
        let mut d = donnees(&r);
        d.retraits_manuels = &manuels;
        let html = render(&d);
        let c = corps(&html);
        assert!(c.contains(T_MANUELS), "section des retraits manuels absente");
        assert!(c.contains("4100238091"));
        // La date est rendue en clair : elle est ce qui distingue ce tableau
        // des deux autres, dont les comptes sont retirés à l'instant.
        assert!(c.contains("31/07/2026"), "date du retrait absente ou non mise en forme");
        assert!(c.contains("06/08/2026"));
        assert!(c.contains("Exclusion décidée en comité"), "motif absent");
    }

    #[test]
    fn le_tableau_des_retraits_manuels_dit_depuis_quand() {
        // La date de référence n'est pas décorative : sans elle, le lecteur ne
        // sait pas si la liste couvre une semaine ou six mois.
        let r = vide();
        let manuels = vec![retrait_manuel("4100238091", "2026-07-31", "décision métier", false)];
        let mut d = donnees(&r);
        d.retraits_manuels = &manuels;

        let apres_rappro = Depuis::DernierRapprochement("2026-07-28".into());
        d.depuis = &apres_rappro;
        let h1 = render(&d);
        let c1 = corps(&h1);
        assert!(c1.contains("28/07/2026"), "date de référence absente");
        assert!(
            c1.contains("dernier rapprochement"),
            "la nature de la date de référence doit se lire"
        );

        let jamais = Depuis::GenerationDuPlan("2026-06-02".into());
        d.depuis = &jamais;
        let h2 = render(&d);
        let c2 = corps(&h2);
        assert!(c2.contains("02/06/2026"));
        assert!(
            c2.contains("génération du plan"),
            "un plan jamais rapproché doit le dire, pas inventer un rapprochement"
        );
        assert!(
            !c2.contains("dernier rapprochement"),
            "les deux formulations ne doivent jamais coexister"
        );
    }

    #[test]
    fn sans_retrait_manuel_le_tableau_n_existe_pas() {
        let r = vide();
        let html = render(&donnees(&r));
        assert!(!corps(&html).contains(T_MANUELS));
    }

    #[test]
    fn un_motif_saisi_par_l_utilisateur_sort_echappe() {
        // Première chaîne d'origine HUMAINE du document : les motifs des deux
        // autres tableaux sont générés par le code (`format!`), celui-ci est
        // tapé dans une boîte de dialogue. Un `esc` oublié ici injecte du
        // balisage dans une pièce transmise.
        let r = vide();
        let manuels = vec![retrait_manuel(
            "<script>alert(1)</script>",
            "2026-07-31",
            "A&B <script>alert(2)</script>",
            false,
        )];
        let mut d = donnees(&r);
        d.retraits_manuels = &manuels;
        let html = render(&d);
        let c = corps(&html);
        assert!(!c.contains("<script>"), "motif ou n° de CF non échappé");
        assert!(c.contains("&lt;script&gt;"));
        assert!(c.contains("A&amp;B"), "l'esperluette du motif doit être échappée");
        assert!(!c.contains("&amp;amp;"), "échappé deux fois");
    }

    const T_ELIG: &str = "<h2>Comptes retirés — éligibilité perdue</h2>";
    const T_DISPARUS: &str = "<h2>Comptes retirés — disparus du fichier</h2>";
    const T_DEPLACES: &str = "<h2>Comptes déplacés — jour de cycle changé</h2>";
    const T_PLATEFORMES: &str = "<h2>Plateformes corrigées</h2>";

    #[test]
    fn la_sentinelle_de_jour_illisible_ne_sort_jamais_en_chiffre() {
        // `jour()` est défensive : aujourd'hui un jour illisible produit
        // toujours `Signaler`, jamais `Deplacer`, donc aucun rendu ne lui
        // passe 0 — le « illisible » de la section « à traiter » est écrit en
        // dur. Testée directement, sans quoi son garde-fou pourrait
        // disparaître sans qu'un seul test rougisse, et le jour où un
        // `Deplacer` porterait la sentinelle, le rapport dirait « 0 ».
        assert_eq!(jour(0), "illisible");
        assert_eq!(jour(1), "1");
        assert_eq!(jour(31), "31");
    }

    #[test]
    fn un_numero_de_run_est_echappe_une_seule_fois() {
        // Le n° de run vient du CSV des runs, entrée non fiable comme le
        // reste. `destination` ne l'échappe pas — `chg` le fait au point
        // d'insertion. Échapper aux deux endroits n'ouvre aucune faille mais
        // afficherait « R&amp;13 » au lecteur, ce qui ne se remarque que sur
        // les rares runs concernés.
        let mut r = vide();
        r.ecarts = vec![ecart_deplace("4100000001", 8, 15, "R&13", "2026-09-22", 3)];
        let origines = origines_de(&[("4100000001", "R&12", "2026-09-15", 3)]);
        let mut d = donnees(&r);
        d.origines = &origines;
        let html = render(&d);
        let c = corps(&html);
        assert!(c.contains("R&amp;13"), "le run d'arrivée doit être échappé");
        assert!(c.contains("R&amp;12"), "le run d'origine doit être échappé");
        assert!(
            !c.contains("&amp;amp;"),
            "échappé deux fois : le lecteur verrait « R&amp;13 » au lieu de « R&13 »"
        );
    }

    #[test]
    fn les_avertissements_du_calcul_et_de_l_annuaire_ne_se_confondent_pas() {
        // Décision verrouillée au chantier précédent : les premiers décrivent
        // ce que le rapprochement FAIT, le second prévient qu'il est
        // INCOMPLET. Les fondre ferait disparaître la nuance.
        let mut r = vide();
        r.avertissements = vec!["le run 12 n'accueille plus de compte".into()];
        let mut d = donnees(&r);
        d.annuaire_incomplet = Some("l'annuaire PPF a été construit par cumul de 3 fichiers");
        let html = render(&d);
        let c = corps(&html);
        // Cherchés ÉCHAPPÉS : `esc` rend l'apostrophe en `&#39;`, et un
        // avertissement backend est du texte non fiable comme le reste.
        assert!(c.contains("le run 12 n&#39;accueille plus de compte"));
        assert!(c.contains("l&#39;annuaire PPF a été construit par cumul de 3 fichiers"));
        assert!(c.contains("<h2>Annuaire PPF incomplet</h2>"), "titre propre à l'annuaire absent");
        assert_eq!(
            c.matches("<section class=\"warn\">").count(),
            2,
            "les deux avertissements doivent vivre dans deux encadrés distincts"
        );
    }

    #[test]
    fn sans_avertissement_aucun_encadre_n_est_rendu() {
        let r = vide();
        let html = render(&donnees(&r));
        assert!(!corps(&html).contains("<section class=\"warn\">"));
    }

    #[test]
    fn les_fichiers_produits_sont_listes_avec_leurs_comptes() {
        let r = vide();
        let fichiers = vec![
            FichierLivre {
                nom: "brm2607_plan_mep_1_2026-05-15.txt",
                mep_id: 1,
                mep_date: "2026-05-15",
                comptes: 28,
            },
            FichierLivre {
                nom: "brm2607_plan_mep_2_2026-06-12.txt",
                mep_id: 2,
                mep_date: "2026-06-12",
                comptes: 61,
            },
        ];
        let mut d = donnees(&r);
        d.fichiers = &fichiers;
        let html = render(&d);
        let c = corps(&html);
        assert!(c.contains("brm2607_plan_mep_1_2026-05-15.txt"));
        assert!(c.contains("15/05/2026"), "la date de MEP doit être lisible");
        assert!(c.contains("<b>28</b>"), "le nombre de comptes doit figurer");
        assert!(c.contains("<b>61</b>"));
    }

    #[test]
    fn un_fichier_supprime_est_dit_supprime() {
        // Une MEP vidée par les retraits perd son fichier. La ligne existe
        // pour dire qu'elle n'existe plus : le destinataire doit jeter sa
        // copie, qui n'est plus dans aucun lot.
        let r = vide();
        let obsoletes = vec!["brm2607_plan_mep_6_2026-11-27.txt".to_string()];
        let mut d = donnees(&r);
        d.obsoletes = &obsoletes;
        let html = render(&d);
        let c = corps(&html);
        assert!(c.contains("brm2607_plan_mep_6_2026-11-27.txt"));
        assert!(c.contains("supprimé"), "la raison de la disparition doit se lire");
    }

    #[test]
    fn sans_fichier_ni_obsolete_la_section_n_existe_pas() {
        let r = vide();
        let html = render(&donnees(&r));
        assert!(!corps(&html).contains("<h2>Fichiers de livraison produits</h2>"));
    }

    const T_ALERTE: &str = "Retrait portant sur une mise en production déjà transmise";
    const T_TODO: &str = "<h2>À traiter à la main</h2>";

    #[test]
    fn un_retrait_sur_mep_transmise_a_sa_propre_section() {
        // Les fichiers sont cumulatifs : le destinataire a une version
        // antérieure de ce fichier entre les mains. Noyer ce cas dans le
        // tableau général est le principal moyen de le rater.
        let mut r = vide();
        let mut e = ecart_eligibilite("4100238877");
        e.gelee = true;
        r.ecarts = vec![e, ecart_eligibilite("4100241902")];
        let html = render(&donnees(&r));
        let c = corps(&html);
        assert!(c.contains(T_ALERTE), "section des retraits sur MEP livrée absente");
        // Ce qui est vérifié ici, c'est la MISE EN ÉVIDENCE : l'alerte doit
        // précéder le premier tableau d'écarts, pas seulement exister.
        let avant_tableaux = c.split("<h2>Comptes retirés").next().unwrap_or("");
        assert!(
            avant_tableaux.contains("4100238877"),
            "le compte gelé doit apparaître AVANT les tableaux d'écarts"
        );
    }

    #[test]
    fn un_retrait_manuel_gele_rejoint_l_alerte_des_mep_transmises() {
        // L'alerte existe pour une CONSÉQUENCE, pas pour une provenance : les
        // fichiers étant cumulatifs, le destinataire tient une version
        // antérieure où le compte figurait. Un retrait manuel sur une MEP
        // passée produit exactement cette situation.
        let r = vide();
        let manuels = vec![
            retrait_manuel(
                "4100247788",
                "2026-08-06",
                "Litige commercial en cours — ne pas facturer",
                true,
            ),
            retrait_manuel("4100238091", "2026-07-31", "périmètre 2027", false),
        ];
        let mut d = donnees(&r);
        d.retraits_manuels = &manuels;
        let html = render(&d);
        let c = corps(&html);
        assert!(c.contains(T_ALERTE), "section d'alerte absente");
        let alerte = c
            .split(T_ALERTE)
            .nth(1)
            .expect("alerte absente")
            .split("</section>")
            .next()
            .unwrap_or("");
        assert!(alerte.contains("4100247788"), "le gelé doit être dans l'alerte");
        assert!(
            alerte.contains("Litige commercial en cours"),
            "le motif de l'utilisateur dit la décision mieux qu'un libellé généré"
        );
        assert!(
            !alerte.contains("4100238091"),
            "un retrait manuel NON gelé n'a rien à faire dans l'alerte"
        );
        // Dans l'alerte ET dans le tableau : la mise en évidence ne dispense
        // pas du tableau — c'est déjà le sort des retraits calculés gelés.
        assert!(c.contains(T_MANUELS), "le gelé doit AUSSI figurer au tableau");
    }

    #[test]
    fn sans_retrait_gele_ni_calcule_ni_manuel_l_alerte_n_existe_pas() {
        let r = vide();
        let manuels = vec![retrait_manuel("4100238091", "2026-07-31", "périmètre 2027", false)];
        let mut d = donnees(&r);
        d.retraits_manuels = &manuels;
        let html = render(&d);
        assert!(!corps(&html).contains(T_ALERTE));
    }

    #[test]
    fn sans_retrait_gele_la_section_d_alerte_n_existe_pas() {
        let mut r = vide();
        r.ecarts = vec![ecart_eligibilite("4100241902")];
        let html = render(&donnees(&r));
        assert!(!corps(&html).contains(T_ALERTE));
    }

    #[test]
    fn un_jour_illisible_se_dit_en_toutes_lettres() {
        // `apres: 0` est une sentinelle hors du domaine 1–31. L'afficher
        // comme un chiffre ferait lire « le compte passe au jour 0 ».
        let mut r = vide();
        r.ecarts = vec![Ecart {
            cf: "4100252009".into(),
            nature: Nature::JourChange { avant: 9, apres: 0 },
            action: Action::Signaler,
            gelee: false,
        }];
        let html = render(&donnees(&r));
        let c = corps(&html);
        // Cherché DANS la section, pas dans le document : les compteurs du
        // résumé affichent eux aussi des zéros, et `<b>0</b>` y figure.
        let todo = c
            .split("<section class=\"todo\">")
            .nth(1)
            .expect("la section « à traiter à la main » doit exister");
        assert!(todo.contains("illisible"), "le jour illisible doit se dire");
        assert!(!todo.contains("jour 0"), "la sentinelle ne doit jamais s'afficher");
        assert!(!todo.contains("<b>0</b>"), "la sentinelle ne doit jamais s'afficher");
    }

    #[test]
    fn un_signalement_n_est_pas_compte_parmi_les_changements() {
        // « Signaler » ne mute rien. Le compter dans les déplacés ferait
        // annoncer un mouvement qui n'a pas eu lieu.
        let mut r = vide();
        // Non nul, pour que le compteur des inchangées ne se confonde pas
        // avec les trois compteurs de changements.
        r.inchangees = 42;
        r.ecarts = vec![Ecart {
            cf: "4100251774".into(),
            nature: Nature::JourChange { avant: 9, apres: 24 },
            action: Action::Signaler,
            gelee: true,
        }];
        let html = render(&donnees(&r));
        let c = corps(&html);
        assert!(c.contains(T_TODO), "section des signalements absente");
        assert!(
            !c.contains(T_DEPLACES),
            "un signalement ne doit pas produire de tableau de déplacés"
        );
        assert_eq!(
            c.matches("<div class=\"v\">0</div>").count(),
            3,
            "retraits, déplacés et plateformes doivent tous rester à zéro"
        );
    }

    #[test]
    fn chaque_nature_a_son_tableau_avec_l_avant_et_l_apres() {
        let mut r = vide();
        r.ecarts = vec![
            ecart_eligibilite("4100000001"),
            ecart_disparu("4100000002"),
            ecart_deplace("4100000003", 8, 15, "R13", "2026-09-22", 3),
            ecart_plateforme("4100000004", "Serensia", "Docoon"),
        ];
        let html = render(&donnees(&r));
        let c = corps(&html);
        assert!(c.contains(T_ELIG), "section éligibilité absente");
        assert!(c.contains(T_DISPARUS), "section disparus absente");
        assert!(c.contains(T_DEPLACES), "section déplacés absente");
        assert!(c.contains(T_PLATEFORMES), "section plateformes absente");
        // L'avant reste lisible : c'est ce que le destinataire a sous les yeux.
        assert!(c.contains("CTC prêt"), "éligibilité d'avant absente");
        assert!(c.contains("CTC non prêt"), "éligibilité d'après absente");
        assert!(c.contains("Serensia"), "plateforme d'avant absente");
        assert!(c.contains("Docoon"), "plateforme d'après absente");
    }

    #[test]
    fn une_nature_sans_ecart_ne_produit_pas_de_tableau_vide() {
        // Un tableau à en-tête seul se lit « rien à signaler ici » alors qu'il
        // veut dire « cette question ne s'est pas posée ».
        let mut r = vide();
        r.ecarts = vec![ecart_eligibilite("4100000001")];
        let html = render(&donnees(&r));
        let c = corps(&html);
        assert!(c.contains(T_ELIG));
        assert!(!c.contains(T_DISPARUS), "tableau des disparus rendu à vide");
        assert!(!c.contains(T_DEPLACES), "tableau des déplacés rendu à vide");
        assert!(!c.contains(T_PLATEFORMES), "tableau des plateformes rendu à vide");
    }

    #[test]
    fn un_champ_venu_du_csv_sort_echappe() {
        // Un CSV est une entrée non fiable. `esc` oublié sur un seul champ
        // suffit à injecter du balisage dans un document qu'on transmet.
        let mut r = vide();
        r.ecarts = vec![ecart_plateforme("<script>alert(1)</script>", "A&B", "C<D")];
        let html = render(&donnees(&r));
        let c = corps(&html);
        assert!(!c.contains("<script>"), "le n° de CF n'est pas échappé");
        assert!(c.contains("&lt;script&gt;"), "le n° de CF devrait être échappé");
        assert!(c.contains("A&amp;B"), "la plateforme d'avant n'est pas échappée");
        assert!(c.contains("C&lt;D"), "la plateforme d'après n'est pas échappée");
    }

    #[test]
    fn un_deplacement_qui_ne_change_pas_de_run_ne_repete_pas_la_valeur() {
        // Le calcul produit un écart dès que le jour lu diffère, même si les
        // deux jours tombent dans le même run. Écrire « Run R16 → Run R16 »
        // est exact mais illisible : le lecteur cherche une différence qui
        // n'existe pas.
        let mut r = vide();
        r.ecarts = vec![ecart_deplace("4100245920", 30, 28, "R16", "2026-10-13", 5)];
        let origines = origines_de(&[("4100245920", "R16", "2026-10-13", 5)]);
        let mut d = donnees(&r);
        d.origines = &origines;
        let html = render(&d);
        let c = corps(&html);
        assert!(c.contains("même run"), "le cas « même run » doit se dire");
        assert_eq!(
            c.matches("Run R16").count(),
            1,
            "le run ne doit apparaître qu'une fois"
        );
        // Le jour, lui, reste en avant → après : c'est là qu'est le changement.
        assert!(c.contains("30"), "le jour d'avant doit rester lisible");
        assert!(c.contains("28"), "le jour d'après doit être là");
    }

    #[test]
    fn un_deplacement_vers_un_autre_run_montre_les_deux() {
        let mut r = vide();
        r.ecarts = vec![ecart_deplace("4100240115", 8, 15, "R13", "2026-09-22", 3)];
        let origines = origines_de(&[("4100240115", "R12", "2026-09-15", 3)]);
        let mut d = donnees(&r);
        d.origines = &origines;
        let html = render(&d);
        let c = corps(&html);
        assert!(c.contains("R12"), "le run d'origine doit figurer");
        assert!(c.contains("R13"), "le run d'arrivée doit figurer");
        assert!(!c.contains("même run"), "les runs diffèrent");
    }

    #[test]
    fn l_entete_nomme_les_deux_fichiers_et_l_empreinte() {
        // Le destinataire compare l'empreinte : sans elle, il ne peut pas
        // vérifier qu'il parle du même fichier que l'émetteur.
        let r = vide();
        let html = render(&donnees(&r));
        let c = corps(&html);
        assert!(c.contains("brm2606.csv"), "fichier d'origine absent");
        assert!(c.contains("brm2607.csv"), "fichier rapproché absent");
        assert!(c.contains("mardi 28 juillet 2026"), "date absente");
        assert!(
            c.contains("9f3c1ab27de4508bb6a1e0f47c25d9836ea15b0c7d42f98e3a6b5c0197de24af"),
            "empreinte absente"
        );
    }

    #[test]
    fn le_resume_compte_les_retraits_par_motif() {
        // Deux natures différentes mènent au même retrait : le résumé doit
        // les distinguer, sinon « 3 retirés » ne dit pas s'il faut aller
        // regarder l'annuaire ou le fichier.
        let mut r = vide();
        r.inchangees = 143;
        r.ecarts = vec![
            ecart_eligibilite("4100000001"),
            ecart_eligibilite("4100000002"),
            ecart_disparu("4100000003"),
        ];
        let html = render(&donnees(&r));
        let c = corps(&html);
        assert!(c.contains("comptes retirés"), "libellé des retraits absent");
        assert!(c.contains(">3<"), "total des retraits absent");
        assert!(c.contains("2</b> non éligibles"), "détail non éligibles absent");
        assert!(c.contains("1</b> disparu"), "détail disparus absent");
        assert!(c.contains("143"), "lignes inchangées absentes");
    }
}
