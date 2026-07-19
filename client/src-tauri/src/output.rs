use crate::config::{ColumnSpec, OutputConfig, OutputEncoding, OutputSeparator, PeppolField};
use crate::csv_io::CsvMeta;
use crate::directory::parse_0225_value;
use crate::pid::canonical;
use crate::store::{PpfFlags, Resolution};
use encoding_rs_io::DecodeReaderBytesBuilder;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};

/// État CTC calculé au moment de l'export — jamais figé en base : les dates
/// stockées suffisent, un « later » bascule seul en « ready » le jour venu.
/// Vide sans extension déclarée (pas d'état à calculer).
pub(crate) fn ctc_status(r: &crate::store::Resolution, now: chrono::DateTime<chrono::Utc>) -> &'static str {
    if r.extended_ctc_fr != Some(true) {
        return "";
    }
    match crate::ctc::state(r.ctc_activation.as_deref(), r.ctc_expiration.as_deref(), now) {
        crate::ctc::CtcState::Ready => "ready",
        crate::ctc::CtcState::Later => "later",
        crate::ctc::CtcState::Expired => "expired",
    }
}

fn fmt_bool(b: Option<bool>) -> &'static str {
    match b {
        Some(true) => "true",
        Some(false) => "false",
        None => "",
    }
}

pub fn field_name(f: PeppolField) -> &'static str {
    match f {
        PeppolField::InPeppol => "in_peppol",
        PeppolField::PaCode => "pa_code",
        PeppolField::PaName => "pa_name",
        PeppolField::PaCountry => "pa_country",
        PeppolField::UblExtended => "ubl_extended",
        PeppolField::CtcActivation => "ctc_activation",
        PeppolField::CtcExpiration => "ctc_expiration",
        PeppolField::CtcStatus => "ctc_status",
        PeppolField::InDirectory => "in_directory",
        PeppolField::AnnuairePpf => "annuaire_ppf",
        PeppolField::PpfActive => "ppf_active",
        PeppolField::PdpDefinie => "pdp_definie",
        PeppolField::PpfUsable => "ppf_usable",
    }
}

/// Insère `_<stamp>` avant l'extension.
/// Nom du fichier de sortie : `<nom de l'entrée><suffixe>.csv`
/// (clients.csv + « _enrichi » → clients_enrichi.csv).
pub fn out_file_name(input_path: &Path, suffix: &str) -> String {
    let stem = input_path
        .file_stem()
        .and_then(|x| x.to_str())
        .unwrap_or("sortie");
    format!("{stem}{suffix}.csv")
}

pub fn with_stamp(path: &Path, stamp: Option<&str>) -> PathBuf {
    match stamp {
        None => path.to_path_buf(),
        Some(s) => {
            let stem = path
                .file_stem()
                .and_then(|x| x.to_str())
                .unwrap_or("sortie");
            let ext = path.extension().and_then(|x| x.to_str()).unwrap_or("csv");
            path.with_file_name(format!("{stem}_{s}.{ext}"))
        }
    }
}

/// Écrit le CSV de sortie : une ligne par ligne d'entrée, colonnes selon le
/// mapping, infos Peppol lues dans `resolutions` (la base). Encodage et
/// séparateur selon `output` (`Auto` = séparateur sniffé sur l'entrée) ;
/// `output.dir`/`suffix`/`timestamp_suffix` sont ignorés ici — l'appelant
/// fournit `out_path` (résolu relativement au YAML) et `stamp` déjà calculés.
///
/// Écriture atomique (comme `config::save`) : tout passe par `<final>.tmp`
/// dans le même répertoire, renommé vers la cible seulement après le flush —
/// un disque plein ou une permission refusée en cours de route ne laisse
/// jamais un CSV d'apparence valide mais tronqué.
// 9 paramètres : chacun porte une donnée distincte de l'export (entrée,
// méta, colonne PID, config de sortie, base, annuaire Peppol, annuaire PPF,
// chemin de sortie, horodatage) — pas un signe de mauvais découpage.
#[allow(clippy::too_many_arguments)]
pub fn generate(
    input_path: &Path,
    meta: &CsvMeta,
    pid_column: &str,
    output: &OutputConfig,
    resolutions: &HashMap<String, Resolution>,
    directory: Option<&HashSet<String>>,
    ppf: Option<&HashMap<String, PpfFlags>>,
    out_path: &Path,
    stamp: Option<&str>,
) -> Result<PathBuf, String> {
    // La garde n'est plus dans Config::validate (une config sans colonnes est
    // légitime avant le choix du fichier) : c'est ici, au moment d'écrire,
    // qu'un mapping vide doit échouer fort plutôt que produire un CSV sans
    // en-têtes.
    if output.columns.is_empty() {
        return Err("aucune colonne de sortie — reconfigure l'étape Colonnes".into());
    }
    let final_path = with_stamp(out_path, stamp);
    // Suffixe vide + même répertoire + pas de date/heure : la sortie porterait
    // le nom de l'entrée. Comparaison lexicale (les deux chemins sortent de la
    // même résolution dans commands.rs) — garde-fou, pas une preuve d'unicité.
    if final_path == input_path {
        return Err(format!(
            "la sortie {} écraserait le fichier d'entrée — change le suffixe, \
             le répertoire ou active la date/heure",
            final_path.display()
        ));
    }
    let mut tmp_os = final_path.clone().into_os_string();
    tmp_os.push(".tmp");
    let tmp_path = PathBuf::from(tmp_os);

    if let Err(e) = write_output(input_path, meta, pid_column, output, resolutions, directory, ppf, &tmp_path) {
        let _ = std::fs::remove_file(&tmp_path); // nettoyage best-effort
        return Err(e);
    }
    std::fs::rename(&tmp_path, &final_path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp_path);
        format!("renommage {tmp_path:?} → {final_path:?} : {e}")
    })?;
    Ok(final_path)
}

/// Ré-encode à la volée le flux UTF-8 produit par `csv::Writer` en
/// windows-1252 ; les caractères non représentables deviennent « ? »
/// (assumé, spec sortie). `carry` conserve une éventuelle séquence UTF-8
/// coupée en fin de chunk pour la recoller au chunk suivant.
struct Windows1252Writer<W: Write> {
    inner: W,
    carry: Vec<u8>,
}

impl<W: Write> Write for Windows1252Writer<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.carry.extend_from_slice(buf);
        let valid = match std::str::from_utf8(&self.carry) {
            Ok(s) => s.len(),
            Err(e) => e.valid_up_to(),
        };
        let mut rest = std::str::from_utf8(&self.carry[..valid]).unwrap();
        let mut encoder = encoding_rs::WINDOWS_1252.new_encoder();
        let mut out = [0u8; 4096];
        while !rest.is_empty() {
            let (result, read, written) =
                encoder.encode_from_utf8_without_replacement(rest, &mut out, false);
            self.inner.write_all(&out[..written])?;
            rest = &rest[read..];
            if let encoding_rs::EncoderResult::Unmappable(_) = result {
                self.inner.write_all(b"?")?;
            }
        }
        self.carry.drain(..valid);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        // Au flush final, csv::Writer a produit de l'UTF-8 complet : un reste
        // signalerait une séquence tronquée — fail loud plutôt que corrompre.
        if !self.carry.is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "séquence UTF-8 incomplète en fin de flux",
            ));
        }
        self.inner.flush()
    }
}

/// Lit l'entrée et écrit toutes les lignes dans `tmp_path` (flush inclus).
// Même arité que `generate` (moins out_path/stamp, plus tmp_path) — même
// rationale : chaque paramètre porte une donnée distincte de l'export.
#[allow(clippy::too_many_arguments)]
fn write_output(
    input_path: &Path,
    meta: &CsvMeta,
    pid_column: &str,
    output: &OutputConfig,
    resolutions: &HashMap<String, Resolution>,
    directory: Option<&HashSet<String>>,
    ppf: Option<&HashMap<String, PpfFlags>>,
    tmp_path: &Path,
) -> Result<(), String> {
    let columns = &output.columns;
    let f = File::open(input_path).map_err(|e| format!("ouverture {input_path:?} : {e}"))?;
    let enc = if meta.encoding == "utf-8" {
        encoding_rs::UTF_8
    } else {
        encoding_rs::WINDOWS_1252
    };
    let decoded: Box<dyn Read> = Box::new(
        DecodeReaderBytesBuilder::new()
            .encoding(Some(enc))
            .bom_sniffing(true)
            .build(f),
    );
    let mut rdr = csv::ReaderBuilder::new()
        .delimiter(meta.delimiter)
        .flexible(true)
        .from_reader(decoded);
    let headers = rdr.headers().map_err(|e| e.to_string())?.clone();
    let pid_idx = headers
        .iter()
        .position(|h| h == pid_column)
        .ok_or_else(|| format!("Colonne '{pid_column}' absente de l'entête"))?;
    // Index des colonnes d'entrée du mapping, résolus une fois.
    let col_idx: Vec<Option<usize>> = columns
        .iter()
        .map(|c| match c {
            ColumnSpec::Input { name } => headers.iter().position(|h| h == name),
            ColumnSpec::Peppol { .. } => None,
        })
        .collect();
    for (c, idx) in columns.iter().zip(&col_idx) {
        if let (ColumnSpec::Input { name }, None) = (c, idx) {
            return Err(format!("Colonne d'entrée '{name}' absente de l'entête"));
        }
    }

    let out_delim = match output.separator {
        OutputSeparator::Auto => meta.delimiter,
        OutputSeparator::Semicolon => b';',
        OutputSeparator::Comma => b',',
        OutputSeparator::Pipe => b'|',
        OutputSeparator::Tab => b'\t',
    };
    let out = File::create(tmp_path).map_err(|e| format!("écriture {tmp_path:?} : {e}"))?;
    let mut out = BufWriter::new(out);
    // BOM UTF-8 (défaut) : le public cible ouvre le CSV par double-clic dans
    // Excel FR, qui casse les accents sans lui.
    if output.encoding == OutputEncoding::Utf8Bom {
        out.write_all(b"\xEF\xBB\xBF")
            .map_err(|e| format!("écriture {tmp_path:?} : {e}"))?;
    }
    let sink: Box<dyn Write> = match output.encoding {
        OutputEncoding::Windows1252 => Box::new(Windows1252Writer {
            inner: out,
            carry: Vec::new(),
        }),
        OutputEncoding::Utf8Bom | OutputEncoding::Utf8 => Box::new(out),
    };
    let mut wtr = csv::WriterBuilder::new()
        .delimiter(out_delim)
        .from_writer(sink);
    // Entête de sortie.
    let out_headers: Vec<&str> = columns
        .iter()
        .map(|c| match c {
            ColumnSpec::Input { name } => name.as_str(),
            ColumnSpec::Peppol { field } => field_name(*field),
        })
        .collect();
    wtr.write_record(&out_headers)
        .map_err(|e| format!("écriture {tmp_path:?} ligne 1 : {e}"))?;

    // Un seul « maintenant » pour tout l'export : l'état CTC de chaque ligne
    // est calculé à cet instant (cohérence du fichier).
    let now = chrono::Utc::now();

    // Numérotation : l'entête est la ligne 1, le premier enregistrement la 2.
    for (line, rec) in (2u64..).zip(rdr.records()) {
        let rec = rec.map_err(|e| format!("lecture {input_path:?} ligne {line} : {e}"))?;
        let raw_pid = rec.get(pid_idx).unwrap_or("");
        let cpid = canonical(raw_pid);
        let res = resolutions.get(&cpid);
        // Valeur 0225 (partagée annuaire Peppol + PPF), calculée une fois.
        let v0225 = parse_0225_value(&cpid);
        // Présence annuaire Peppol : hors du gate `res` (un déclaré non
        // provisionné n'a pas de Resolution mais doit ressortir "true").
        let in_dir: &str = match directory {
            None => "",
            Some(set) => match &v0225 {
                Some(v) if set.contains(v) => "true",
                Some(_) => "false",
                None => "",
            },
        };
        // Drapeaux PPF (hors gate `res`, comme in_dir). None = annuaire vide OU
        // non-0225 → cellule vide ; Some(défaut) = 0225 absent de l'annuaire →
        // tout "false".
        let ppf_flags: Option<PpfFlags> = match (ppf, &v0225) {
            (Some(map), Some(v)) => Some(map.get(v).copied().unwrap_or_default()),
            _ => None,
        };
        let ppf_ann = match ppf_flags { Some(f) => fmt_bool(Some(f.in_ppf)), None => "" };
        let ppf_act = match ppf_flags { Some(f) => fmt_bool(Some(f.active)), None => "" };
        let ppf_pdp = match ppf_flags { Some(f) => fmt_bool(Some(f.pdp_definie)), None => "" };
        let ppf_use = match ppf_flags { Some(f) => fmt_bool(Some(f.usable)), None => "" };
        // Zéro allocation par cellule : la ligne est un Vec<&str>.
        let row: Vec<&str> = columns
            .iter()
            .zip(&col_idx)
            .map(|(c, idx)| match c {
                ColumnSpec::Input { .. } => rec.get(idx.unwrap()).unwrap_or(""),
                ColumnSpec::Peppol { field: PeppolField::InDirectory } => in_dir,
                ColumnSpec::Peppol { field: PeppolField::AnnuairePpf } => ppf_ann,
                ColumnSpec::Peppol { field: PeppolField::PpfActive } => ppf_act,
                ColumnSpec::Peppol { field: PeppolField::PdpDefinie } => ppf_pdp,
                ColumnSpec::Peppol { field: PeppolField::PpfUsable } => ppf_use,
                ColumnSpec::Peppol { field } => match res {
                    None => "",
                    Some(r) => match field {
                        PeppolField::InPeppol => fmt_bool(r.exists_in_peppol),
                        PeppolField::PaCode => r.pa_code.as_deref().unwrap_or(""),
                        PeppolField::PaName => r.pa_name.as_deref().unwrap_or(""),
                        PeppolField::PaCountry => r.pa_country.as_deref().unwrap_or(""),
                        PeppolField::UblExtended => fmt_bool(r.extended_ctc_fr),
                        PeppolField::CtcActivation => r.ctc_activation.as_deref().unwrap_or(""),
                        PeppolField::CtcExpiration => r.ctc_expiration.as_deref().unwrap_or(""),
                        PeppolField::CtcStatus => ctc_status(r, now),
                        PeppolField::InDirectory => unreachable!("traité par le bras dédié ci-dessus"),
                        PeppolField::AnnuairePpf
                        | PeppolField::PpfActive
                        | PeppolField::PdpDefinie
                        | PeppolField::PpfUsable => {
                            unreachable!("champs PPF traités par les bras dédiés ci-dessus")
                        }
                    },
                },
            })
            .collect();
        wtr.write_record(&row)
            .map_err(|e| format!("écriture {tmp_path:?} ligne {line} : {e}"))?;
    }
    // Flush complet (csv → [1252] → BufWriter → fichier) avant le rename
    // atomique. into_inner vide le tampon csv, flush propage jusqu'au fichier.
    wtr.into_inner()
        .map_err(|e| format!("écriture {tmp_path:?} : {e}"))?
        .flush()
        .map_err(|e| format!("écriture {tmp_path:?} : {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ColumnSpec, OutputEncoding, OutputSeparator, PeppolField};
    use crate::csv_io::CsvMeta;
    use crate::store::{PpfFlags, Resolution};
    use std::collections::HashMap;
    use std::io::Write;

    /// OutputConfig de test : défauts historiques (UTF-8+BOM, séparateur de
    /// l'entrée). `dir`/`suffix`/`timestamp_suffix` sont ignorés par generate
    /// (résolus par l'appelant).
    fn out_cfg(columns: Vec<ColumnSpec>) -> OutputConfig {
        OutputConfig {
            dir: String::new(),
            suffix: "_enrichi".into(),
            path: String::new(),
            timestamp_suffix: false,
            encoding: OutputEncoding::Utf8Bom,
            separator: OutputSeparator::Auto,
            columns,
        }
    }

    fn resolutions() -> HashMap<String, Resolution> {
        let mut m = HashMap::new();
        m.insert(
            "iso6523-actorid-upis::0009:1".to_string(),
            Resolution {
                participant: "iso6523-actorid-upis::0009:1".into(),
                exists_in_peppol: Some(true),
                pa_code: Some("PA0042".into()),
                pa_name: Some("ACME PA".into()),
                pa_country: Some("FR".into()),
                extended_ctc_fr: Some(false),
                api_status: "ok".into(),
                resolved_at: 0,
                note: None,
                ctc_activation: None,
                ctc_expiration: None,
            },
        );
        m
    }

    #[test]
    fn out_file_name_derive_du_nom_de_l_entree() {
        use std::path::Path;
        assert_eq!(out_file_name(Path::new("/x/clients.csv"), "_enrichi"), "clients_enrichi.csv");
        // L'extension de sortie est toujours .csv, même pour une entrée .txt.
        assert_eq!(out_file_name(Path::new("data.txt"), "_peppol"), "data_peppol.csv");
        assert_eq!(out_file_name(Path::new("/x/clients.csv"), ""), "clients.csv");
    }

    #[test]
    fn colonnes_ctc_dates_et_etat_calcule_a_l_export() {
        // L'état n'est PAS en base : il se recalcule au moment de l'export à
        // partir des dates stockées — un « later » bascule seul en « ready »
        // le jour venu, sans re-résolution. Dates extrêmes : le test ne
        // dépend pas du jour. Sans extension : colonnes vides.
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("in.csv");
        std::fs::File::create(&input)
            .unwrap()
            .write_all(b"siren\n0009:pret\n0009:plustard\n0009:expire\n0009:sansext\n")
            .unwrap();
        let out = dir.path().join("out.csv");
        let mk = |ext: bool, act: Option<&str>, exp: Option<&str>| Resolution {
            participant: String::new(),
            exists_in_peppol: Some(true),
            pa_code: None,
            pa_name: None,
            pa_country: None,
            extended_ctc_fr: Some(ext),
            api_status: "ok".into(),
            resolved_at: 0,
            note: None,
            ctc_activation: act.map(Into::into),
            ctc_expiration: exp.map(Into::into),
        };
        let m = HashMap::from([
            (canonical("0009:pret"), mk(true, Some("2000-01-01"), None)),
            (canonical("0009:plustard"), mk(true, Some("2999-01-01T00:00:00Z"), None)),
            (canonical("0009:expire"), mk(true, Some("2000-01-01"), Some("2001-01-01"))),
            (canonical("0009:sansext"), mk(false, None, None)),
        ]);
        let cols = vec![
            ColumnSpec::Peppol { field: PeppolField::CtcActivation },
            ColumnSpec::Peppol { field: PeppolField::CtcExpiration },
            ColumnSpec::Peppol { field: PeppolField::CtcStatus },
        ];
        let meta = CsvMeta { delimiter: b';', encoding: "utf-8" };
        let written =
            generate(&input, &meta, "siren", &out_cfg(cols), &m, None, None, &out, None).unwrap();
        let content = std::fs::read_to_string(&written).unwrap();
        let lines: Vec<&str> = content.trim_start_matches('\u{feff}').lines().collect();
        assert_eq!(lines[0], "ctc_activation;ctc_expiration;ctc_status");
        assert_eq!(lines[1], "2000-01-01;;ready", "actif : dates brutes + ready");
        assert_eq!(lines[2], "2999-01-01T00:00:00Z;;later");
        assert_eq!(lines[3], "2000-01-01;2001-01-01;expired");
        assert_eq!(lines[4], ";;", "sans extension : pas d'état CTC");
    }

    #[test]
    fn generate_refuse_un_mapping_sans_colonnes() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("in.csv");
        std::fs::File::create(&input)
            .unwrap()
            .write_all(b"siren\n0009:1\n")
            .unwrap();
        let out = dir.path().join("out.csv");
        let meta = CsvMeta { delimiter: b';', encoding: "utf-8" };
        let err = generate(&input, &meta, "siren", &out_cfg(vec![]), &resolutions(), None, None, &out, None)
            .unwrap_err();
        assert!(err.contains("colonne"), "{err}");
        assert!(!out.exists());
    }

    #[test]
    fn generate_refuse_d_ecraser_le_fichier_d_entree() {
        // Suffixe vide + même répertoire + pas de date/heure : sans cette
        // garde, la sortie détruirait silencieusement le fichier source.
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("in.csv");
        std::fs::File::create(&input)
            .unwrap()
            .write_all(b"siren\n0009:1\n")
            .unwrap();
        let meta = CsvMeta { delimiter: b';', encoding: "utf-8" };
        let cols = vec![ColumnSpec::Input { name: "siren".into() }];
        let err = generate(&input, &meta, "siren", &out_cfg(cols), &resolutions(), None, None, &input, None)
            .unwrap_err();
        assert!(err.contains("écraserait"), "{err}");
        // Le fichier d'entrée est intact.
        assert_eq!(std::fs::read_to_string(&input).unwrap(), "siren\n0009:1\n");
    }

    #[test]
    fn sortie_une_ligne_par_ligne_d_entree_meme_pid_duplique() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("in.csv");
        std::fs::File::create(&input)
            .unwrap()
            .write_all(b"siren;nom\n0009:1;ACME\n0009:2;GLOBEX\n0009:1;ACME BIS\n")
            .unwrap();
        let out = dir.path().join("out.csv");
        let cols = vec![
            ColumnSpec::Input { name: "nom".into() },
            ColumnSpec::Peppol {
                field: PeppolField::InPeppol,
            },
            ColumnSpec::Peppol {
                field: PeppolField::PaCode,
            },
        ];
        let meta = CsvMeta {
            delimiter: b';',
            encoding: "utf-8",
        };
        let written = generate(
            &input,
            &meta,
            "siren",
            &out_cfg(cols),
            &resolutions(),
            None,
            None,
            &out,
            None,
        )
        .unwrap();
        let content = std::fs::read_to_string(&written).unwrap();
        // BOM retiré ici, testé dédié dans generate_convertit_windows1252_en_utf8_avec_bom.
        let content = content.trim_start_matches('\u{feff}');
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 4); // entête + 3 lignes (autant que l'entrée)
        assert_eq!(lines[0], "nom;in_peppol;pa_code");
        assert_eq!(lines[1], "ACME;true;PA0042");
        assert_eq!(lines[2], "GLOBEX;;"); // non résolu : colonnes vides
        assert_eq!(lines[3], "ACME BIS;true;PA0042"); // même PID → mêmes infos (base)
    }

    #[test]
    fn generate_convertit_windows1252_en_utf8_avec_bom() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("in.csv");
        // "Société" avec les deux é encodés windows-1252 (0xE9).
        let mut bytes = b"siren;nom\n0009:1;Soci".to_vec();
        bytes.push(0xE9);
        bytes.push(b't');
        bytes.push(0xE9);
        bytes.push(b'\n');
        std::fs::write(&input, &bytes).unwrap();
        let out = dir.path().join("out.csv");
        let cols = vec![
            ColumnSpec::Input { name: "nom".into() },
            ColumnSpec::Peppol {
                field: PeppolField::InPeppol,
            },
        ];
        let meta = CsvMeta {
            delimiter: b';',
            encoding: "windows-1252",
        };
        let written = generate(
            &input,
            &meta,
            "siren",
            &out_cfg(cols),
            &resolutions(),
            None,
            None,
            &out,
            None,
        )
        .unwrap();
        let raw = std::fs::read(&written).unwrap();
        assert!(
            raw.starts_with(b"\xEF\xBB\xBF"),
            "la sortie doit commencer par le BOM UTF-8"
        );
        let content = String::from_utf8(raw).unwrap();
        assert!(content.contains("Société"), "contenu : {content}");
    }

    #[test]
    fn generate_erreur_claire_si_colonne_pid_absente() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("in.csv");
        std::fs::write(&input, b"siren;nom\n0009:1;ACME\n").unwrap();
        let out = dir.path().join("out.csv");
        let cols = vec![ColumnSpec::Input { name: "nom".into() }];
        let meta = CsvMeta {
            delimiter: b';',
            encoding: "utf-8",
        };
        let err = generate(
            &input,
            &meta,
            "zz",
            &out_cfg(cols),
            &resolutions(),
            None,
            None,
            &out,
            None,
        )
        .unwrap_err();
        assert!(err.contains("zz"), "message : {err}");
    }

    #[test]
    fn sortie_utf8_sans_bom() {
        // encoding: utf-8 (sans BOM) — pour les consommateurs non-Excel qui
        // traitent le BOM comme des octets parasites en tête de fichier.
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("in.csv");
        std::fs::write(&input, "siren;nom\n0009:1;Société\n").unwrap();
        let out = dir.path().join("out.csv");
        let mut cfg = out_cfg(vec![ColumnSpec::Input { name: "nom".into() }]);
        cfg.encoding = OutputEncoding::Utf8;
        let meta = CsvMeta {
            delimiter: b';',
            encoding: "utf-8",
        };
        let written = generate(&input, &meta, "siren", &cfg, &resolutions(), None, None, &out, None).unwrap();
        let raw = std::fs::read(&written).unwrap();
        assert!(!raw.starts_with(b"\xEF\xBB\xBF"), "pas de BOM attendu");
        assert!(String::from_utf8(raw).unwrap().contains("Société"));
    }

    #[test]
    fn sortie_windows1252_reencode_et_remplace_les_non_mappables() {
        // é → 0xE9 ; « → » (U+2192, absent de 1252) → « ? » (assumé, spec).
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("in.csv");
        std::fs::write(&input, "siren;nom\n0009:1;Société→Fin\n").unwrap();
        let out = dir.path().join("out.csv");
        let mut cfg = out_cfg(vec![ColumnSpec::Input { name: "nom".into() }]);
        cfg.encoding = OutputEncoding::Windows1252;
        let meta = CsvMeta {
            delimiter: b';',
            encoding: "utf-8",
        };
        let written = generate(&input, &meta, "siren", &cfg, &resolutions(), None, None, &out, None).unwrap();
        let raw = std::fs::read(&written).unwrap();
        assert!(!raw.starts_with(b"\xEF\xBB\xBF"), "pas de BOM en 1252");
        let pos = raw.windows(5).position(|w| w == b"Soci\xE9");
        assert!(pos.is_some(), "é doit être l'octet 0xE9 : {raw:?}");
        assert!(
            raw.windows(5).any(|w| w == b"\xE9?Fin"),
            "U+2192 doit devenir ? : {raw:?}"
        );
    }

    #[test]
    fn separateur_force_virgule_sur_entree_point_virgule() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("in.csv");
        std::fs::write(&input, "siren;nom\n0009:1;ACME\n").unwrap();
        let out = dir.path().join("out.csv");
        let mut cfg = out_cfg(vec![
            ColumnSpec::Input { name: "nom".into() },
            ColumnSpec::Peppol {
                field: PeppolField::InPeppol,
            },
        ]);
        cfg.separator = OutputSeparator::Comma;
        let meta = CsvMeta {
            delimiter: b';',
            encoding: "utf-8",
        };
        let written = generate(&input, &meta, "siren", &cfg, &resolutions(), None, None, &out, None).unwrap();
        let content = std::fs::read_to_string(&written).unwrap();
        let content = content.trim_start_matches('\u{feff}');
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines[0], "nom,in_peppol");
        assert_eq!(lines[1], "ACME,true");
    }

    #[test]
    fn suffixe_timestamp_insere_avant_l_extension() {
        let p = with_stamp(std::path::Path::new("/tmp/out.csv"), Some("20260712-1430"));
        assert_eq!(p, std::path::PathBuf::from("/tmp/out_20260712-1430.csv"));
        let p2 = with_stamp(std::path::Path::new("/tmp/out.csv"), None);
        assert_eq!(p2, std::path::PathBuf::from("/tmp/out.csv"));
    }

    #[test]
    fn in_directory_true_false_vide_selon_annuaire() {
        // Ligne "111" (0225 présent) → true ; "222" (0225 absent) → false ;
        // "0009:333" (non-0225) → vide. resolutions VIDE : prouve que le calcul
        // ne dépend pas de la résolution.
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("in.csv");
        std::fs::File::create(&input).unwrap()
            .write_all(b"siren\n111\n222\n0009:333\n").unwrap();
        let out = dir.path().join("out.csv");
        let meta = CsvMeta { delimiter: b';', encoding: "utf-8" };
        let mut set = std::collections::HashSet::new();
        set.insert("111".to_string());
        let cols = vec![ColumnSpec::Peppol { field: PeppolField::InDirectory }];
        let written = generate(&input, &meta, "siren", &out_cfg(cols),
                               &HashMap::new(), Some(&set), None, &out, None).unwrap();
        let content = std::fs::read_to_string(&written).unwrap();
        let lines: Vec<&str> = content.trim_start_matches('\u{feff}').lines().collect();
        assert_eq!(lines[0], "in_directory");
        assert_eq!(lines[1], "true", "0225 présent dans l'annuaire");
        assert_eq!(lines[2], "false", "0225 absent");
        // Le crate csv quote un champ vide qui est le SEUL champ de la ligne
        // (désambiguïsation avec une ligne blanche, cf. csv-core::WriterBuilder
        // ::quote_style) — n'arrive jamais en pratique (la colonne PID
        // accompagne toujours in_directory), mais ici la colonne est isolée.
        assert_eq!(lines[3], "\"\"", "non-0225 → vide");
    }

    #[test]
    fn ppf_champs_true_false_vide() {
        // "111" présent+usable ; "222" présent annuaire seul ; "0009:333"
        // (non-0225) → vide ; "444" (0225 mais ABSENT de la map) → false partout
        // (distinct de vide). resolutions VIDE : le calcul ne dépend pas de la
        // résolution.
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("in.csv");
        std::fs::File::create(&input)
            .unwrap()
            .write_all(b"siren\n111\n222\n0009:333\n444\n")
            .unwrap();
        let out = dir.path().join("out.csv");
        let meta = CsvMeta { delimiter: b';', encoding: "utf-8" };
        let mut map = HashMap::new();
        map.insert(
            "111".to_string(),
            PpfFlags { in_ppf: true, active: true, pdp_definie: true, usable: true },
        );
        map.insert(
            "222".to_string(),
            PpfFlags { in_ppf: true, active: false, pdp_definie: false, usable: false },
        );
        let cols = vec![
            ColumnSpec::Peppol { field: PeppolField::AnnuairePpf },
            ColumnSpec::Peppol { field: PeppolField::PpfUsable },
        ];
        let written = generate(
            &input, &meta, "siren", &out_cfg(cols), &HashMap::new(), None, Some(&map), &out, None,
        )
        .unwrap();
        let content = std::fs::read_to_string(&written).unwrap();
        let lines: Vec<&str> = content.trim_start_matches('\u{feff}').lines().collect();
        assert_eq!(lines[0], "annuaire_ppf;ppf_usable");
        assert_eq!(lines[1], "true;true", "111 usable");
        assert_eq!(lines[2], "true;false", "222 annuaire seul");
        assert_eq!(lines[3], ";", "non-0225 → deux vides");
        assert_eq!(lines[4], "false;false", "0225 absent de l'annuaire → false, pas vide");
    }

    #[test]
    fn ppf_champs_vides_si_annuaire_ppf_absent() {
        // ppf = None → les 4 colonnes vides même pour un 0225.
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("in.csv");
        std::fs::File::create(&input).unwrap().write_all(b"siren\n111\n").unwrap();
        let out = dir.path().join("out.csv");
        let meta = CsvMeta { delimiter: b';', encoding: "utf-8" };
        let cols = vec![
            ColumnSpec::Peppol { field: PeppolField::AnnuairePpf },
            ColumnSpec::Peppol { field: PeppolField::PpfActive },
            ColumnSpec::Peppol { field: PeppolField::PdpDefinie },
            ColumnSpec::Peppol { field: PeppolField::PpfUsable },
        ];
        let written = generate(
            &input, &meta, "siren", &out_cfg(cols), &HashMap::new(), None, None, &out, None,
        )
        .unwrap();
        let content = std::fs::read_to_string(&written).unwrap();
        let lines: Vec<&str> = content.trim_start_matches('\u{feff}').lines().collect();
        assert_eq!(lines[0], "annuaire_ppf;ppf_active;pdp_definie;ppf_usable");
        assert_eq!(lines[1], ";;;", "annuaire PPF absent → 4 vides");
    }

    #[test]
    fn in_directory_vide_si_annuaire_non_charge() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("in.csv");
        std::fs::File::create(&input).unwrap().write_all(b"siren\n111\n").unwrap();
        let out = dir.path().join("out.csv");
        let meta = CsvMeta { delimiter: b';', encoding: "utf-8" };
        let cols = vec![ColumnSpec::Peppol { field: PeppolField::InDirectory }];
        // directory = None → colonne vide même pour un 0225.
        let written = generate(&input, &meta, "siren", &out_cfg(cols),
                               &HashMap::new(), None, None, &out, None).unwrap();
        let content = std::fs::read_to_string(&written).unwrap();
        let lines: Vec<&str> = content.trim_start_matches('\u{feff}').lines().collect();
        assert_eq!(lines[0], "in_directory");
        // Champ vide seul sur la ligne → quoté par le crate csv (même remarque
        // que ci-dessus).
        assert_eq!(lines[1], "\"\"", "annuaire non chargé → vide");
    }
}
