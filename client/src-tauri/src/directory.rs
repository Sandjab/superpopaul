//! Ingestion de l'annuaire Peppol (fichier export-all-participants.csv) —
//! fonctionnalité CLIENT-ONLY : aucune parité avec cli/popaul.py.
//! On ne charge que l'adressage 0225 (SIRENE français), stocké sans son
//! préfixe de scheme/ICD.

use std::io::Read;

/// Préfixe des Participant ID d'adressage 0225. Le scheme est l'invariant de
/// `pid::DEFAULT_SCHEME` ; le « 0225 » est l'exigence explicite du chantier
/// (test `prefixe_coherent_avec_pid` en garde-fou contre la dérive).
pub(crate) const PREFIX_0225: &str = "iso6523-actorid-upis::0225:";

/// URL d'export de l'annuaire Peppol (Télécharger).
pub const DIRECTORY_URL: &str = "https://directory.peppol.eu/export/participants-csv";

/// Renvoie la valeur (partie après `iso6523-actorid-upis::0225:`) si le
/// Participant ID est en 0225, sinon `None`. Verbatim : les suffixes
/// (`_replyto`, `_cdv_…`, `_SIRET`) sont conservés. Préfixe seul sans valeur → `None`.
pub fn parse_0225_value(participant_id: &str) -> Option<String> {
    match participant_id.trim().strip_prefix(PREFIX_0225) {
        Some(rest) if !rest.is_empty() => Some(rest.to_string()),
        _ => None,
    }
}

/// Lit un CSV mono-colonne (`Participant ID`) en flux et renvoie les valeurs
/// 0225 dans l'ordre. `on_progress(lignes_lues)` est appelé tous les 100 000
/// enregistrements puis une fois en fin de lecture. BLOQUANT (5,2 M lignes
/// possibles) : appeler depuis `spawn_blocking`.
pub fn stream_0225_values<R: Read>(
    reader: R,
    mut on_progress: impl FnMut(u64),
) -> Result<Vec<String>, String> {
    let mut rdr = csv::ReaderBuilder::new().has_headers(true).from_reader(reader);
    let mut record = csv::StringRecord::new();
    let mut out = Vec::new();
    let mut lines: u64 = 0;
    loop {
        match rdr.read_record(&mut record) {
            Ok(true) => {
                lines += 1;
                if let Some(field) = record.get(0) {
                    if let Some(v) = parse_0225_value(field) {
                        out.push(v);
                    }
                }
                if lines % 100_000 == 0 {
                    on_progress(lines);
                }
            }
            Ok(false) => break,
            Err(e) => return Err(format!("lecture CSV de l'annuaire : {e}")),
        }
    }
    on_progress(lines);
    Ok(out)
}

/// Télécharge l'annuaire (streaming, chunk par chunk) dans un fichier
/// temporaire supprimé au Drop — le brut 214 Mo n'est jamais conservé.
/// `on_progress(octets_reçus, content_length)` alimente la barre.
/// Honore le proxy configuré (même construction que `api.rs`).
pub async fn download_to_temp(
    url: &str,
    proxy_url: Option<&str>,
    creds: Option<&crate::api::ProxyCreds>,
    mut on_progress: impl FnMut(u64, Option<u64>),
) -> Result<tempfile::NamedTempFile, String> {
    use std::io::Write;
    let mut b = reqwest::Client::builder().connect_timeout(std::time::Duration::from_secs(10));
    if let Some(purl) = proxy_url {
        let mut p = reqwest::Proxy::all(purl).map_err(|e| format!("proxy : {e}"))?;
        if let Some(c) = creds {
            p = p.basic_auth(&c.username, &c.password);
        }
        b = b.proxy(p);
    }
    let client = b.build().map_err(|e| e.to_string())?;
    let mut resp = client.get(url).send().await.map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("téléchargement de l'annuaire : HTTP {}", resp.status().as_u16()));
    }
    let total = resp.content_length();
    let mut tmp = tempfile::NamedTempFile::new().map_err(|e| e.to_string())?;
    let mut done: u64 = 0;
    while let Some(chunk) = resp.chunk().await.map_err(|e| e.to_string())? {
        tmp.write_all(&chunk).map_err(|e| e.to_string())?;
        done += chunk.len() as u64;
        on_progress(done, total);
    }
    tmp.flush().map_err(|e| e.to_string())?;
    Ok(tmp)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefixe_coherent_avec_pid() {
        // Garde-fou : le préfixe 0225 doit rester aligné sur le scheme par
        // défaut de la canonicalisation.
        assert_eq!(PREFIX_0225, format!("{}::0225:", crate::pid::DEFAULT_SCHEME));
    }

    #[test]
    fn extrait_la_valeur_0225_nue() {
        assert_eq!(
            parse_0225_value("iso6523-actorid-upis::0225:000122308"),
            Some("000122308".to_string())
        );
    }

    #[test]
    fn conserve_les_suffixes_techniques_verbatim() {
        // Les entrées à suffixe (_replyto, _cdv_…) sont de vrais inscrits :
        // on les garde tels quels, on ne normalise pas.
        assert_eq!(
            parse_0225_value("iso6523-actorid-upis::0225:000009777_0054_replyto"),
            Some("000009777_0054_replyto".to_string())
        );
        assert_eq!(
            parse_0225_value("iso6523-actorid-upis::0225:005580436_cdv_d6a4bbca"),
            Some("005580436_cdv_d6a4bbca".to_string())
        );
    }

    #[test]
    fn ignore_les_autres_schemes() {
        assert_eq!(parse_0225_value("iso6523-actorid-upis::0002:000126010"), None);
        assert_eq!(parse_0225_value("iso6523-actorid-upis::0009:552100554"), None);
    }

    #[test]
    fn ignore_le_prefixe_seul_et_l_entete() {
        assert_eq!(parse_0225_value("iso6523-actorid-upis::0225:"), None);
        assert_eq!(parse_0225_value("Participant ID"), None);
        assert_eq!(parse_0225_value(""), None);
    }

    #[test]
    fn trimme_l_entree() {
        assert_eq!(
            parse_0225_value("  iso6523-actorid-upis::0225:000122308  "),
            Some("000122308".to_string())
        );
    }

    #[test]
    fn stream_ne_garde_que_le_0225_dans_l_ordre() {
        // En-tête + mélange de schemes ; seules les valeurs 0225 ressortent,
        // dans l'ordre de lecture, en-tête ignoré.
        let csv = "\"Participant ID\"\n\
                   \"iso6523-actorid-upis::0002:000126010\"\n\
                   \"iso6523-actorid-upis::0225:000122308\"\n\
                   \"iso6523-actorid-upis::0009:552100554\"\n\
                   \"iso6523-actorid-upis::0225:000009777_0054_replyto\"\n";
        let mut progress_calls = 0u32;
        let vals = stream_0225_values(std::io::Cursor::new(csv), |_| progress_calls += 1).unwrap();
        assert_eq!(vals, vec!["000122308".to_string(), "000009777_0054_replyto".to_string()]);
        assert!(progress_calls >= 1, "on_progress doit être appelé au moins une fois");
    }

    #[test]
    fn stream_csv_vide_ou_entete_seule() {
        let vals = stream_0225_values(std::io::Cursor::new("\"Participant ID\"\n"), |_| {}).unwrap();
        assert!(vals.is_empty());
    }

    #[test]
    fn stream_csv_malforme_remonte_une_erreur() {
        // Un CSV incohérent (ligne à 2 champs alors que l'en-tête en a 1)
        // doit faire échouer tout l'import plutôt que produire un annuaire
        // partiel silencieux — « fail loud ». Le lecteur csv est en mode
        // strict (flexible=false par défaut) : nombre de champs incohérent =
        // erreur.
        let csv = "\"Participant ID\"\n\"iso6523-actorid-upis::0225:000122308\",\"en trop\"\n";
        let res = stream_0225_values(std::io::Cursor::new(csv), |_| {});
        assert!(res.is_err(), "un CSV malformé doit remonter une Err");
        assert!(
            res.unwrap_err().contains("lecture CSV de l'annuaire"),
            "le message d'erreur doit être celui de l'annuaire"
        );
    }

    #[tokio::test]
    async fn download_ecrit_le_corps_et_rapporte_la_progression() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        let body = "\"Participant ID\"\n\"iso6523-actorid-upis::0225:000122308\"\n";
        Mock::given(method("GET"))
            .and(path("/export/participants-csv"))
            .respond_with(ResponseTemplate::new(200).set_body_string(body))
            .mount(&server)
            .await;

        let mut last_done = 0u64;
        let tmp = download_to_temp(
            &format!("{}/export/participants-csv", server.uri()),
            None,
            None,
            |done, _total| last_done = done,
        )
        .await
        .unwrap();

        let content = std::fs::read_to_string(tmp.path()).unwrap();
        assert_eq!(content, body);
        assert_eq!(last_done, body.len() as u64);
    }

    #[tokio::test]
    async fn download_statut_erreur_remonte_une_err() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/export/participants-csv"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;

        let res = download_to_temp(
            &format!("{}/export/participants-csv", server.uri()),
            None,
            None,
            |_, _| {},
        )
        .await;
        assert!(res.is_err(), "un statut HTTP non-2xx doit remonter une Err");
        assert!(
            res.unwrap_err().contains("HTTP 500"),
            "le message doit mentionner le code HTTP"
        );
    }
}
