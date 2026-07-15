use crate::store::Store;
use serde::Deserialize;

// NB : deny_unknown_fields protège les variantes struct (Reprise/Refresh)
// contre les champs inconnus du front ; les variantes unit (Full) les
// ignorent toujours (limitation serde).
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "mode", rename_all = "lowercase", deny_unknown_fields)]
pub enum RunMode {
    Full,
    Reprise { retry_failures: bool },
    Refresh { max_age_days: u32 },
}

/// Liste des adressages à résoudre parmi les PIDs uniques du fichier
/// d'entrée, selon le mode. L'ordre d'entrée est conservé.
pub fn compute_todo(
    mode: &RunMode,
    unique_pids: &[String],
    store: &Store,
    now: i64,
) -> Result<Vec<String>, String> {
    if matches!(mode, RunMode::Full) {
        return Ok(unique_pids.to_vec());
    }
    let known = store.load_map(unique_pids)?;
    let keep = |pid: &&String| -> bool {
        match known.get(*pid) {
            None => true, // jamais tenté
            Some(r) => match mode {
                RunMode::Full => true,
                RunMode::Reprise { retry_failures } => *retry_failures && r.api_status != "ok",
                RunMode::Refresh { max_age_days } => {
                    r.api_status != "ok" || r.resolved_at < now - (*max_age_days as i64) * 86400
                }
            },
        }
    };
    Ok(unique_pids.iter().filter(keep).cloned().collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Resolution;

    /// Base : a::1 résolu ok récent, a::2 résolu ok VIEUX, a::3 en échec,
    /// a::4 absent. now = 100 jours (en secondes).
    fn base() -> (Store, Vec<String>, i64) {
        let s = Store::open_in_memory().unwrap();
        let now = 100 * 86400_i64;
        let mk = |pid: &str, status: &str, at: i64| Resolution {
            participant: pid.into(),
            exists_in_peppol: Some(status == "ok"),
            pa_code: None,
            pa_name: None,
            pa_country: None,
            extended_ctc_fr: None,
            api_status: status.into(),
            resolved_at: at,
            note: None,
        };
        s.upsert(&mk("a::1", "ok", now - 86400)).unwrap(); // 1 jour
        s.upsert(&mk("a::2", "ok", now - 50 * 86400)).unwrap(); // 50 jours
        s.upsert(&mk("a::3", "error:503", now - 86400)).unwrap(); // échec
        let pids: Vec<String> = ["a::1", "a::2", "a::3", "a::4"]
            .into_iter()
            .map(String::from)
            .collect();
        (s, pids, now)
    }

    #[test]
    fn full_prend_tout() {
        let (s, pids, now) = base();
        assert_eq!(compute_todo(&RunMode::Full, &pids, &s, now).unwrap(), pids);
    }

    #[test]
    fn reprise_prend_les_absents_seulement() {
        let (s, pids, now) = base();
        let mode = RunMode::Reprise {
            retry_failures: false,
        };
        assert_eq!(
            compute_todo(&mode, &pids, &s, now).unwrap(),
            vec!["a::4".to_string()]
        );
    }

    #[test]
    fn reprise_avec_retry_reprend_aussi_les_echecs() {
        let (s, pids, now) = base();
        let mode = RunMode::Reprise {
            retry_failures: true,
        };
        assert_eq!(
            compute_todo(&mode, &pids, &s, now).unwrap(),
            vec!["a::3".to_string(), "a::4".to_string()]
        );
    }

    #[test]
    fn refresh_prend_absents_echecs_et_perimes() {
        let (s, pids, now) = base();
        let mode = RunMode::Refresh { max_age_days: 30 };
        // a::2 (50 jours) est périmé ; a::3 (échec) repris ; a::4 absent ;
        // a::1 (1 jour) est frais → exclu.
        assert_eq!(
            compute_todo(&mode, &pids, &s, now).unwrap(),
            vec!["a::2".to_string(), "a::3".to_string(), "a::4".to_string()]
        );
    }

    #[test]
    fn refresh_borne_exactement_max_age_est_frais() {
        let (s, _, now) = base();
        // Résolution ok datée d'exactement max_age_days : la sémantique est
        // `<` stricte, donc elle est encore fraîche → non reprise.
        s.upsert(&Resolution {
            participant: "a::borne".into(),
            exists_in_peppol: Some(true),
            pa_code: None,
            pa_name: None,
            pa_country: None,
            extended_ctc_fr: None,
            api_status: "ok".into(),
            resolved_at: now - 30 * 86400,
            note: None,
        })
        .unwrap();
        let mode = RunMode::Refresh { max_age_days: 30 };
        let pids = vec!["a::borne".to_string()];
        assert_eq!(
            compute_todo(&mode, &pids, &s, now).unwrap(),
            Vec::<String>::new()
        );
    }

    #[test]
    fn champ_inconnu_rejete_sur_variante_struct() {
        // deny_unknown_fields agit sur les variantes struct…
        let r = serde_json::from_str::<RunMode>(
            r#"{"mode":"reprise","retry_failures":true,"typo_field":1}"#,
        );
        assert!(
            r.is_err(),
            "champ inconnu accepté sur variante struct: {r:?}"
        );
        // …mais PAS sur les variantes unit (caveat serde) : Full ignore
        // les champs parasites.
        let r = serde_json::from_str::<RunMode>(r#"{"mode":"full","stray":1}"#);
        assert!(matches!(r, Ok(RunMode::Full)), "attendu Ok(Full): {r:?}");
    }
}
