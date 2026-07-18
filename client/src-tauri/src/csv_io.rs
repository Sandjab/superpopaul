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
    /// Signature des en-têtes (columns_hash) — comparée à celle des profils.
    pub columns_hash: String,
    pub size_bytes: u64,
}

/// Détecte séparateur et encodage sur les premiers 64 Ko.
pub fn sniff(path: &Path) -> Result<CsvMeta, String> {
    let mut buf = vec![0u8; 64 * 1024];
    let n = File::open(path)
        .and_then(|mut f| f.read(&mut buf))
        .map_err(|e| format!("lecture {path:?} : {e}"))?;
    let sample = &buf[..n];

    // Garde : un octet NUL signale un binaire renommé .csv (xlsx, zip…) —
    // rejeté tôt plutôt qu'un aperçu illisible. L'UTF-16 est rejeté aussi
    // (assumé : seuls utf-8 / windows-1252 sont supportés).
    if sample.contains(&0u8) {
        return Err("fichier binaire (pas un CSV texte)".into());
    }

    // UTF-8 si l'échantillon est valide (l'ASCII pur en est un sous-ensemble) ;
    // une coupure au milieu d'un caractère multi-octets en toute fin de buffer
    // ne doit pas fausser la détection. Sinon : windows-1252 (cas Excel FR).
    let encoding = match std::str::from_utf8(sample) {
        Ok(_) => "utf-8",
        Err(e) if e.valid_up_to() + 4 >= sample.len() => "utf-8",
        Err(_) => "windows-1252",
    };

    let first_line = sample.split(|&b| b == b'\n').next().unwrap_or(sample);
    let counts = count_delims_outside_quotes(first_line);
    let delimiter = [b';', b',', b'\t', b'|']
        .into_iter()
        .max_by_key(|d| counts[*d as usize])
        .unwrap();
    Ok(CsvMeta {
        delimiter,
        encoding,
    })
}

/// Compte les occurrences de chaque octet hors sections quotées (`"…"`).
/// Un simple basculement d'état suffit, y compris pour l'échappement `""` :
/// il fait sortir puis rentrer dans la section quotée sans compter d'octet
/// entre les deux guillemets.
fn count_delims_outside_quotes(line: &[u8]) -> [usize; 256] {
    let mut counts = [0usize; 256];
    let mut in_quotes = false;
    for &b in line {
        if b == b'"' {
            in_quotes = !in_quotes;
        } else if !in_quotes {
            counts[b as usize] += 1;
        }
    }
    counts
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
/// Suppose une ligne d'entête (choix de design du wizard) : un fichier sans
/// entête verra sa première ligne consommée comme entête.
pub fn preview(path: &Path, n: usize) -> Result<Preview, String> {
    let meta = sniff(path)?;
    let mut rdr = reader(path, &meta)?;
    let headers: Vec<String> = rdr
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
    let size_bytes = std::fs::metadata(path)
        .map_err(|e| format!("métadonnées {path:?} : {e}"))?
        .len();
    let hash = columns_hash(&headers);
    Ok(Preview {
        headers,
        rows,
        delimiter: (meta.delimiter as char).to_string(),
        encoding: meta.encoding.to_string(),
        columns_hash: hash,
        size_bytes,
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
        if score >= 0.6 && best.is_none_or(|(_, s)| score > s) {
            best = Some((col, score));
        }
    }
    best.map(|(i, _)| i)
}

fn fnv1a(mut h: u64, bytes: &[u8]) -> u64 {
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Signature des en-têtes d'entrée : FNV-1a 64 bits sur les octets UTF-8,
/// chaque en-tête préfixé par sa longueur (8 octets little-endian). Ordre et
/// casse significatifs, aucune normalisation. Valeur PERSISTÉE dans les
/// profils : l'algorithme ne doit jamais changer (test avec valeur en dur).
pub fn columns_hash(headers: &[String]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for name in headers {
        h = fnv1a(h, &(name.len() as u64).to_le_bytes());
        h = fnv1a(h, name.as_bytes());
    }
    format!("{h:016x}")
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
    fn sniff_ignore_les_delimiteurs_dans_les_guillemets() {
        let f = tmp_csv(b"id;\"tags,a,b,c,d\"\n1;x\n");
        let m = sniff(f.path()).unwrap();
        assert_eq!(m.delimiter, b';');
    }

    #[test]
    fn sniff_rejette_un_fichier_binaire() {
        // Garde étape 1 : un binaire renommé .csv (xlsx, zip…) contient des
        // octets NUL — le rejeter tôt évite un aperçu illisible. Effet de
        // bord assumé : l'UTF-16 est rejeté aussi (encodages supportés :
        // utf-8 / windows-1252 uniquement).
        let f = tmp_csv(b"PK\x03\x04\x00\x00binaire");
        let err = sniff(f.path()).unwrap_err();
        assert!(err.contains("binaire"), "message inattendu : {err}");
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
            columns_hash: String::new(),
            size_bytes: 0,
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
            columns_hash: String::new(),
            size_bytes: 0,
        };
        assert_eq!(suggest_pid_column(&p), None);
    }

    #[test]
    fn columns_hash_stable_ordre_casse_et_non_ambigu() {
        let h = |names: &[&str]| {
            columns_hash(&names.iter().map(|s| s.to_string()).collect::<Vec<_>>())
        };
        // Valeur en dur : le hash est PERSISTÉ dans les profils — si cette
        // assertion casse, l'algorithme a changé et tous les profils existants
        // deviennent incompatibles. Ne jamais « corriger » la valeur attendue.
        assert_eq!(h(&["SIREN", "RAISON_SOCIALE", "VILLE"]), "ec46ac4b9e99375d");
        // L'ordre des colonnes est significatif.
        assert_ne!(
            h(&["SIREN", "RAISON_SOCIALE", "VILLE"]),
            h(&["VILLE", "RAISON_SOCIALE", "SIREN"])
        );
        // La casse est significative (la résolution des colonnes l'est aussi).
        assert_ne!(
            h(&["SIREN", "RAISON_SOCIALE", "VILLE"]),
            h(&["siren", "raison_sociale", "ville"])
        );
        // Préfixage par longueur : pas d'ambiguïté de concaténation.
        assert_ne!(h(&["ab", "c"]), h(&["a", "bc"]));
    }

    #[test]
    fn preview_expose_hash_des_colonnes_et_taille() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("x.csv");
        std::fs::write(&p, "a;b\n1;2\n").unwrap();
        let prev = preview(&p, 5).unwrap();
        assert_eq!(prev.columns_hash, columns_hash(&prev.headers));
        assert_eq!(prev.columns_hash, "41c80da72d0aec94"); // ["a", "b"], valeur en dur
        assert_eq!(prev.size_bytes, 8); // "a;b\n1;2\n"
    }
}
