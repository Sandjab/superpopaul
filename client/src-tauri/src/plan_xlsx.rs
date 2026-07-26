//! Classeur XLSX du périmètre du plan — **tous** les comptes du fichier
//! d'entrée, au plan ou non.
//!
//! La composition du tableau (`lignes`) est PURE et testable ; l'écriture
//! (`ecrire`) n'a aucune logique métier. Même séparation que `charge` et
//! `charts` pour le rapport.

use crate::plan::{LigneEntree, LignePlan};
use std::path::Path;

/// Rapport d'un compte au plan. Trois états, pas deux : un compte **retiré**
/// n'est pas un compte jamais placé — ce sont deux décisions opposées.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Appartenance {
    Oui,
    Retire,
    Non,
}

impl Appartenance {
    /// Libellé de la colonne « Dans le plan ».
    pub fn libelle(self) -> &'static str {
        match self {
            Appartenance::Oui => "oui",
            Appartenance::Retire => "retiré",
            Appartenance::Non => "non",
        }
    }
}

/// Une ligne du classeur : un compte du fichier d'entrée.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LigneExport {
    /// Vide si le compte n'a jamais été placé ; **conservé** s'il a été retiré.
    pub run: String,
    pub cf: String,
    /// Le jour de cycle tel qu'il figurait dans le fichier, même illisible.
    pub jj: String,
    /// Adressage sous forme nue — **sans son ICD**, comme en base et dans le
    /// CSV de sortie — quand le schéma s'y prête.
    pub adressage: String,
    pub raison_sociale: String,
    pub ctc_status: String,
    pub ppf_usable: bool,
    pub appartenance: Appartenance,
}

/// Compose le tableau : une ligne par compte du fichier d'entrée, dans l'ordre
/// où il les fournit. PURE — ni disque, ni format.
pub fn lignes(entrees: &[LigneEntree], plan: &[LignePlan]) -> Vec<LigneExport> {
    let par_cf: std::collections::HashMap<&str, &LignePlan> =
        plan.iter().map(|l| (l.cf.as_str(), l)).collect();

    entrees
        .iter()
        .map(|e| {
            let (run, appartenance) = match par_cf.get(e.cf.as_str()) {
                Some(l) if l.retiree() => (l.run_num.clone(), Appartenance::Retire),
                Some(l) => (l.run_num.clone(), Appartenance::Oui),
                None => (String::new(), Appartenance::Non),
            };
            LigneExport {
                run,
                cf: e.cf.clone(),
                jj: e.jj_brut.clone(),
                // Forme stockée, sans ICD : celle qu'écrivent déjà le CSV de
                // sortie et la base, donc le classeur se recoupe avec les
                // autres exports et l'annuaire PPF sans retraitement.
                adressage: crate::directory::parse_0225_value(&e.participant)
                    .unwrap_or_else(|| e.participant.clone()),
                raison_sociale: e.raison_sociale.clone(),
                ctc_status: e.ctc_status.clone(),
                ppf_usable: e.ppf_usable,
                appartenance,
            }
        })
        .collect()
}

/// En-têtes du classeur, dans l'ordre des colonnes.
const ENTETES: [&str; 8] = [
    "N° de run",
    "N° de CF",
    "JJ",
    "Adressage",
    "Raison sociale",
    "Statut CTC",
    "PPF usable",
    "Dans le plan",
];

/// Écrit le classeur : en-tête figé et filtres automatiques, qui sont l'usage
/// attendu du fichier. Aucune logique métier ici.
///
/// **Toutes les valeurs sont écrites en texte**, y compris le JJ : le fichier
/// documente ce que contenait le CSV, et Excel ne doit réinterpréter aucun
/// identifiant en nombre ni en notation scientifique.
pub fn ecrire(chemin: &Path, lignes: &[LigneExport]) -> Result<(), String> {
    use rust_xlsxwriter::{Format, Workbook};

    let mut classeur = Workbook::new();
    let feuille = classeur.add_worksheet();
    feuille
        .set_name("Comptes")
        .map_err(|e| format!("classeur : {e}"))?;

    let gras = Format::new().set_bold();
    for (i, titre) in ENTETES.iter().enumerate() {
        feuille
            .write_with_format(0, i as u16, *titre, &gras)
            .map_err(|e| format!("en-tête : {e}"))?;
    }

    for (n, l) in lignes.iter().enumerate() {
        let r = n as u32 + 1;
        let cellules: [&str; 8] = [
            &l.run,
            &l.cf,
            &l.jj,
            &l.adressage,
            &l.raison_sociale,
            &l.ctc_status,
            if l.ppf_usable { "true" } else { "false" },
            l.appartenance.libelle(),
        ];
        for (c, v) in cellules.iter().enumerate() {
            feuille
                .write(r, c as u16, *v)
                .map_err(|e| format!("ligne {r} : {e}"))?;
        }
    }

    // En-tête figé et filtres : sans eux, le fichier n'est qu'un CSV déguisé.
    feuille.set_freeze_panes(1, 0).map_err(|e| format!("volets : {e}"))?;
    feuille
        .autofilter(0, 0, lignes.len() as u32, ENTETES.len() as u16 - 1)
        .map_err(|e| format!("filtres : {e}"))?;
    for (i, largeur) in [12.0, 16.0, 6.0, 30.0, 38.0, 12.0, 12.0, 14.0].iter().enumerate() {
        feuille
            .set_column_width(i as u16, *largeur)
            .map_err(|e| format!("largeur : {e}"))?;
    }

    classeur
        .save(chemin)
        .map_err(|e| format!("écriture du classeur : {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jour(iso: &str) -> chrono::NaiveDate {
        chrono::NaiveDate::parse_from_str(iso, "%Y-%m-%d").unwrap()
    }

    fn entree(cf: &str, jj: &str, ctc: &str) -> LigneEntree {
        LigneEntree {
            cf: cf.into(),
            participant: "iso6523-actorid-upis::0225:12345678900012".into(),
            jj_brut: jj.into(),
            raison_sociale: "ACME".into(),
            pa: "Cegedim".into(),
            resolu: true,
            ctc_ready: ctc == "ready",
            ctc_status: ctc.into(),
            ppf_usable: true,
            in_directory: true,
            resolved_at: 0,
        }
    }

    /// Décode le classeur en cellules texte, en-tête compris, huit colonnes par
    /// ligne. Les valeurs d'un `.xlsx` ne vivent pas dans la feuille mais dans
    /// `xl/sharedStrings.xml` : `sheet1.xml` ne porte que des index, et une
    /// cellule vide n'y est pas écrite du tout. Sans ce décodage, un test ne
    /// peut pas voir qu'une colonne a changé de place.
    fn cellules(chemin: &Path) -> Vec<Vec<String>> {
        use std::io::Read;
        let fichier = std::fs::File::open(chemin).expect("ouverture");
        let mut archive = zip::ZipArchive::new(fichier).expect("archive illisible");
        let mut lire = |nom: &str| {
            let mut s = String::new();
            archive.by_name(nom).expect(nom).read_to_string(&mut s).expect("lecture");
            s
        };
        let table = lire("xl/sharedStrings.xml");
        let feuille = lire("xl/worksheets/sheet1.xml");

        let chaines: Vec<&str> = table
            .split("<si><t")
            .skip(1)
            .map(|bloc| {
                let apres = bloc.split_once('>').expect("<t> mal formé").1;
                apres.split_once("</t>").expect("</t> absent").0
            })
            .collect();

        let corps = feuille
            .split_once("<sheetData>")
            .expect("sheetData absent")
            .1
            .split_once("</sheetData>")
            .expect("sheetData non refermé")
            .0;
        corps
            .split("<row ")
            .skip(1)
            .map(|ligne| {
                let mut cols = vec![String::new(); ENTETES.len()];
                for bloc in ligne.split("<c r=\"").skip(1) {
                    let (reference, reste) = bloc.split_once('"').expect("référence de cellule");
                    // Huit colonnes : la référence tient sur une lettre (A..H).
                    let col = reference.as_bytes()[0] as usize - b'A' as usize;
                    let index: usize = reste
                        .split_once("<v>")
                        .expect("cellule sans valeur")
                        .1
                        .split_once("</v>")
                        .expect("valeur non refermée")
                        .0
                        .parse()
                        .expect("index de chaîne partagée");
                    cols[col] = chaines[index].to_string();
                }
                cols
            })
            .collect()
    }

    fn ligne_plan(cf: &str, run: &str) -> LignePlan {
        LignePlan {
            cf: cf.into(),
            participant: "0225:12345678900012".into(),
            jj: 5,
            raison_sociale: "ACME".into(),
            pa: "Cegedim".into(),
            mep_id: 1,
            mep_date: jour("2026-08-01"),
            run_num: run.into(),
            run_date: jour("2026-08-11"),
            origine: crate::plan::Origine::Auto,
            in_directory: true,
            resolved_at: 0,
            planned_at: 0,
            retire: None,
        }
    }

    #[test]
    fn export_couvre_toutes_les_lignes_du_fichier() {
        // Un compte hors du pool (non résolu, sans plateforme) figure quand même
        // au tableau : « l'intégralité des comptes du fichier ».
        let mut hors_pool = entree("CF2", "5", "");
        hors_pool.resolu = false;
        hors_pool.pa = String::new();
        let entrees = vec![entree("CF1", "5", "ready"), hors_pool];
        let plan = vec![ligne_plan("CF1", "R1")];
        let out = lignes(&entrees, &plan);
        assert_eq!(out.len(), 2);
        let cf2 = out.iter().find(|l| l.cf == "CF2").expect("CF2 absent");
        assert_eq!(cf2.appartenance, Appartenance::Non);
        assert_eq!(cf2.run, "", "jamais placé : pas de run");
    }

    #[test]
    fn un_compte_retire_conserve_son_run_et_vaut_retire() {
        let mut retiree = ligne_plan("CF1", "R1");
        retiree.retire = Some(crate::plan::Retrait { le: 1, motif: "clôturé".into() });
        let out = lignes(&[entree("CF1", "5", "ready")], &[retiree]);
        assert_eq!(out[0].appartenance, Appartenance::Retire);
        assert_eq!(out[0].run, "R1", "le run est la trace de ce dont on l'a sorti");
    }

    #[test]
    fn un_compte_au_plan_porte_son_run() {
        let out = lignes(&[entree("CF1", "5", "ready")], &[ligne_plan("CF1", "R1")]);
        assert_eq!(out[0].appartenance, Appartenance::Oui);
        assert_eq!(out[0].run, "R1");
    }

    #[test]
    fn l_adressage_sort_sous_forme_nue() {
        let out = lignes(&[entree("CF1", "5", "ready")], &[]);
        assert_eq!(out[0].adressage, "12345678900012", "forme canonique non réduite");
    }

    #[test]
    fn un_adressage_non_0225_sort_sous_forme_canonique() {
        // Repli : pas de valeur nue à extraire d'un autre schéma.
        let mut e = entree("CF1", "5", "ready");
        e.participant = "iso6523-actorid-upis::0088:7300010000001".into();
        let out = lignes(&[e], &[]);
        assert_eq!(out[0].adressage, "iso6523-actorid-upis::0088:7300010000001");
    }

    #[test]
    fn le_statut_ctc_nest_pas_aplati() {
        let out = lignes(&[entree("CF1", "5", "later")], &[]);
        assert_eq!(out[0].ctc_status, "later");
    }

    #[test]
    fn le_jour_de_cycle_illisible_est_rendu_tel_quel() {
        // Le classeur documente le fichier, il ne le corrige pas.
        let out = lignes(&[entree("CF1", "zzz", "")], &[]);
        assert_eq!(out[0].jj, "zzz");
    }

    #[test]
    fn ecrire_produit_un_fichier_lisible() {
        // On ne relit pas le xlsx (pas de lecteur dans les dépendances) : on
        // vérifie qu'un fichier non vide est produit et qu'il porte la signature
        // d'un conteneur ZIP, ce qu'est un .xlsx.
        let dir = std::env::temp_dir().join("popaul_test_xlsx");
        std::fs::create_dir_all(&dir).unwrap();
        let chemin = dir.join("t.xlsx");
        let l = lignes(&[entree("CF1", "5", "ready")], &[ligne_plan("CF1", "R1")]);
        ecrire(&chemin, &l).expect("écriture");
        let octets = std::fs::read(&chemin).expect("relecture");
        assert!(octets.len() > 1000, "classeur suspect : {} octets", octets.len());
        assert_eq!(&octets[..2], b"PK", "un .xlsx est un conteneur ZIP");
        std::fs::remove_file(&chemin).ok();
    }

    #[test]
    fn le_classeur_porte_ses_filtres_et_son_volet_fige() {
        // Ce sont eux qui justifient un vrai .xlsx plutôt qu'un CSV : sans test,
        // retirer ces deux lignes ne casserait rien jusqu'à ce que quelqu'un ouvre
        // le fichier. Les entrées du ZIP sont deflatées : il faut décompresser.
        let dir = std::env::temp_dir().join("popaul_test_xlsx_filtres");
        std::fs::create_dir_all(&dir).unwrap();
        let chemin = dir.join("f.xlsx");
        let l = lignes(&[entree("CF1", "5", "ready")], &[ligne_plan("CF1", "R1")]);
        ecrire(&chemin, &l).expect("écriture");

        let fichier = std::fs::File::open(&chemin).expect("ouverture");
        let mut archive = zip::ZipArchive::new(fichier).expect("archive illisible");
        let mut xml = String::new();
        {
            use std::io::Read;
            archive
                .by_name("xl/worksheets/sheet1.xml")
                .expect("feuille absente")
                .read_to_string(&mut xml)
                .expect("lecture");
        }
        assert!(xml.contains("<autoFilter"), "filtres automatiques absents : {xml}");
        assert!(xml.contains("state=\"frozen\""), "volet figé absent : {xml}");
        std::fs::remove_file(&chemin).ok();
    }

    #[test]
    fn le_classeur_ecrit_les_valeurs_de_chaque_ligne() {
        // Sans cette vérification, `ecrire` peut produire un classeur à en-tête
        // seul, ou permuter deux colonnes, sans qu'aucun test ne bronche : les
        // autres n'assertent que la taille du fichier et sa signature ZIP.
        let dir = std::env::temp_dir().join("popaul_test_xlsx_valeurs");
        std::fs::create_dir_all(&dir).unwrap();
        let chemin = dir.join("v.xlsx");
        let mut retiree = ligne_plan("CF2", "R2");
        retiree.retire = Some(crate::plan::Retrait { le: 1, motif: "clôturé".into() });
        let l = lignes(
            &[entree("CF1", "5", "ready"), entree("CF2", "12", "later")],
            &[ligne_plan("CF1", "R1"), retiree],
        );
        ecrire(&chemin, &l).expect("écriture");

        let table = cellules(&chemin);
        assert_eq!(table.len(), 3, "l'en-tête et les deux comptes : {table:?}");
        assert_eq!(table[0], ENTETES, "colonnes dans l'ordre annoncé");
        assert_eq!(
            table[1],
            ["R1", "CF1", "5", "12345678900012", "ACME", "ready", "true", "oui"]
        );
        assert_eq!(
            table[2],
            ["R2", "CF2", "12", "12345678900012", "ACME", "later", "true", "retiré"]
        );
        std::fs::remove_file(&chemin).ok();
    }

    #[test]
    fn ecrire_un_tableau_vide_ne_panique_pas() {
        let dir = std::env::temp_dir().join("popaul_test_xlsx");
        std::fs::create_dir_all(&dir).unwrap();
        let chemin = dir.join("vide.xlsx");
        ecrire(&chemin, &[]).expect("un plan sans compte reste un fichier valide");
        std::fs::remove_file(&chemin).ok();
    }
}
