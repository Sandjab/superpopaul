use crate::config::{ColumnSpec, OutputConfig, OutputEncoding, OutputSeparator, PeppolField};
use crate::csv_io::CsvMeta;
use crate::pid::canonical;
use crate::store::Resolution;
use encoding_rs_io::DecodeReaderBytesBuilder;
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};

fn fmt_bool(b: Option<bool>) -> &'static str {
    match b {
        Some(true) => "true",
        Some(false) => "false",
        None => "",
    }
}

pub fn field_name(f: PeppolField) -> &'static str {
    match f {
        PeppolField::Exists => "exists",
        PeppolField::PaCode => "pa_code",
        PeppolField::PaName => "pa_name",
        PeppolField::PaCountry => "pa_country",
        PeppolField::ExtendedCtcFr => "extended_ctc_fr",
    }
}

/// Insère `_<stamp>` avant l'extension.
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
/// `output.path`/`timestamp_suffix` sont ignorés ici — l'appelant fournit
/// `out_path` (résolu relativement au YAML) et `stamp` déjà calculés.
///
/// Écriture atomique (comme `config::save`) : tout passe par `<final>.tmp`
/// dans le même répertoire, renommé vers la cible seulement après le flush —
/// un disque plein ou une permission refusée en cours de route ne laisse
/// jamais un CSV d'apparence valide mais tronqué.
pub fn generate(
    input_path: &Path,
    meta: &CsvMeta,
    pid_column: &str,
    output: &OutputConfig,
    resolutions: &HashMap<String, Resolution>,
    out_path: &Path,
    stamp: Option<&str>,
) -> Result<PathBuf, String> {
    let final_path = with_stamp(out_path, stamp);
    let mut tmp_os = final_path.clone().into_os_string();
    tmp_os.push(".tmp");
    let tmp_path = PathBuf::from(tmp_os);

    if let Err(e) = write_output(input_path, meta, pid_column, output, resolutions, &tmp_path) {
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
fn write_output(
    input_path: &Path,
    meta: &CsvMeta,
    pid_column: &str,
    output: &OutputConfig,
    resolutions: &HashMap<String, Resolution>,
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

    // Numérotation : l'entête est la ligne 1, le premier enregistrement la 2.
    for (line, rec) in (2u64..).zip(rdr.records()) {
        let rec = rec.map_err(|e| format!("lecture {input_path:?} ligne {line} : {e}"))?;
        let raw_pid = rec.get(pid_idx).unwrap_or("");
        let res = resolutions.get(&canonical(raw_pid));
        // Zéro allocation par cellule : la ligne est un Vec<&str>.
        let row: Vec<&str> = columns
            .iter()
            .zip(&col_idx)
            .map(|(c, idx)| match c {
                ColumnSpec::Input { .. } => rec.get(idx.unwrap()).unwrap_or(""),
                ColumnSpec::Peppol { field } => match res {
                    None => "",
                    Some(r) => match field {
                        PeppolField::Exists => fmt_bool(r.exists_in_peppol),
                        PeppolField::PaCode => r.pa_code.as_deref().unwrap_or(""),
                        PeppolField::PaName => r.pa_name.as_deref().unwrap_or(""),
                        PeppolField::PaCountry => r.pa_country.as_deref().unwrap_or(""),
                        PeppolField::ExtendedCtcFr => fmt_bool(r.extended_ctc_fr),
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
    use crate::store::Resolution;
    use std::collections::HashMap;
    use std::io::Write;

    /// OutputConfig de test : défauts historiques (UTF-8+BOM, séparateur de
    /// l'entrée). `path`/`timestamp_suffix` sont ignorés par generate (résolus
    /// par l'appelant).
    fn out_cfg(columns: Vec<ColumnSpec>) -> OutputConfig {
        OutputConfig {
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
            },
        );
        m
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
                field: PeppolField::Exists,
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
            &out,
            None,
        )
        .unwrap();
        let content = std::fs::read_to_string(&written).unwrap();
        // BOM retiré ici, testé dédié dans generate_convertit_windows1252_en_utf8_avec_bom.
        let content = content.trim_start_matches('\u{feff}');
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 4); // entête + 3 lignes (autant que l'entrée)
        assert_eq!(lines[0], "nom;exists;pa_code");
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
                field: PeppolField::Exists,
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
        let written = generate(&input, &meta, "siren", &cfg, &resolutions(), &out, None).unwrap();
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
        let written = generate(&input, &meta, "siren", &cfg, &resolutions(), &out, None).unwrap();
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
                field: PeppolField::Exists,
            },
        ]);
        cfg.separator = OutputSeparator::Comma;
        let meta = CsvMeta {
            delimiter: b';',
            encoding: "utf-8",
        };
        let written = generate(&input, &meta, "siren", &cfg, &resolutions(), &out, None).unwrap();
        let content = std::fs::read_to_string(&written).unwrap();
        let content = content.trim_start_matches('\u{feff}');
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines[0], "nom,exists");
        assert_eq!(lines[1], "ACME,true");
    }

    #[test]
    fn suffixe_timestamp_insere_avant_l_extension() {
        let p = with_stamp(std::path::Path::new("/tmp/out.csv"), Some("20260712-1430"));
        assert_eq!(p, std::path::PathBuf::from("/tmp/out_20260712-1430.csv"));
        let p2 = with_stamp(std::path::Path::new("/tmp/out.csv"), None);
        assert_eq!(p2, std::path::PathBuf::from("/tmp/out.csv"));
    }
}
