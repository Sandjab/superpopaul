use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

/// Concurrence adaptative : ÷2 sur rate-limit, +1 après 50 succès consécutifs
/// (AIMD), bornée à [1, max].
pub struct Aimd {
    allowed: AtomicU32,
    max: u32,
    ok_streak: AtomicU32,
}

const AIMD_STREAK: u32 = 50;

impl Aimd {
    pub fn new(max: u32) -> Self {
        Aimd {
            allowed: AtomicU32::new(max.max(1)),
            max: max.max(1),
            ok_streak: AtomicU32::new(0),
        }
    }

    pub fn allowed(&self) -> u32 {
        self.allowed.load(Ordering::Relaxed)
    }

    pub fn on_rate_limited(&self) {
        self.ok_streak.store(0, Ordering::Relaxed);
        let cur = self.allowed.load(Ordering::Relaxed);
        self.allowed.store((cur / 2).max(1), Ordering::Relaxed);
    }

    pub fn on_success(&self) {
        let streak = self.ok_streak.fetch_add(1, Ordering::Relaxed) + 1;
        if streak >= AIMD_STREAK {
            self.ok_streak.store(0, Ordering::Relaxed);
            let cur = self.allowed.load(Ordering::Relaxed);
            self.allowed
                .store((cur + 1).min(self.max), Ordering::Relaxed);
        }
    }
}

/// Circuit breaker : ouvre après `threshold` échecs consécutifs (5xx/réseau),
/// avec un backoff 30 s doublé à chaque ouverture (plafond 300 s).
pub struct Breaker {
    threshold: u32,
    consecutive: u32,
    opens: u32,
}

impl Breaker {
    pub fn new(threshold: u32) -> Self {
        Breaker {
            threshold,
            consecutive: 0,
            opens: 0,
        }
    }

    pub fn on_failure(&mut self) -> Option<Duration> {
        self.consecutive += 1;
        if self.consecutive >= self.threshold {
            self.consecutive = 0;
            let secs = (30u64 << self.opens.min(3)).min(300);
            self.opens += 1;
            Some(Duration::from_secs(secs))
        } else {
            None
        }
    }

    pub fn on_success(&mut self) {
        self.consecutive = 0;
        self.opens = 0;
    }
}

#[cfg(test)]
mod tests_ctrl {
    use super::*;

    #[test]
    fn aimd_divise_par_deux_sur_429_et_remonte_de_un() {
        let a = Aimd::new(16);
        assert_eq!(a.allowed(), 16);
        a.on_rate_limited();
        assert_eq!(a.allowed(), 8);
        a.on_rate_limited();
        assert_eq!(a.allowed(), 4);
        // 50 succès consécutifs → +1, plafonné au max initial
        for _ in 0..50 {
            a.on_success();
        }
        assert_eq!(a.allowed(), 5);
        for _ in 0..(50 * 20) {
            a.on_success();
        }
        assert_eq!(a.allowed(), 16); // jamais au-dessus du max configuré
    }

    #[test]
    fn aimd_ne_descend_jamais_sous_un() {
        let a = Aimd::new(2);
        a.on_rate_limited();
        a.on_rate_limited();
        a.on_rate_limited();
        assert_eq!(a.allowed(), 1);
    }

    #[test]
    fn breaker_ouvre_apres_seuil_et_backoff_croissant() {
        let mut b = Breaker::new(3);
        assert_eq!(b.on_failure(), None);
        assert_eq!(b.on_failure(), None);
        let d1 = b.on_failure().expect("ouvre au 3e échec");
        assert_eq!(d1.as_secs(), 30);
        // ré-ouvre : backoff double, plafonné à 300 s
        b.on_failure();
        b.on_failure();
        let d2 = b.on_failure().unwrap();
        assert_eq!(d2.as_secs(), 60);
        b.on_success(); // succès → tout est réarmé
        b.on_failure();
        b.on_failure();
        assert_eq!(b.on_failure().unwrap().as_secs(), 30);
    }
}
