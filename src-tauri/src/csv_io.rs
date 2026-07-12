use encoding_rs_io::DecodeReaderBytesBuilder;
use serde::Serialize;
use std::fs::File;
use std::io::Read;
use std::path::Path;

#[derive(Debug, Clone, Serialize)]
pub struct CsvMeta {
    pub delimiter: u8,
    pub encoding: &'static str, // "utf-8" | "windows-1252"
}

#[derive(Debug, Clone, Serialize)]
pub struct Preview {
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
    pub delimiter: String,
    pub encoding: String,
}

/// Détecte séparateur et encodage sur les premiers 64 Ko.
pub fn sniff(path: &Path) -> Result<CsvMeta, String> {
    let mut buf = vec![0u8; 64 * 1024];
    let n = File::open(path)
        .and_then(|mut f| f.read(&mut buf))
        .map_err(|e| format!("lecture {path:?} : {e}"))?;
    let sample = &buf[..n];

    // UTF-8 si l'échantillon est valide (l'ASCII pur en est un sous-ensemble) ;
    // une coupure au milieu d'un caractère multi-octets en toute fin de buffer
    // ne doit pas fausser la détection. Sinon : windows-1252 (cas Excel FR).
    let encoding = match std::str::from_utf8(sample) {
        Ok(_) => "utf-8",
        Err(e) if e.valid_up_to() + 4 >= sample.len() => "utf-8",
        Err(_) => "windows-1252",
    };

    let first_line = sample.split(|&b| b == b'\n').next().unwrap_or(sample);
    let delimiter = [b';', b',', b'\t', b'|']
        .into_iter()
        .max_by_key(|d| first_line.iter().filter(|&&b| b == *d).count())
        .unwrap();
    Ok(CsvMeta {
        delimiter,
        encoding,
    })
}

fn reader(path: &Path, meta: &CsvMeta) -> Result<csv::Reader<Box<dyn Read>>, String> {
    let f = File::open(path).map_err(|e| format!("ouverture {path:?} : {e}"))?;
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
    Ok(csv::ReaderBuilder::new()
        .delimiter(meta.delimiter)
        .flexible(true)
        .from_reader(decoded))
}

/// Entêtes + n premières lignes, pour l'aperçu du wizard.
pub fn preview(path: &Path, n: usize) -> Result<Preview, String> {
    let meta = sniff(path)?;
    let mut rdr = reader(path, &meta)?;
    let headers = rdr
        .headers()
        .map_err(|e| e.to_string())?
        .iter()
        .map(String::from)
        .collect();
    let mut rows = Vec::with_capacity(n);
    for rec in rdr.records().take(n) {
        let rec = rec.map_err(|e| e.to_string())?;
        rows.push(rec.iter().map(String::from).collect());
    }
    Ok(Preview {
        headers,
        rows,
        delimiter: (meta.delimiter as char).to_string(),
        encoding: meta.encoding.to_string(),
    })
}

/// Toutes les valeurs (brutes, non dédupliquées) d'une colonne, dans l'ordre
/// du fichier. Streaming : la mémoire ne contient que les valeurs.
pub fn read_column(path: &Path, meta: &CsvMeta, column: &str) -> Result<Vec<String>, String> {
    let mut rdr = reader(path, meta)?;
    let headers = rdr.headers().map_err(|e| e.to_string())?.clone();
    let idx = headers
        .iter()
        .position(|h| h == column)
        .ok_or_else(|| format!("Colonne '{column}' absente de l'entête : {headers:?}"))?;
    let mut out = Vec::new();
    for rec in rdr.records() {
        let rec = rec.map_err(|e| e.to_string())?;
        out.push(rec.get(idx).unwrap_or("").to_string());
    }
    Ok(out)
}

/// Ressemble à un adressage Peppol : forme longue "scheme::valeur",
/// "xxxx:yyyy" (préfixe numérique à 4 chiffres), ou SIREN (9 chiffres).
fn looks_like_pid(v: &str) -> bool {
    let v = v.trim();
    if v.contains("::") {
        return true;
    }
    if let Some((prefix, rest)) = v.split_once(':') {
        return prefix.len() == 4 && prefix.chars().all(|c| c.is_ascii_digit()) && !rest.is_empty();
    }
    v.len() == 9 && v.chars().all(|c| c.is_ascii_digit())
}

/// Suggère l'index de la colonne d'adressage : celle dont ≥ 60 % des valeurs
/// d'exemple non vides ressemblent à un PID (meilleur score si plusieurs).
pub fn suggest_pid_column(p: &Preview) -> Option<usize> {
    let mut best: Option<(usize, f64)> = None;
    for col in 0..p.headers.len() {
        let vals: Vec<&str> = p
            .rows
            .iter()
            .filter_map(|r| r.get(col).map(String::as_str))
            .filter(|v| !v.trim().is_empty())
            .collect();
        if vals.is_empty() {
            continue;
        }
        let score = vals.iter().filter(|v| looks_like_pid(v)).count() as f64 / vals.len() as f64;
        if score >= 0.6 && best.map_or(true, |(_, s)| score > s) {
            best = Some((col, score));
        }
    }
    best.map(|(i, _)| i)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn tmp_csv(content: &[u8]) -> tempfile::NamedTempFile {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(content).unwrap();
        f.flush().unwrap();
        f
    }

    #[test]
    fn sniff_detecte_point_virgule_et_utf8() {
        let f = tmp_csv("siren;raison_sociale\n0009:1;ACME\n".as_bytes());
        let m = sniff(f.path()).unwrap();
        assert_eq!(m.delimiter, b';');
        assert_eq!(m.encoding, "utf-8");
    }

    #[test]
    fn sniff_detecte_virgule_et_windows1252() {
        // "société" avec é encodé windows-1252 (0xE9)
        let mut bytes = b"siren,soci".to_vec();
        bytes.push(0xE9);
        bytes.extend_from_slice(b"t\n1,ACME\n");
        let f = tmp_csv(&bytes);
        let m = sniff(f.path()).unwrap();
        assert_eq!(m.delimiter, b',');
        assert_eq!(m.encoding, "windows-1252");
    }

    #[test]
    fn preview_renvoie_entetes_et_lignes() {
        let f = tmp_csv("a;b\n1;x\n2;y\n3;z\n".as_bytes());
        let p = preview(f.path(), 2).unwrap();
        assert_eq!(p.headers, vec!["a", "b"]);
        assert_eq!(p.rows, vec![vec!["1", "x"], vec!["2", "y"]]);
    }

    #[test]
    fn read_column_renvoie_toutes_les_valeurs_dans_l_ordre() {
        let f = tmp_csv("id;siren\nl1;0009:1\nl2;0009:2\nl3;0009:1\n".as_bytes());
        let m = sniff(f.path()).unwrap();
        let vals = read_column(f.path(), &m, "siren").unwrap();
        assert_eq!(vals, vec!["0009:1", "0009:2", "0009:1"]);
    }

    #[test]
    fn read_column_colonne_inconnue_erreur_claire() {
        let f = tmp_csv("a;b\n1;2\n".as_bytes());
        let m = sniff(f.path()).unwrap();
        let err = read_column(f.path(), &m, "zz").unwrap_err();
        assert!(err.contains("zz"), "message: {err}");
    }

    #[test]
    fn suggest_trouve_la_colonne_pid() {
        let p = Preview {
            headers: vec!["id".into(), "siren".into(), "nom".into()],
            rows: vec![
                vec!["l1".into(), "0009:552100554".into(), "ACME".into()],
                vec!["l2".into(), "552100554".into(), "GLOBEX".into()],
            ],
            delimiter: ";".into(),
            encoding: "utf-8".into(),
        };
        assert_eq!(suggest_pid_column(&p), Some(1));
    }

    #[test]
    fn suggest_none_si_rien_ne_ressemble() {
        let p = Preview {
            headers: vec!["nom".into()],
            rows: vec![vec!["ACME".into()], vec!["GLOBEX".into()]],
            delimiter: ";".into(),
            encoding: "utf-8".into(),
        };
        assert_eq!(suggest_pid_column(&p), None);
    }
}
