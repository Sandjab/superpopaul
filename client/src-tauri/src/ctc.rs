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

/// Échéance réglementaire de la réforme française : représentée dans les
/// paliers tant qu'elle est à venir (décision du 16/07/2026).
pub const ECHEANCE_REFORME: &str = "2026-09-01";

/// Horizon de projection : les activations au-delà sont du bruit déclaratif
/// (comme les expirations « 9999-… ») et sortent des paliers affichés —
/// mais un adressage « prêt plus tard » lointain reste compté comme tel.
const HORIZON_MOIS: u32 = 24;

/// État du support CTC d'un adressage à l'instant t, calculé — jamais figé —
/// à partir de la fenêtre stockée. Expiré = dégradation simple : plus prêt,
/// point (pas d'état « était prêt » à part dans les comptages).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CtcState {
    /// Actif aujourd'hui (aucune borne, ou activation passée et expiration
    /// à venir).
    Ready,
    /// Activation à venir : bascule seul en Ready le jour venu.
    Later,
    /// Expiration dépassée.
    Expired,
}

pub fn state(activation: Option<&str>, expiration: Option<&str>, now: DateTime<Utc>) -> CtcState {
    let act = activation.and_then(parse_smp_date);
    let exp = expiration.and_then(parse_smp_date);
    // Même ordre de classement que pick_window : borne illisible = absente.
    match (act, exp) {
        (Some(a), _) if a > now => CtcState::Later,
        (_, Some(e)) if e <= now => CtcState::Expired,
        _ => CtcState::Ready,
    }
}

/// Jour (UTC) d'une date d'activation, clé des comptes par date —
/// « 2026-09-01 », tri lexicographique = tri chronologique.
pub fn activation_day(raw: &str) -> Option<String> {
    parse_smp_date(raw).map(|dt| dt.date_naive().to_string())
}

/// Comptes d'activations futures d'un même jour (adressages et lignes).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct DateCount {
    /// « AAAA-MM-JJ » (clé produite par activation_day).
    pub date: String,
    pub addr: u64,
    pub lines: u64,
}

/// Un palier de projection : au JJ/MM, `addr` adressages de PLUS seront
/// prêts (cumul depuis aujourd'hui — chaque ligne est une vérité exacte,
/// les dates omises sont absorbées par le palier suivant).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Palier {
    pub date: String,
    pub addr: u64,
    pub lines: u64,
}

/// Sélection des paliers affichés (3 max) parmi les comptes par date,
/// `counts` trié chronologiquement (sortie du snapshot) :
///   1. horizon borné (HORIZON_MOIS après `today`) ;
///   2. cumul croissant ;
///   3. ≤ 3 dates → toutes ; sinon première + dernière + échéance
///      réglementaire si candidate, complété par les plus proches ;
///   4. l'échéance réglementaire est toujours représentée tant qu'elle est
///      future (même sans activation ce jour-là : le cumul reste vrai).
pub fn paliers(counts: &[DateCount], today: chrono::NaiveDate) -> Vec<Palier> {
    let today_s = today.to_string();
    let horizon = (today + chrono::Months::new(HORIZON_MOIS)).to_string();
    let in_h: Vec<&DateCount> = counts
        .iter()
        .filter(|c| c.date.as_str() > today_s.as_str() && c.date.as_str() <= horizon.as_str())
        .collect();
    let Some(last) = in_h.last() else {
        return Vec::new(); // rien à projeter : pas de ligne fabriquée
    };
    // Dates retenues (l'ordre lexicographique des clés ISO est chronologique).
    let mut sel = std::collections::BTreeSet::from([in_h[0].date.as_str(), last.date.as_str()]);
    if ECHEANCE_REFORME > today_s.as_str() {
        sel.insert(ECHEANCE_REFORME);
    }
    for c in &in_h {
        if sel.len() >= 3 {
            break;
        }
        sel.insert(c.date.as_str());
    }
    sel.into_iter()
        .map(|d| {
            let (addr, lines) = in_h
                .iter()
                .filter(|c| c.date.as_str() <= d)
                .fold((0, 0), |(a, l), c| (a + c.addr, l + c.lines));
            Palier { date: d.to_string(), addr, lines }
        })
        .collect()
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

    // ---- État calculé -----------------------------------------------------

    #[test]
    fn etat_calcule_jamais_fige() {
        let avant = parse_smp_date("2026-08-31T23:59:59Z").unwrap();
        let apres = parse_smp_date("2026-09-01T00:00:01Z").unwrap();
        // Le même enregistrement bascule seul en Ready le jour venu.
        assert_eq!(state(Some("2026-09-01"), None, avant), CtcState::Later);
        assert_eq!(state(Some("2026-09-01"), None, apres), CtcState::Ready);
        // Expiration dépassée = dégradation simple.
        assert_eq!(
            state(Some("2020-01-01"), Some("2026-01-01"), now()),
            CtcState::Expired
        );
        // Sans borne, ou bornes illisibles : prêt.
        assert_eq!(state(None, None, now()), CtcState::Ready);
        assert_eq!(state(Some("n/a"), Some("n/a"), now()), CtcState::Ready);
        // Expiration 9999 : jamais expirée, aucun cas spécial.
        assert_eq!(
            state(Some("2020-01-01"), Some("9999-12-31T23:59:59.999999Z"), now()),
            CtcState::Ready
        );
    }

    #[test]
    fn jour_d_activation_normalise() {
        assert_eq!(activation_day("2026-09-01T00:00:00Z").as_deref(), Some("2026-09-01"));
        assert_eq!(activation_day("2026-09-01").as_deref(), Some("2026-09-01"));
        assert_eq!(activation_day("n/a"), None);
    }

    // ---- Paliers ----------------------------------------------------------

    fn dc(date: &str, addr: u64, lines: u64) -> DateCount {
        DateCount { date: date.into(), addr, lines }
    }

    fn today() -> chrono::NaiveDate {
        "2026-07-16".parse().unwrap()
    }

    #[test]
    fn paliers_cumules_toutes_dates_si_trois_ou_moins() {
        // Le cas réel du 16/07/2026 : 498 au 01/09, 37 au 22/09.
        assert_eq!(
            paliers(&[dc("2026-09-01", 498, 514), dc("2026-09-22", 37, 38)], today()),
            vec![
                Palier { date: "2026-09-01".into(), addr: 498, lines: 514 },
                Palier { date: "2026-09-22".into(), addr: 535, lines: 552 },
            ]
        );
    }

    #[test]
    fn paliers_plafonnes_premiere_echeance_derniere() {
        // 5 dates dont l'échéance : première + échéance + dernière (cumul
        // total) — les intermédiaires sont absorbées par le palier suivant.
        let counts = [
            dc("2026-08-01", 10, 10),
            dc("2026-08-15", 5, 5),
            dc("2026-09-01", 100, 100),
            dc("2026-10-01", 7, 7),
            dc("2027-03-15", 3, 3),
        ];
        assert_eq!(
            paliers(&counts, today()),
            vec![
                Palier { date: "2026-08-01".into(), addr: 10, lines: 10 },
                Palier { date: "2026-09-01".into(), addr: 115, lines: 115 },
                Palier { date: "2027-03-15".into(), addr: 125, lines: 125 },
            ]
        );
    }

    #[test]
    fn paliers_sans_echeance_deux_premieres_puis_derniere() {
        // Échéance passée (today après le 01/09/2026) : règle de base.
        let apres_echeance: chrono::NaiveDate = "2026-10-01".parse().unwrap();
        let counts = [
            dc("2026-11-01", 1, 1),
            dc("2026-12-01", 2, 2),
            dc("2027-01-01", 3, 3),
            dc("2027-02-01", 4, 4),
        ];
        assert_eq!(
            paliers(&counts, apres_echeance),
            vec![
                Palier { date: "2026-11-01".into(), addr: 1, lines: 1 },
                Palier { date: "2026-12-01".into(), addr: 3, lines: 3 },
                Palier { date: "2027-02-01".into(), addr: 10, lines: 10 },
            ]
        );
    }

    #[test]
    fn echeance_representee_meme_sans_activation_ce_jour_la() {
        // Aucune donnée au 01/09 mais l'échéance est future : ligne ajoutée,
        // cumul vrai (tout ce qui est ≤ 01/09).
        let counts = [dc("2026-08-01", 10, 10), dc("2026-10-01", 5, 5)];
        assert_eq!(
            paliers(&counts, today()),
            vec![
                Palier { date: "2026-08-01".into(), addr: 10, lines: 10 },
                Palier { date: "2026-09-01".into(), addr: 10, lines: 10 },
                Palier { date: "2026-10-01".into(), addr: 15, lines: 15 },
            ]
        );
    }

    #[test]
    fn horizon_deux_ans_borne_les_paliers() {
        // L'activation à 10 ans (bruit déclaratif) sort des paliers ; la
        // dernière ligne reste dans l'horizon.
        let counts = [dc("2026-09-01", 498, 514), dc("2036-05-18", 52, 52)];
        assert_eq!(
            paliers(&counts, today()),
            vec![Palier { date: "2026-09-01".into(), addr: 498, lines: 514 }]
        );
        // Tout hors horizon : aucun palier (la carte Projection n'apparaît pas).
        assert_eq!(paliers(&[dc("2036-05-18", 52, 52)], today()), vec![]);
    }

    #[test]
    fn aucune_activation_future_aucun_palier() {
        // Pas de ligne « échéance » fabriquée quand il n'y a rien à projeter.
        assert_eq!(paliers(&[], today()), vec![]);
    }
}
