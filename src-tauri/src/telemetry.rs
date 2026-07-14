use serde::Serialize;
use std::collections::{BTreeMap, VecDeque};
use std::sync::Mutex;
use std::time::Instant;

/// Fenêtre glissante pour les débits instantanés.
const WINDOW_S: f64 = 10.0;

/// Bornes (incluses) des tranches de l'histogramme de latences, en ms.
/// Un dernier bucket ouvert (le_ms = u32::MAX) reçoit tout ce qui dépasse.
const LAT_HIST_BOUNDS_MS: [u32; 7] = [50, 100, 200, 500, 1000, 2000, 5000];

/// Motifs d'erreur distincts conservés au maximum : les messages peuvent
/// contenir des parties variables (identifiants…) — au-delà, tout part dans
/// « (autres) » pour borner mémoire et taille du snapshot.
const MAX_ERROR_MOTIFS: usize = 20;
const AUTRES: &str = "(autres)";

pub struct Telemetry {
    total: u64,
    inner: Mutex<Inner>,
}

struct Inner {
    done: u64,
    exists: u64,
    ctc: u64,
    failed: u64,
    http: BTreeMap<u16, u64>,
    pa: BTreeMap<String, u64>, // nom de PA → adressages routés vers elle
    errors: BTreeMap<String, u64>, // motif d'échec → adressages concernés
    lines: LineWeights,        // mêmes compteurs, pondérés en lignes de fichier
    latencies_ms: Vec<u32>,
    calls: VecDeque<(Instant, u32)>, // (instant, adressages traités par l'appel)
    /// Temps actif cumulé (pauses utilisateur et suspensions système exclues),
    /// échantillonné par `tick_active` — voir le superviseur du moteur.
    active: std::time::Duration,
    last_tick: Instant,
}

#[derive(Debug, Clone, Serialize)]
pub struct LatStats {
    pub min: u32,
    pub mean: u32,
    pub p50: u32,
    pub p90: u32,
    pub p99: u32,
    pub max: u32,
}

/// Poids en lignes de fichier des adressages d'un appel : un même adressage
/// peut apparaître sur plusieurs lignes du fichier d'entrée.
#[derive(Debug, Clone, Copy, Default)]
pub struct LineWeights {
    pub done: u64,
    pub exists: u64,
    pub ctc: u64,
}

/// Un libellé (nom de PA, motif d'échec…) et un nombre d'adressages.
#[derive(Debug, Clone, Serialize)]
pub struct NamedCount {
    pub name: String,
    pub count: u64,
}

/// Tranche de l'histogramme de latences : appels dont la latence est
/// ≤ le_ms (u32::MAX pour le bucket ouvert au-delà de la dernière borne).
#[derive(Debug, Clone, Serialize)]
pub struct HistBucket {
    pub le_ms: u32,
    pub count: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Snapshot {
    pub done: u64,
    pub total: u64,
    pub exists: u64,
    pub ctc: u64,
    pub failed: u64,
    /// Équivalents en lignes de fichier de done/exists/ctc.
    pub done_lines: u64,
    pub exists_lines: u64,
    pub ctc_lines: u64,
    pub http: BTreeMap<u16, u64>,
    /// PA découvertes, classées par représentativité décroissante
    /// (à égalité : ordre alphabétique, pour un affichage stable).
    pub pa: Vec<NamedCount>,
    /// Motifs d'échec (erreurs item et échecs HTTP définitifs), classés par
    /// fréquence décroissante — mêmes règles de tri que `pa`.
    pub errors: Vec<NamedCount>,
    pub latency: Option<LatStats>,
    /// Répartition des appels par tranche de latence (bornes fixes,
    /// cumulée depuis le début du run). Toujours 8 buckets.
    pub latency_hist: Vec<HistBucket>,
    /// Concurrence effective autorisée par l'AIMD et plafond configuré.
    /// La télémétrie ne les connaît pas : le superviseur du moteur les
    /// renseigne à l'émission (0/0 sur un snapshot pris hors moteur).
    pub concurrency_allowed: u32,
    pub concurrency_max: u32,
    pub req_per_s: f64,
    pub addr_per_s: f64,
    pub eta_s: Option<u64>,
    /// Durée active du run en secondes (pauses et suspensions exclues).
    pub active_s: f64,
}

impl Telemetry {
    pub fn new(total: u64) -> Self {
        Telemetry {
            total,
            inner: Mutex::new(Inner {
                done: 0,
                exists: 0,
                ctc: 0,
                failed: 0,
                http: BTreeMap::new(),
                pa: BTreeMap::new(),
                errors: BTreeMap::new(),
                lines: LineWeights::default(),
                latencies_ms: Vec::new(),
                calls: VecDeque::new(),
                active: std::time::Duration::ZERO,
                last_tick: Instant::now(),
            }),
        }
    }

    /// Échantillonne le temps actif : ajoute l'intervalle écoulé depuis le
    /// tick précédent si le moteur était actif (ni pause utilisateur, ni
    /// suspension système) sur cet intervalle. Appelé 4×/s par le
    /// superviseur, plus un tick final à l'émission de Finished pour ne pas
    /// perdre la dernière tranche (un run éclair n'aurait sinon aucun tick).
    pub fn tick_active(&self, active: bool) {
        let mut i = self.inner.lock().unwrap();
        let now = Instant::now();
        if active {
            let delta = now - i.last_tick;
            i.active += delta;
        }
        i.last_tick = now;
    }

    /// Un appel HTTP abouti (200) : addr adressages traités, dont `exists`
    /// présents Peppol, `ctc` supportant CTC-FR, `failed` en erreur item.
    /// `pa` : adressages de l'appel agrégés par nom de PA.
    /// `lines` : les mêmes compteurs pondérés en lignes de fichier.
    /// `errors` : adressages en erreur item, agrégés par motif.
    #[allow(clippy::too_many_arguments)]
    pub fn record_call(
        &self,
        http_status: u16,
        latency_ms: u64,
        addr: u32,
        exists: u32,
        ctc: u32,
        failed: u32,
        pa: &BTreeMap<String, u32>,
        lines: LineWeights,
        errors: &BTreeMap<String, u32>,
    ) {
        let mut i = self.inner.lock().unwrap();
        i.done += addr as u64;
        i.exists += exists as u64;
        i.ctc += ctc as u64;
        i.failed += failed as u64;
        i.lines.done += lines.done;
        i.lines.exists += lines.exists;
        i.lines.ctc += lines.ctc;
        *i.http.entry(http_status).or_insert(0) += 1;
        for (name, n) in pa {
            *i.pa.entry(name.clone()).or_insert(0) += *n as u64;
        }
        for (motif, n) in errors {
            bump_error(&mut i.errors, motif, *n as u64);
        }
        i.latencies_ms.push(latency_ms.min(u32::MAX as u64) as u32);
        i.calls.push_back((Instant::now(), addr));
    }

    /// Un appel HTTP en erreur (0 = réseau) : compté. `addr_failed` vaut 0
    /// pour une erreur retentable (aucune progression : le retry s'en
    /// chargera) et la taille du paquet pour un échec définitif
    /// (ApiError::Client) — ces adressages sont alors "done" (en échec) même
    /// si aucune requête ne les re-tentera, pour que la progression/ETA
    /// restent cohérents.
    /// `lines_failed` : lignes de fichier couvertes par les adressages en
    /// échec définitif (0 pour une erreur retentable, comme `addr_failed`).
    pub fn record_error(&self, http_status: u16, addr_failed: u32, lines_failed: u64) {
        let mut i = self.inner.lock().unwrap();
        i.done += addr_failed as u64;
        i.failed += addr_failed as u64;
        i.lines.done += lines_failed;
        if addr_failed > 0 {
            // Échec définitif : le motif est le statut HTTP du paquet.
            bump_error(&mut i.errors, &format!("HTTP {http_status}"), addr_failed as u64);
        }
        *i.http.entry(http_status).or_insert(0) += 1;
        // La fenêtre crédite les adressages en échec définitif : ils comptent
        // dans le débit (addr_per_s), donc dans l'ETA, comme la progression.
        i.calls.push_back((Instant::now(), addr_failed));
    }

    pub fn snapshot(&self) -> Snapshot {
        // Sous le verrou : purge de la fenêtre + copies. Le tri des latences
        // (coûteux à grand volume) se fait HORS verrou pour ne pas bloquer
        // les record_call des workers.
        let (done, exists, ctc, failed, http, pa, errors, lines, latencies, req_per_s, addr_per_s, active_s) = {
            let mut i = self.inner.lock().unwrap();
            let now = Instant::now();
            while let Some((t, _)) = i.calls.front() {
                if now.duration_since(*t).as_secs_f64() > WINDOW_S {
                    i.calls.pop_front();
                } else {
                    break;
                }
            }
            // Fenêtre effective : depuis le plus vieil appel conservé (évite de
            // diviser par 10 s quand le run vient de démarrer).
            let span = i
                .calls
                .front()
                .map(|(t, _)| now.duration_since(*t).as_secs_f64().max(0.25))
                .unwrap_or(1.0);
            let req_per_s = i.calls.len() as f64 / span;
            let addr_in_window: u64 = i.calls.iter().map(|(_, a)| *a as u64).sum();
            let addr_per_s = addr_in_window as f64 / span;
            (
                i.done,
                i.exists,
                i.ctc,
                i.failed,
                i.http.clone(),
                i.pa.clone(),
                i.errors.clone(),
                i.lines,
                i.latencies_ms.clone(),
                req_per_s,
                addr_per_s,
                i.active.as_secs_f64(),
            )
        }; // verrou relâché ici
        // Classement par représentativité (BTreeMap garantit déjà l'ordre
        // alphabétique, que sort_by_key stable préserve à égalité de compte).
        let ranked = |map: BTreeMap<String, u64>| -> Vec<NamedCount> {
            let mut v: Vec<NamedCount> = map
                .into_iter()
                .map(|(name, count)| NamedCount { name, count })
                .collect();
            v.sort_by_key(|p| std::cmp::Reverse(p.count));
            v
        };
        let (pa, errors) = (ranked(pa), ranked(errors));
        let remaining = self.total.saturating_sub(done);
        let eta_s = if addr_per_s > 0.0 && remaining > 0 {
            Some((remaining as f64 / addr_per_s).round() as u64)
        } else {
            None
        };
        Snapshot {
            done,
            total: self.total,
            exists,
            ctc,
            failed,
            done_lines: lines.done,
            exists_lines: lines.exists,
            ctc_lines: lines.ctc,
            http,
            pa,
            errors,
            latency: lat_stats(&latencies),
            latency_hist: lat_hist(&latencies),
            concurrency_allowed: 0,
            concurrency_max: 0,
            req_per_s,
            addr_per_s,
            eta_s,
            active_s,
        }
    }
}

/// Incrémente un motif d'erreur en bornant le nombre de motifs distincts :
/// au-delà de MAX_ERROR_MOTIFS, le compte part dans « (autres) ».
fn bump_error(errors: &mut BTreeMap<String, u64>, motif: &str, n: u64) {
    let distinct = errors.len() - usize::from(errors.contains_key(AUTRES));
    if errors.contains_key(motif) || distinct < MAX_ERROR_MOTIFS {
        *errors.entry(motif.to_string()).or_insert(0) += n;
    } else {
        *errors.entry(AUTRES.to_string()).or_insert(0) += n;
    }
}

fn lat_hist(lat: &[u32]) -> Vec<HistBucket> {
    let mut hist: Vec<HistBucket> = LAT_HIST_BOUNDS_MS
        .iter()
        .map(|&le_ms| HistBucket { le_ms, count: 0 })
        .chain(std::iter::once(HistBucket {
            le_ms: u32::MAX,
            count: 0,
        }))
        .collect();
    for &ms in lat {
        let idx = LAT_HIST_BOUNDS_MS
            .iter()
            .position(|&b| ms <= b)
            .unwrap_or(LAT_HIST_BOUNDS_MS.len());
        hist[idx].count += 1;
    }
    hist
}

fn lat_stats(lat: &[u32]) -> Option<LatStats> {
    if lat.is_empty() {
        return None;
    }
    let mut v = lat.to_vec();
    v.sort_unstable();
    let pct = |p: f64| v[((v.len() as f64 - 1.0) * p / 100.0) as usize];
    Some(LatStats {
        min: v[0],
        mean: (v.iter().map(|&x| x as u64).sum::<u64>() / v.len() as u64) as u32,
        p50: pct(50.0),
        p90: pct(90.0),
        p99: pct(99.0),
        max: *v.last().unwrap(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vide() -> BTreeMap<String, u32> {
        BTreeMap::new()
    }

    #[test]
    fn temps_actif_exclut_les_periodes_suspendues() {
        // La durée affichée en fin de run ne doit compter que le temps où le
        // moteur travaille : un intervalle échantillonné inactif (pause ou
        // suspension) n'ajoute RIEN au cumul.
        let t = Telemetry::new(10);
        std::thread::sleep(std::time::Duration::from_millis(20));
        t.tick_active(true);
        let apres_actif = t.snapshot().active_s;
        assert!(apres_actif >= 0.015, "{apres_actif}");
        std::thread::sleep(std::time::Duration::from_millis(20));
        t.tick_active(false); // période suspendue
        assert_eq!(t.snapshot().active_s, apres_actif);
        std::thread::sleep(std::time::Duration::from_millis(20));
        t.tick_active(true); // le cumul repart après la suspension
        assert!(t.snapshot().active_s > apres_actif);
    }

    #[test]
    fn compteurs_et_pourcentages() {
        let t = Telemetry::new(1000);
        // 2 appels : 50 adressages ok (30 existent, 20 ctc), puis 25 ok + 5 échecs
        t.record_call(200, 120, 50, 30, 20, 0, &vide(), LineWeights::default(), &vide());
        t.record_call(200, 250, 30, 10, 5, 5, &vide(), LineWeights::default(), &vide());
        let s = t.snapshot();
        assert_eq!(s.total, 1000);
        assert_eq!(s.done, 80);
        assert_eq!(s.exists, 40);
        assert_eq!(s.ctc, 25);
        assert_eq!(s.failed, 5);
        assert_eq!(s.http.get(&200), Some(&2));
    }

    #[test]
    fn erreurs_comptees_sans_progression() {
        let t = Telemetry::new(100);
        t.record_error(429, 0, 0);
        t.record_error(0, 0, 0); // réseau
        let s = t.snapshot();
        assert_eq!(s.done, 0);
        assert_eq!(s.http.get(&429), Some(&1));
        assert_eq!(s.http.get(&0), Some(&1));
    }

    #[test]
    fn compteurs_en_lignes_de_fichier() {
        // Les lignes pondèrent les adressages par leur multiplicité dans le
        // fichier : un appel de 2 adressages (dont 1 présent Peppol, 1 CTC)
        // peut couvrir 7 lignes dont 5 présentes et 3 CTC. Un échec d'appel
        // définitif couvre aussi ses lignes (dénominateur cohérent).
        let t = Telemetry::new(10);
        t.record_call(200, 100, 2, 1, 1, 0, &vide(),
            LineWeights { done: 7, exists: 5, ctc: 3 }, &vide());
        t.record_error(400, 2, 4);
        let s = t.snapshot();
        assert_eq!((s.done_lines, s.exists_lines, s.ctc_lines), (11, 5, 3));
    }

    #[test]
    fn classement_pa_par_representativite() {
        // Les comptes s'accumulent entre appels ; le snapshot classe par
        // nombre d'adressages décroissant (rang 1 = la plus représentée),
        // à égalité par nom pour un ordre stable.
        let t = Telemetry::new(100);
        t.record_call(200, 100, 8, 8, 0, 0, &BTreeMap::from([("Beta".into(), 5), ("Alpha".into(), 3)]),
            LineWeights::default(), &vide());
        t.record_call(200, 100, 4, 4, 0, 0, &BTreeMap::from([("Alpha".into(), 4), ("Gamma".into(), 5)]),
            LineWeights::default(), &vide());
        let s = t.snapshot();
        let ranking: Vec<(&str, u64)> = s.pa.iter().map(|p| (p.name.as_str(), p.count)).collect();
        assert_eq!(
            ranking,
            vec![("Alpha", 7), ("Beta", 5), ("Gamma", 5)] // Beta avant Gamma : égalité → ordre alphabétique
        );
    }

    #[test]
    fn percentiles_latence() {
        let t = Telemetry::new(100);
        for (i, ms) in (1..=100u64).enumerate() {
            t.record_call(200, ms, 1, 0, 0, 0, &vide(), LineWeights::default(), &vide());
            let _ = i;
        }
        let s = t.snapshot();
        let l = s.latency.unwrap();
        assert_eq!(l.min, 1);
        assert_eq!(l.max, 100);
        assert_eq!(l.p50, 50);
        assert_eq!(l.p90, 90);
        assert_eq!(l.p99, 99);
    }

    #[test]
    fn top_erreurs_agregees_et_classees() {
        let t = Telemetry::new(100);
        // 2 erreurs item « timeout SMP », 1 « participant invalide » dans un
        // appel 200 ; puis un échec HTTP définitif de 10 adressages.
        t.record_call(200, 100, 5, 2, 0, 3, &vide(), LineWeights::default(),
            &BTreeMap::from([("timeout SMP".into(), 2), ("participant invalide".into(), 1)]));
        t.record_error(404, 10, 10);
        let s = t.snapshot();
        let top: Vec<(&str, u64)> = s.errors.iter().map(|e| (e.name.as_str(), e.count)).collect();
        assert_eq!(
            top,
            vec![("HTTP 404", 10), ("timeout SMP", 2), ("participant invalide", 1)]
        );
    }

    #[test]
    fn erreur_reseau_retentable_sans_motif() {
        // Une erreur retentable (addr_failed = 0) ne crée pas de motif :
        // rien n'a définitivement échoué.
        let t = Telemetry::new(100);
        t.record_error(0, 0, 0);
        assert!(t.snapshot().errors.is_empty());
    }

    #[test]
    fn motifs_d_erreur_bornes_avec_bucket_autres() {
        // Les messages d'erreur peuvent contenir des parties variables :
        // au-delà de 20 motifs distincts, tout part dans « (autres) » pour
        // borner mémoire et taille du snapshot.
        let t = Telemetry::new(1000);
        for i in 0..25 {
            t.record_call(200, 100, 1, 0, 0, 1, &vide(), LineWeights::default(),
                &BTreeMap::from([(format!("motif {i:02}"), 1u32)]));
        }
        let s = t.snapshot();
        assert_eq!(s.errors.len(), 21); // 20 motifs + « (autres) »
        let autres = s.errors.iter().find(|e| e.name == "(autres)").unwrap();
        assert_eq!(autres.count, 5);
    }

    #[test]
    fn histogramme_de_latences_par_tranches() {
        let t = Telemetry::new(10);
        for ms in [10u64, 60, 150, 700, 6000] {
            t.record_call(200, ms, 1, 0, 0, 0, &vide(), LineWeights::default(), &vide());
        }
        let hist = t.snapshot().latency_hist;
        // Bornes fixes ≤50, ≤100, ≤200, ≤500, ≤1000, ≤2000, ≤5000, au-delà.
        assert_eq!(hist.len(), 8);
        let counts: Vec<u64> = hist.iter().map(|b| b.count).collect();
        assert_eq!(counts, vec![1, 1, 1, 0, 1, 0, 0, 1]);
        assert_eq!(hist[0].le_ms, 50);
        assert_eq!(hist[7].le_ms, u32::MAX); // bucket ouvert « > 5000 »
    }

    #[test]
    fn latence_none_avant_le_premier_appel() {
        let s = Telemetry::new(10).snapshot();
        assert!(s.latency.is_none());
        assert!(s.latency_hist.iter().all(|b| b.count == 0));
        assert_eq!(s.req_per_s, 0.0);
        assert_eq!(s.addr_per_s, 0.0);
        assert!(s.eta_s.is_none());
    }

    #[test]
    fn adr_par_s_distinct_de_req_par_s() {
        // 1 requête portant 50 adressages : addr_per_s ≈ 50 × req_per_s.
        // Un swap des deux champs ne passerait pas ce test.
        let t = Telemetry::new(1000);
        t.record_call(200, 100, 50, 0, 0, 0, &vide(), LineWeights::default(), &vide());
        let s = t.snapshot();
        assert!(s.req_per_s > 0.0);
        let ratio = s.addr_per_s / s.req_per_s;
        assert!((49.0..=51.0).contains(&ratio), "ratio: {ratio}");
    }

    #[test]
    fn eta_present_des_qu_il_y_a_du_debit() {
        let t = Telemetry::new(1000);
        assert!(t.snapshot().eta_s.is_none()); // rien traité
        t.record_call(200, 100, 100, 0, 0, 0, &vide(), LineWeights::default(), &vide());
        let s = t.snapshot();
        assert!(s.addr_per_s > 0.0);
        assert!(s.eta_s.is_some());
    }
}
