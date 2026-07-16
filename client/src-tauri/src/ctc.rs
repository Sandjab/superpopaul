//! Fenêtre temporelle du support CTC (v0.4.0). La spec SMP borne le support
//! d'un doctype dans le temps (ServiceActivationDate/ServiceExpirationDate) :
//! on stocke les dates, jamais l'état — un adressage « activation 01/09 »
//! bascule seul en « prêt » le jour venu. Parité serveur :
//! peppol_api.ctc_window (même priorité, mêmes règles de parsing).

use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};

/// Date SMP ISO 8601 : « Z », offset explicite, microsecondes, datetime naïf
/// (supposé UTC) ou date seule (minuit UTC). Valeur illisible = borne absente
/// — même règle que le serveur.
pub fn parse_smp_date(raw: &str) -> Option<DateTime<Utc>> {
    let raw = raw.trim();
    if let Ok(dt) = DateTime::parse_from_rfc3339(raw) {
        return Some(dt.with_timezone(&Utc));
    }
    if let Ok(dt) = NaiveDateTime::parse_from_str(raw, "%Y-%m-%dT%H:%M:%S%.f") {
        return Some(dt.and_utc());
    }
    if let Ok(d) = NaiveDate::parse_from_str(raw, "%Y-%m-%d") {
        return Some(d.and_hms_opt(0, 0, 0).expect("minuit valide").and_utc());
    }
    None
}

/// Fenêtre (activation, expiration — chaînes brutes) de l'endpoint CTC
/// PERTINENT parmi ceux du doctype : celui qui détermine l'état. Priorité :
/// actif (expiration la plus lointaine, « sans limite » gagnant) > activation
/// future la plus proche > expiré le plus récent. Un min/max naïf sur
/// l'ensemble fabriquerait des fenêtres fantômes (actif apparent entre un
/// expiré et un futur).
pub fn pick_window(
    endpoints: &[(Option<String>, Option<String>)],
    now: DateTime<Utc>,
) -> (Option<String>, Option<String>) {
    // (act, exp, brutes réémises seulement si parsables)
    struct W<'a> {
        act: Option<DateTime<Utc>>,
        exp: Option<DateTime<Utc>>,
        raw_act: Option<&'a str>,
        raw_exp: Option<&'a str>,
    }
    let (mut actifs, mut futurs, mut expires) = (Vec::new(), Vec::new(), Vec::new());
    for (raw_act, raw_exp) in endpoints {
        let act = raw_act.as_deref().and_then(parse_smp_date);
        let exp = raw_exp.as_deref().and_then(parse_smp_date);
        let w = W {
            act,
            exp,
            raw_act: act.and(raw_act.as_deref()),
            raw_exp: exp.and(raw_exp.as_deref()),
        };
        match (act, exp) {
            (Some(a), _) if a > now => futurs.push(w),
            (_, Some(e)) if e <= now => expires.push(w),
            _ => actifs.push(w),
        }
    }
    let best = if !actifs.is_empty() {
        actifs
            .into_iter()
            .max_by_key(|w| w.exp.unwrap_or(DateTime::<Utc>::MAX_UTC))
    } else if !futurs.is_empty() {
        futurs.into_iter().min_by_key(|w| w.act.expect("futur daté"))
    } else {
        expires.into_iter().max_by_key(|w| w.exp.expect("expiré daté"))
    };
    match best {
        Some(w) => (w.raw_act.map(str::to_string), w.raw_exp.map(str::to_string)),
        None => (None, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> DateTime<Utc> {
        parse_smp_date("2026-07-16T12:00:00Z").unwrap()
    }

    fn ep(act: Option<&str>, exp: Option<&str>) -> (Option<String>, Option<String>) {
        (act.map(str::to_string), exp.map(str::to_string))
    }

    #[test]
    fn parse_formats_smp() {
        // Date seule, Z, offset, microsecondes « sans limite », naïf.
        assert!(parse_smp_date("2026-09-01").is_some());
        assert!(parse_smp_date("2026-09-01T00:00:00Z").is_some());
        assert!(parse_smp_date("2026-07-09T00:00:00+00:00").is_some());
        assert!(parse_smp_date("9999-12-31T23:59:59.999999Z").is_some());
        assert!(parse_smp_date("2026-05-18T15:12:28").is_some());
        assert_eq!(
            parse_smp_date("2026-09-01"),
            parse_smp_date("2026-09-01T00:00:00Z")
        );
        assert!(parse_smp_date("n/a").is_none());
        assert!(parse_smp_date("").is_none());
    }

    #[test]
    fn fenetre_endpoint_actif_emise_brute() {
        assert_eq!(
            pick_window(&[ep(Some("2026-01-01"), Some("2027-01-01"))], now()),
            (Some("2026-01-01".into()), Some("2027-01-01".into()))
        );
    }

    #[test]
    fn actif_sans_dates_gagne_sur_activation_future() {
        // Prêt sans limite : aucune fenêtre à émettre.
        assert_eq!(
            pick_window(&[ep(None, None), ep(Some("2026-09-01"), None)], now()),
            (None, None)
        );
    }

    #[test]
    fn expire_plus_futur_choisit_le_futur() {
        // Un min/max naïf donnerait (2025-01-01, 2026-03-05) : « prêt », faux.
        assert_eq!(
            pick_window(
                &[
                    ep(Some("2025-01-01"), Some("2026-03-05")),
                    ep(Some("2026-09-01"), None),
                ],
                now()
            ),
            (Some("2026-09-01".into()), None)
        );
    }

    #[test]
    fn deux_futurs_activation_la_plus_proche() {
        assert_eq!(
            pick_window(
                &[ep(Some("2026-09-22"), None), ep(Some("2026-09-01"), None)],
                now()
            ),
            (Some("2026-09-01".into()), None)
        );
    }

    #[test]
    fn deux_actifs_expiration_la_plus_lointaine() {
        assert_eq!(
            pick_window(
                &[
                    ep(Some("2025-01-01"), Some("2026-08-01")),
                    ep(Some("2025-01-01"), Some("2036-01-01")),
                ],
                now()
            ),
            (Some("2025-01-01".into()), Some("2036-01-01".into()))
        );
    }

    #[test]
    fn tous_expires_le_plus_recent() {
        assert_eq!(
            pick_window(
                &[
                    ep(Some("2024-01-01"), Some("2025-01-01")),
                    ep(Some("2024-01-01"), Some("2026-06-01")),
                ],
                now()
            ),
            (Some("2024-01-01".into()), Some("2026-06-01".into()))
        );
    }

    #[test]
    fn borne_illisible_ignoree() {
        // Activation imparsable = absente : endpoint actif, seule
        // l'expiration (valide) est émise.
        assert_eq!(
            pick_window(&[ep(Some("n/a"), Some("2027-01-01"))], now()),
            (None, Some("2027-01-01".into()))
        );
    }

    #[test]
    fn aucun_endpoint_aucune_fenetre() {
        assert_eq!(pick_window(&[], now()), (None, None));
    }
}
