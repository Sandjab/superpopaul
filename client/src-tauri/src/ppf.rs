//! Ingestion de l'annuaire PPF (export B2B du Portail Public de Facturation) —
//! fonctionnalité CLIENT-ONLY : aucune parité avec cli/popaul.py.
//! Format : CSV `;`, en-tête (BOM toléré), colonnes
//! SIREN;IDENTIFIANT;MOTIF_PRESENCE;UTILISE_PDP_FICTIVE. On conserve tout
//! sauf le SIREN (colonne 0).

use std::io::Read;

/// Une ligne de données PPF retenue (le SIREN de tête est ignoré).
#[derive(Debug, Clone)]
pub struct PpfRow {
    pub identifiant: String,
    pub motif: String,
    pub pdp_fictive: i64, // 0 | 1
}

/// Résultat d'un parse : lignes retenues + nombre de lignes de données lues.
#[derive(Debug)]
pub struct PpfParse {
    pub rows: Vec<PpfRow>,
    pub lines: u64,
}

/// Lit un CSV PPF (`;`, en-tête, BOM toléré) en flux. Colonnes par index :
/// 0 SIREN (ignoré), 1 IDENTIFIANT, 2 MOTIF_PRESENCE, 3 UTILISE_PDP_FICTIVE.
/// `on_progress(lignes_lues)` tous les 100 000 puis une fois en fin de lecture.
/// BLOQUANT (fichier volumineux) : appeler depuis `spawn_blocking`.
pub fn stream_ppf<R: Read>(
    reader: R,
    mut on_progress: impl FnMut(u64),
) -> Result<PpfParse, String> {
    let mut rdr = csv::ReaderBuilder::new()
        .delimiter(b';')
        .has_headers(true)
        .from_reader(reader);
    let mut record = csv::StringRecord::new();
    let mut rows = Vec::new();
    let mut lines: u64 = 0;
    loop {
        match rdr.read_record(&mut record) {
            Ok(true) => {
                lines += 1;
                let identifiant = record.get(1).unwrap_or("").trim();
                let motif = record.get(2).unwrap_or("").trim();
                let pdp_raw = record.get(3).unwrap_or("").trim();
                if identifiant.is_empty() {
                    return Err(format!("ligne {lines} : IDENTIFIANT vide"));
                }
                let pdp_fictive = match pdp_raw {
                    "0" => 0,
                    "1" => 1,
                    other => {
                        return Err(format!(
                            "ligne {lines} : UTILISE_PDP_FICTIVE invalide '{other}' (attendu 0 ou 1)"
                        ))
                    }
                };
                rows.push(PpfRow {
                    identifiant: identifiant.to_string(),
                    motif: motif.to_string(),
                    pdp_fictive,
                });
                if lines.is_multiple_of(100_000) {
                    on_progress(lines);
                }
            }
            Ok(false) => break,
            Err(e) => return Err(format!("lecture CSV PPF : {e}")),
        }
    }
    on_progress(lines);
    Ok(PpfParse { rows, lines })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ppf_point_virgule_et_bom() {
        // BOM UTF-8 + en-tête + 3 lignes (id 005520242 sur deux motifs).
        let csv = "\u{feff}SIREN;IDENTIFIANT;MOTIF_PRESENCE;UTILISE_PDP_FICTIVE\n\
                   005520176;005520176;C;1\n\
                   005520242;005520242;C;1\n\
                   005520242;005520242;V;0\n";
        let mut calls = 0u32;
        let p = stream_ppf(std::io::Cursor::new(csv), |_| calls += 1).unwrap();
        assert_eq!(p.lines, 3);
        assert_eq!(p.rows.len(), 3);
        assert_eq!(p.rows[0].identifiant, "005520176");
        assert_eq!(p.rows[0].motif, "C");
        assert_eq!(p.rows[0].pdp_fictive, 1);
        assert_eq!(p.rows[2].motif, "V");
        assert_eq!(p.rows[2].pdp_fictive, 0);
        assert!(calls >= 1, "on_progress doit être appelé au moins une fois");
    }

    #[test]
    fn parse_ppf_pdp_invalide_est_une_erreur() {
        let csv = "SIREN;IDENTIFIANT;MOTIF_PRESENCE;UTILISE_PDP_FICTIVE\n\
                   005520176;005520176;C;X\n";
        let res = stream_ppf(std::io::Cursor::new(csv), |_| {});
        assert!(res.is_err());
        assert!(res.unwrap_err().contains("UTILISE_PDP_FICTIVE"));
    }

    #[test]
    fn parse_ppf_champ_manquant_remonte_une_erreur() {
        // 3 champs au lieu de 4 (mode strict : nombre de champs incohérent).
        let csv = "SIREN;IDENTIFIANT;MOTIF_PRESENCE;UTILISE_PDP_FICTIVE\n\
                   005520176;005520176;C\n";
        let res = stream_ppf(std::io::Cursor::new(csv), |_| {});
        assert!(res.is_err(), "un CSV malformé doit remonter une Err");
    }

    #[test]
    fn parse_ppf_identifiant_vide_est_une_erreur() {
        let csv = "SIREN;IDENTIFIANT;MOTIF_PRESENCE;UTILISE_PDP_FICTIVE\n\
                   005520176;;C;1\n";
        let res = stream_ppf(std::io::Cursor::new(csv), |_| {});
        assert!(res.is_err());
        assert!(res.unwrap_err().contains("IDENTIFIANT"));
    }

    #[test]
    fn parse_ppf_entete_seule_ne_produit_rien() {
        let p = stream_ppf(
            std::io::Cursor::new("SIREN;IDENTIFIANT;MOTIF_PRESENCE;UTILISE_PDP_FICTIVE\n"),
            |_| {},
        )
        .unwrap();
        assert_eq!(p.lines, 0);
        assert!(p.rows.is_empty());
    }
}
