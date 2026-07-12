use serde::Serialize;
use std::collections::{BTreeMap, VecDeque};
use std::sync::Mutex;
use std::time::Instant;

/// Fenêtre glissante pour les débits instantanés.
const WINDOW_S: f64 = 10.0;

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
    latencies_ms: Vec<u32>,
    calls: VecDeque<(Instant, u32)>, // (instant, adressages traités par l'appel)
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

#[derive(Debug, Clone, Serialize)]
pub struct Snapshot {
    pub done: u64,
    pub total: u64,
    pub exists: u64,
    pub ctc: u64,
    pub failed: u64,
    pub http: BTreeMap<u16, u64>,
    pub latency: Option<LatStats>,
    pub req_per_s: f64,
    pub addr_per_s: f64,
    pub eta_s: Option<u64>,
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
                latencies_ms: Vec::new(),
                calls: VecDeque::new(),
            }),
        }
    }

    /// Un appel HTTP abouti (200) : addr adressages traités, dont `exists`
    /// présents Peppol, `ctc` supportant CTC-FR, `failed` en erreur item.
    pub fn record_call(
        &self,
        http_status: u16,
        latency_ms: u64,
        addr: u32,
        exists: u32,
        ctc: u32,
        failed: u32,
    ) {
        let mut i = self.inner.lock().unwrap();
        i.done += addr as u64;
        i.exists += exists as u64;
        i.ctc += ctc as u64;
        i.failed += failed as u64;
        *i.http.entry(http_status).or_insert(0) += 1;
        i.latencies_ms.push(latency_ms.min(u32::MAX as u64) as u32);
        i.calls.push_back((Instant::now(), addr));
    }

    /// Un appel HTTP en erreur (0 = réseau) : compté, aucune progression.
    pub fn record_error(&self, http_status: u16) {
        let mut i = self.inner.lock().unwrap();
        *i.http.entry(http_status).or_insert(0) += 1;
        i.calls.push_back((Instant::now(), 0));
    }

    pub fn snapshot(&self) -> Snapshot {
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
        let remaining = self.total.saturating_sub(i.done);
        let eta_s = if addr_per_s > 0.0 && remaining > 0 {
            Some((remaining as f64 / addr_per_s).round() as u64)
        } else {
            None
        };
        Snapshot {
            done: i.done,
            total: self.total,
            exists: i.exists,
            ctc: i.ctc,
            failed: i.failed,
            http: i.http.clone(),
            latency: lat_stats(&i.latencies_ms),
            req_per_s,
            addr_per_s,
            eta_s,
        }
    }
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

    #[test]
    fn compteurs_et_pourcentages() {
        let t = Telemetry::new(1000);
        // 2 appels : 50 adressages ok (30 existent, 20 ctc), puis 25 ok + 5 échecs
        t.record_call(200, 120, 50, 30, 20, 0);
        t.record_call(200, 250, 30, 10, 5, 5);
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
        t.record_error(429);
        t.record_error(0); // réseau
        let s = t.snapshot();
        assert_eq!(s.done, 0);
        assert_eq!(s.http.get(&429), Some(&1));
        assert_eq!(s.http.get(&0), Some(&1));
    }

    #[test]
    fn percentiles_latence() {
        let t = Telemetry::new(100);
        for (i, ms) in (1..=100u64).enumerate() {
            t.record_call(200, ms, 1, 0, 0, 0);
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
    fn eta_present_des_qu_il_y_a_du_debit() {
        let t = Telemetry::new(1000);
        assert!(t.snapshot().eta_s.is_none()); // rien traité
        t.record_call(200, 100, 100, 0, 0, 0);
        let s = t.snapshot();
        assert!(s.addr_per_s > 0.0);
        assert!(s.eta_s.is_some());
    }
}
