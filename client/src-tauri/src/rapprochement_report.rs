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
    html.push_str("</section>\n");

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
            annuaire_incomplet: None,
        }
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
