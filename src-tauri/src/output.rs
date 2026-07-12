use crate::config::{ColumnSpec, PeppolField};
use crate::csv_io::CsvMeta;
use crate::pid::canonical;
use crate::store::Resolution;
use encoding_rs_io::DecodeReaderBytesBuilder;
use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
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
/// mapping, infos Peppol lues dans `resolutions` (la base). UTF-8 en sortie.
pub fn generate(
    input_path: &Path,
    meta: &CsvMeta,
    pid_column: &str,
    columns: &[ColumnSpec],
    resolutions: &HashMap<String, Resolution>,
    out_path: &Path,
    stamp: Option<&str>,
) -> Result<PathBuf, String> {
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

    let final_path = with_stamp(out_path, stamp);
    let mut wtr = csv::WriterBuilder::new()
        .delimiter(meta.delimiter)
        .from_path(&final_path)
        .map_err(|e| format!("écriture {final_path:?} : {e}"))?;
    // Entête de sortie.
    let out_headers: Vec<String> = columns
        .iter()
        .map(|c| match c {
            ColumnSpec::Input { name } => name.clone(),
            ColumnSpec::Peppol { field } => field_name(*field).to_string(),
        })
        .collect();
    wtr.write_record(&out_headers).map_err(|e| e.to_string())?;

    for rec in rdr.records() {
        let rec = rec.map_err(|e| e.to_string())?;
        let raw_pid = rec.get(pid_idx).unwrap_or("");
        let res = resolutions.get(&canonical(raw_pid));
        let row: Vec<String> = columns
            .iter()
            .zip(&col_idx)
            .map(|(c, idx)| match c {
                ColumnSpec::Input { .. } => rec.get(idx.unwrap()).unwrap_or("").to_string(),
                ColumnSpec::Peppol { field } => match res {
                    None => String::new(),
                    Some(r) => match field {
                        PeppolField::Exists => fmt_bool(r.exists_in_peppol).to_string(),
                        PeppolField::PaCode => r.pa_code.clone().unwrap_or_default(),
                        PeppolField::PaName => r.pa_name.clone().unwrap_or_default(),
                        PeppolField::PaCountry => r.pa_country.clone().unwrap_or_default(),
                        PeppolField::ExtendedCtcFr => fmt_bool(r.extended_ctc_fr).to_string(),
                    },
                },
            })
            .collect();
        wtr.write_record(&row).map_err(|e| e.to_string())?;
    }
    wtr.flush().map_err(|e| e.to_string())?;
    Ok(final_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ColumnSpec, PeppolField};
    use crate::csv_io::CsvMeta;
    use crate::store::Resolution;
    use std::collections::HashMap;
    use std::io::Write;

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
        let written = generate(&input, &meta, "siren", &cols, &resolutions(), &out, None).unwrap();
        let content = std::fs::read_to_string(&written).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 4); // entête + 3 lignes (autant que l'entrée)
        assert_eq!(lines[0], "nom;exists;pa_code");
        assert_eq!(lines[1], "ACME;true;PA0042");
        assert_eq!(lines[2], "GLOBEX;;"); // non résolu : colonnes vides
        assert_eq!(lines[3], "ACME BIS;true;PA0042"); // même PID → mêmes infos (base)
    }

    #[test]
    fn suffixe_timestamp_insere_avant_l_extension() {
        let p = with_stamp(std::path::Path::new("/tmp/out.csv"), Some("20260712-1430"));
        assert_eq!(p, std::path::PathBuf::from("/tmp/out_20260712-1430.csv"));
        let p2 = with_stamp(std::path::Path::new("/tmp/out.csv"), None);
        assert_eq!(p2, std::path::PathBuf::from("/tmp/out.csv"));
    }
}
