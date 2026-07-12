use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
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

use crate::api::{ApiClient, ApiError, ApiItem};
use crate::pid::canonical;
use crate::store::{Resolution, Store};
use crate::telemetry::{Snapshot, Telemetry};
use std::collections::VecDeque;
use std::sync::atomic::AtomicBool;
use tokio::sync::{mpsc, watch};

#[derive(Debug)]
pub enum EngineEvent {
    Telemetry(Snapshot),
    /// reason ∈ {"auth_api", "auth_proxy", "server_down"}
    Suspended {
        reason: String,
        message: String,
        retry_in_s: Option<u64>,
    },
    Resumed,
    Finished {
        done: u64,
        failed: u64,
        stopped: bool,
    },
}

pub struct EngineParams {
    pub batch_size: usize,
    pub concurrency: u32,
}

pub struct RunHandle {
    /// Pause demandée par l'utilisateur (bouton Pause) — indépendante des
    /// suspensions système, pour qu'une reprise automatique (timer du
    /// breaker) ne puisse pas annuler une pause utilisateur.
    user_paused: Arc<AtomicBool>,
    /// Pause système (suspension auth ou breaker ouvert).
    sys_paused: Arc<AtomicBool>,
    suspended: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    key_tx: watch::Sender<String>,
    pub telemetry: Arc<Telemetry>,
}

impl RunHandle {
    pub fn set_paused(&self, p: bool) {
        self.user_paused.store(p, Ordering::Relaxed);
    }
    pub fn request_stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }
    /// Nouvelle clé API : lève aussi la suspension système — ressaisir la
    /// clé relance le run (les workers adoptent la clé via le watch avant
    /// de reprendre la file).
    pub fn update_key(&self, key: &str) {
        let _ = self.key_tx.send(key.to_string());
        self.sys_paused.store(false, Ordering::Relaxed);
        self.suspended.store(false, Ordering::Relaxed);
    }
    pub fn is_paused(&self) -> bool {
        self.user_paused.load(Ordering::Relaxed)
    }
}

fn now_epoch() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

/// Convertit un item API en résolution à persister. `sent` = PID envoyé
/// (repli si l'API ne renvoie pas participant_id).
fn to_resolution(item: &ApiItem, sent: &str, at: i64) -> Resolution {
    let participant = item
        .participant_id
        .clone()
        .or_else(|| item.participant.clone().map(|p| canonical(&p)))
        .unwrap_or_else(|| canonical(sent));
    match &item.error {
        Some(e) => Resolution {
            participant,
            exists_in_peppol: None,
            pa_code: None,
            pa_name: None,
            pa_country: None,
            extended_ctc_fr: None,
            api_status: format!("error:{e}"),
            resolved_at: at,
        },
        None => Resolution {
            participant,
            exists_in_peppol: item.exists,
            pa_code: item.pa.as_ref().and_then(|p| p.code.clone()),
            pa_name: item.pa.as_ref().and_then(|p| p.name.clone()),
            pa_country: item.pa.as_ref().and_then(|p| p.country.clone()),
            extended_ctc_fr: item.supports_extended_ctc_fr,
            api_status: "ok".into(),
            resolved_at: at,
        },
    }
}

/// Résolutions d'échec définitif (ApiError::Client) pour tout un paquet :
/// non retentable, chaque PID envoyé est écrit en base en échec.
fn client_error_resolutions(chunk: &[String], code: u16, at: i64) -> Vec<Resolution> {
    chunk
        .iter()
        .map(|pid| Resolution {
            participant: canonical(pid),
            exists_in_peppol: None,
            pa_code: None,
            pa_name: None,
            pa_country: None,
            extended_ctc_fr: None,
            api_status: format!("error:HTTP {code}"),
            resolved_at: at,
        })
        .collect()
}

pub struct Engine;

impl Engine {
    pub fn start(
        client: ApiClient,
        params: EngineParams,
        todo: Vec<String>,
        store: Arc<Mutex<Store>>,
        tx: mpsc::Sender<EngineEvent>,
    ) -> RunHandle {
        let total = todo.len() as u64;
        let telemetry = Arc::new(Telemetry::new(total));
        let user_paused = Arc::new(AtomicBool::new(false));
        let sys_paused = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let aimd = Arc::new(Aimd::new(params.concurrency));
        let breaker = Arc::new(Mutex::new(Breaker::new(5)));
        let suspended = Arc::new(AtomicBool::new(false));
        let (key_tx, key_rx) = watch::channel(String::new());
        // La clé initiale vit dans le client ; le canal ne sert qu'aux MAJ.

        let queue: Arc<Mutex<VecDeque<Vec<String>>>> = Arc::new(Mutex::new(
            todo.chunks(params.batch_size.max(1))
                .map(|c| c.to_vec())
                .collect(),
        ));
        let in_flight = Arc::new(AtomicU32::new(0));

        let mut workers = Vec::new();
        for idx in 0..params.concurrency {
            let (
                client,
                queue,
                store,
                telemetry,
                user_paused,
                sys_paused,
                stop,
                aimd,
                breaker,
                suspended,
                tx,
                mut key_rx,
            ) = (
                client.clone(),
                queue.clone(),
                store.clone(),
                telemetry.clone(),
                user_paused.clone(),
                sys_paused.clone(),
                stop.clone(),
                aimd.clone(),
                breaker.clone(),
                suspended.clone(),
                tx.clone(),
                key_rx.clone(),
            );
            let in_flight = in_flight.clone();
            workers.push(tokio::spawn(async move {
                let mut client = client;
                loop {
                    if stop.load(Ordering::Relaxed) {
                        break;
                    }
                    // Sortie de boucle : file vide ET non-suspendu ET aucun
                    // paquet in-flight. Le check est en tête de boucle pour
                    // qu'un worker bridé par l'AIMD (idx >= allowed) puisse
                    // aussi se terminer — sinon le superviseur ne verrait
                    // jamais la fin du run après un 429. La pause UTILISATEUR
                    // n'empêche pas la sortie : si tout est traité, le run est
                    // fini, pause ou pas.
                    // Ordre des lectures : in_flight (SeqCst) AVANT la file —
                    // un push_front d'erreur précède toujours le décrément
                    // d'in_flight, donc in_flight == 0 garantit qu'aucun
                    // paquet ne reviendra en tête de file.
                    // NB : il reste une fenêtre bénigne entre pop_front et
                    // fetch_add — un worker idle peut voir « file vide et
                    // rien in-flight » et sortir alors qu'un paquet vient
                    // d'être pris ; le worker preneur, lui, reste vivant et
                    // traitera (et retraitera au besoin) son paquet. Aucune
                    // perte : au pire un peu moins de parallélisme en toute
                    // fin de run.
                    if !sys_paused.load(Ordering::Relaxed)
                        && !suspended.load(Ordering::Relaxed)
                        && in_flight.load(Ordering::SeqCst) == 0
                        && queue.lock().unwrap().is_empty()
                    {
                        break;
                    }
                    if user_paused.load(Ordering::Relaxed)
                        || sys_paused.load(Ordering::Relaxed)
                        || idx >= aimd.allowed()
                    {
                        tokio::time::sleep(Duration::from_millis(150)).await;
                        continue;
                    }
                    // Nouvelle clé disponible ? (reprise après 401)
                    if key_rx.has_changed().unwrap_or(false) {
                        let k = key_rx.borrow_and_update().clone();
                        if !k.is_empty() {
                            client = client.with_key(&k);
                            suspended.store(false, Ordering::Relaxed);
                        }
                    }
                    let chunk = { queue.lock().unwrap().pop_front() };
                    let Some(chunk) = chunk else {
                        // File vide mais run pas fini (paquets in-flight qui
                        // peuvent revenir en tête de file) : on attend.
                        tokio::time::sleep(Duration::from_millis(150)).await;
                        continue;
                    };
                    in_flight.fetch_add(1, Ordering::SeqCst);
                    match client.resolve_batch(&chunk).await {
                        Ok((items, stats)) => {
                            breaker.lock().unwrap().on_success();
                            aimd.on_success();
                            let at = now_epoch();
                            let (mut ex, mut ctc, mut failed) = (0u32, 0u32, 0u32);
                            let mut resolutions = Vec::with_capacity(items.len());
                            for (i, item) in items.iter().enumerate() {
                                let sent = chunk.get(i).map(String::as_str).unwrap_or("");
                                let r = to_resolution(item, sent, at);
                                if r.api_status == "ok" {
                                    if r.exists_in_peppol == Some(true) {
                                        ex += 1;
                                    }
                                    if r.extended_ctc_fr == Some(true) {
                                        ctc += 1;
                                    }
                                } else {
                                    failed += 1;
                                }
                                resolutions.push(r);
                            }
                            {
                                let st = store.lock().unwrap();
                                let _ = st.upsert_batch(&resolutions);
                            }
                            telemetry.record_call(
                                stats.http_status,
                                stats.latency_ms,
                                items.len() as u32,
                                ex,
                                ctc,
                                failed,
                            );
                        }
                        Err(ApiError::Client(code)) => {
                            // Non retentable : pas de re-queue, pas
                            // d'alimentation du breaker ni de l'AIMD. On
                            // écrit l'échec en base pour tout le paquet et on
                            // compte la progression (le run continue).
                            let at = now_epoch();
                            let resolutions = client_error_resolutions(&chunk, code, at);
                            let addr_failed = resolutions.len() as u32;
                            {
                                let st = store.lock().unwrap();
                                let _ = st.upsert_batch(&resolutions);
                            }
                            telemetry.record_error(code, addr_failed);
                        }
                        Err(e) => {
                            telemetry.record_error(e.http_status(), 0);
                            // Le paquet repart en tête de file : rien n'est perdu.
                            queue.lock().unwrap().push_front(chunk);
                            match e {
                                ApiError::RateLimited { retry_after_s } => {
                                    aimd.on_rate_limited();
                                    tokio::time::sleep(Duration::from_secs_f64(
                                        retry_after_s.clamp(0.0, 60.0),
                                    ))
                                    .await;
                                }
                                ApiError::Auth(_) | ApiError::ProxyAuth => {
                                    // Si une nouvelle clé est arrivée pendant
                                    // que cette requête (partie avec l'ancienne
                                    // clé) était en vol, ce 401 est périmé :
                                    // on adopte la clé et on retente, sans
                                    // re-suspendre un run déjà repris.
                                    let stale = key_rx.has_changed().unwrap_or(false) && {
                                        let k = key_rx.borrow_and_update().clone();
                                        if k.is_empty() {
                                            false
                                        } else {
                                            client = client.with_key(&k);
                                            suspended.store(false, Ordering::Relaxed);
                                            true
                                        }
                                    };
                                    if !stale {
                                        // Suspension immédiate de tous les
                                        // workers ; un seul événement émis.
                                        sys_paused.store(true, Ordering::Relaxed);
                                        if !suspended.swap(true, Ordering::Relaxed) {
                                            let reason = if matches!(e, ApiError::ProxyAuth) {
                                                "auth_proxy"
                                            } else {
                                                "auth_api"
                                            };
                                            let _ = tx
                                                .send(EngineEvent::Suspended {
                                                    reason: reason.into(),
                                                    message: e.to_string(),
                                                    retry_in_s: None,
                                                })
                                                .await;
                                        }
                                    }
                                }
                                ApiError::Server(_) | ApiError::Network(_) => {
                                    let opened = breaker.lock().unwrap().on_failure();
                                    if let Some(d) = opened {
                                        // Une rafale de N échecs en vol peut
                                        // ouvrir le breaker plusieurs fois :
                                        // seul le premier gagnant émet
                                        // l'événement et arme LE timer de
                                        // reprise (sinon des timers 30/60 s
                                        // superposés se réveilleraient en
                                        // cascade sur un serveur down).
                                        if !suspended.swap(true, Ordering::Relaxed) {
                                            sys_paused.store(true, Ordering::Relaxed);
                                            // Réduit la rafale encore en vol
                                            // à la reprise.
                                            aimd.on_rate_limited();
                                            let _ = tx
                                                .send(EngineEvent::Suspended {
                                                    reason: "server_down".into(),
                                                    message: e.to_string(),
                                                    retry_in_s: Some(d.as_secs()),
                                                })
                                                .await;
                                            // Re-test automatique après le
                                            // backoff.
                                            let sys_paused2 = sys_paused.clone();
                                            let suspended2 = suspended.clone();
                                            let stop2 = stop.clone();
                                            let tx2 = tx.clone();
                                            tokio::spawn(async move {
                                                tokio::time::sleep(d).await;
                                                if stop2.load(Ordering::Relaxed) {
                                                    return; // run stoppé entre-temps
                                                }
                                                sys_paused2.store(false, Ordering::Relaxed);
                                                suspended2.store(false, Ordering::Relaxed);
                                                let _ = tx2.send(EngineEvent::Resumed).await;
                                            });
                                        }
                                    } else {
                                        tokio::time::sleep(Duration::from_secs(1)).await;
                                    }
                                }
                                ApiError::Client(_) => unreachable!("traité ci-dessus"),
                            }
                        }
                    }
                    in_flight.fetch_sub(1, Ordering::SeqCst);
                }
            }));
        }

        // Superviseur : télémétrie 4×/s, détection de fin.
        {
            let (telemetry, tx, stop) = (telemetry.clone(), tx.clone(), stop.clone());
            tokio::spawn(async move {
                for w in workers {
                    let _ = w.await;
                }
                let s = telemetry.snapshot();
                let _ = tx
                    .send(EngineEvent::Finished {
                        done: s.done,
                        failed: s.failed,
                        stopped: stop.load(Ordering::Relaxed),
                    })
                    .await;
            });
        }
        {
            let (telemetry, tx, stop) = (telemetry.clone(), tx, stop.clone());
            tokio::spawn(async move {
                loop {
                    tokio::time::sleep(Duration::from_millis(250)).await;
                    if tx
                        .send(EngineEvent::Telemetry(telemetry.snapshot()))
                        .await
                        .is_err()
                    {
                        break; // le récepteur a disparu : run terminé
                    }
                    if stop.load(Ordering::Relaxed) {
                        break;
                    }
                }
            });
        }

        RunHandle {
            user_paused,
            sys_paused,
            suspended,
            stop,
            key_tx,
            telemetry,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct CalibrationReport {
    pub best_concurrency: u32,
    pub addr_per_s: f64,
    pub rate_limited: bool,
}

/// Salves à concurrence croissante (1, 2, 4, … ≤ max) : mesure le débit de
/// chaque palier, s'arrête au premier 429 ou quand le gain devient < 15 %.
pub async fn calibrate(
    client: &ApiClient,
    sample: &[String],
    batch_size: usize,
    max_concurrency: u32,
) -> CalibrationReport {
    let mut best = (1u32, 0.0f64);
    let mut rate_limited = false;
    let mut level = 1u32;
    while level <= max_concurrency {
        let t0 = std::time::Instant::now();
        let mut handles = Vec::new();
        for i in 0..level {
            let client = client.clone();
            let chunk: Vec<String> = sample
                .iter()
                .cycle()
                .skip((i as usize * batch_size) % sample.len().max(1))
                .take(batch_size)
                .cloned()
                .collect();
            handles.push(tokio::spawn(
                async move { client.resolve_batch(&chunk).await },
            ));
        }
        let mut ok = 0usize;
        for h in handles {
            match h.await {
                Ok(Ok((items, _))) => ok += items.len(),
                Ok(Err(ApiError::RateLimited { .. })) => rate_limited = true,
                _ => {}
            }
        }
        let throughput = ok as f64 / t0.elapsed().as_secs_f64().max(0.001);
        if throughput > best.1 * 1.15 {
            best = (level, throughput);
        } else {
            break; // le palier n'apporte plus assez : on garde le précédent
        }
        if rate_limited {
            break;
        }
        level *= 2;
    }
    CalibrationReport {
        best_concurrency: best.0,
        addr_per_s: best.1,
        rate_limited,
    }
}

#[cfg(test)]
mod tests_engine {
    use super::*;
    use crate::api::ApiClient;
    use crate::store::Store;
    use std::sync::{Arc, Mutex};
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

    /// Répond 200 en faisant écho : chaque participant reçu existe dans Peppol.
    struct EchoResolver;
    impl Respond for EchoResolver {
        fn respond(&self, req: &Request) -> ResponseTemplate {
            let body: serde_json::Value = serde_json::from_slice(&req.body).unwrap();
            let results: Vec<serde_json::Value> = body["participants"]
                .as_array()
                .unwrap()
                .iter()
                .map(|p| {
                    serde_json::json!({
                        "participant_id": p, "exists": true,
                        "pa": {"code": "PA1", "name": "PA UN", "country": "FR"},
                        "supports_extended_ctc_fr": true
                    })
                })
                .collect();
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"results": results}))
        }
    }

    fn pids(n: usize) -> Vec<String> {
        (0..n)
            .map(|i| format!("iso6523-actorid-upis::0009:{i}"))
            .collect()
    }

    async fn run_engine(
        server: &MockServer,
        key: &str,
        todo: Vec<String>,
    ) -> (
        RunHandle,
        tokio::sync::mpsc::Receiver<EngineEvent>,
        Arc<Mutex<Store>>,
    ) {
        let store = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
        let client = ApiClient::new(&server.uri(), key, None, None).unwrap();
        let (tx, rx) = tokio::sync::mpsc::channel(256);
        let handle = Engine::start(
            client,
            EngineParams {
                batch_size: 10,
                concurrency: 4,
            },
            todo,
            store.clone(),
            tx,
        );
        (handle, rx, store)
    }

    async fn wait_finished(rx: &mut tokio::sync::mpsc::Receiver<EngineEvent>) -> (u64, u64) {
        loop {
            match tokio::time::timeout(std::time::Duration::from_secs(10), rx.recv()).await {
                Ok(Some(EngineEvent::Finished { done, failed, .. })) => return (done, failed),
                Ok(Some(_)) => continue,
                other => panic!("Finished attendu, obtenu {other:?}"),
            }
        }
    }

    #[tokio::test]
    async fn chemin_nominal_tout_est_resolu_en_base() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/resolve/batch"))
            .respond_with(EchoResolver)
            .mount(&server)
            .await;
        let (handle, mut rx, store) = run_engine(&server, "K", pids(53)).await;
        let (done, failed) = wait_finished(&mut rx).await;
        assert_eq!((done, failed), (53, 0));
        let m = store.lock().unwrap().load_map(&pids(53)).unwrap();
        assert_eq!(m.len(), 53);
        assert!(m
            .values()
            .all(|r| r.api_status == "ok" && r.pa_code.as_deref() == Some("PA1")));
        let _ = handle;
    }

    #[tokio::test]
    async fn un_429_ralentit_puis_le_run_aboutit() {
        let server = MockServer::start().await;
        // Le premier appel prend un 429, les suivants passent.
        Mock::given(method("POST"))
            .and(path("/resolve/batch"))
            .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "0"))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/resolve/batch"))
            .respond_with(EchoResolver)
            .mount(&server)
            .await;
        let (handle, mut rx, _store) = run_engine(&server, "K", pids(30)).await;
        let (done, _) = wait_finished(&mut rx).await;
        assert_eq!(done, 30);
        assert_eq!(handle.telemetry.snapshot().http.get(&429), Some(&1));
    }

    #[tokio::test]
    async fn cle_invalide_suspend_puis_nouvelle_cle_reprend() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/resolve/batch"))
            .and(header("X-API-Key", "BONNE"))
            .respond_with(EchoResolver)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/resolve/batch"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;

        let (handle, mut rx, _store) = run_engine(&server, "MAUVAISE", pids(20)).await;
        // On doit recevoir Suspended{auth_api}
        loop {
            match tokio::time::timeout(std::time::Duration::from_secs(10), rx.recv()).await {
                Ok(Some(EngineEvent::Suspended { reason, .. })) => {
                    assert_eq!(reason, "auth_api");
                    break;
                }
                Ok(Some(_)) => continue,
                other => panic!("Suspended attendu, obtenu {other:?}"),
            }
        }
        handle.update_key("BONNE");
        handle.set_paused(false);
        let (done, _) = wait_finished(&mut rx).await;
        assert_eq!(done, 20);
    }

    #[tokio::test]
    async fn stop_arrete_sans_perdre_ce_qui_est_fait() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/resolve/batch"))
            .respond_with(EchoResolver)
            .mount(&server)
            .await;
        let (handle, mut rx, store) = run_engine(&server, "K", pids(50)).await;
        handle.request_stop();
        let (_done, _) = wait_finished(&mut rx).await;
        // Tout ce qui est marqué done est réellement en base.
        let snap = handle.telemetry.snapshot();
        let m = store.lock().unwrap().load_map(&pids(50)).unwrap();
        assert_eq!(m.len() as u64, snap.done);
    }

    /// Une rafale d'échecs 5xx en vol (10 requêtes simultanées, seuil du
    /// breaker à 5) ne doit ouvrir le breaker et suspendre le run qu'UNE
    /// seule fois — pas un événement + un timer de reprise par ouverture.
    /// Temps tokio en pause : le backoff de 30 s s'écoule virtuellement.
    #[tokio::test(start_paused = true)]
    async fn rafale_5xx_ouvre_le_breaker_une_seule_fois_puis_reprend() {
        let server = MockServer::start().await;
        // Les 10 premiers appels échouent en 503 (rafale > seuil), ensuite
        // tout passe : les 10 paquets re-queués aboutissent après reprise.
        Mock::given(method("POST"))
            .and(path("/resolve/batch"))
            .respond_with(ResponseTemplate::new(503))
            .up_to_n_times(10)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/resolve/batch"))
            .respond_with(EchoResolver)
            .mount(&server)
            .await;

        let store = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
        let client = ApiClient::new(&server.uri(), "K", None, None).unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::channel(256);
        let _handle = Engine::start(
            client,
            EngineParams {
                batch_size: 5,
                concurrency: 10,
            },
            pids(50),
            store,
            tx,
        );
        let mut suspensions = 0u32;
        let done = loop {
            match tokio::time::timeout(std::time::Duration::from_secs(10), rx.recv()).await {
                Ok(Some(EngineEvent::Suspended { reason, .. })) => {
                    assert_eq!(reason, "server_down");
                    suspensions += 1;
                }
                Ok(Some(EngineEvent::Finished { done, .. })) => break done,
                Ok(Some(_)) => continue,
                other => panic!("Finished attendu, obtenu {other:?}"),
            }
        };
        assert_eq!(
            suspensions, 1,
            "une rafale d'échecs ne doit émettre qu'un seul Suspended{{server_down}}"
        );
        assert_eq!(done, 50);
    }

    /// Stopper un run suspendu (401 permanent) doit émettre Finished
    /// {stopped: true} — les workers ne restent pas bloqués en pause.
    #[tokio::test]
    async fn stop_pendant_suspension_emet_finished() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/resolve/batch"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;
        let (handle, mut rx, _store) = run_engine(&server, "MAUVAISE", pids(20)).await;
        loop {
            match tokio::time::timeout(std::time::Duration::from_secs(10), rx.recv()).await {
                Ok(Some(EngineEvent::Suspended { .. })) => break,
                Ok(Some(_)) => continue,
                other => panic!("Suspended attendu, obtenu {other:?}"),
            }
        }
        handle.request_stop();
        loop {
            match tokio::time::timeout(std::time::Duration::from_secs(10), rx.recv()).await {
                Ok(Some(EngineEvent::Finished { stopped, .. })) => {
                    assert!(stopped, "Finished{{stopped: true}} attendu après stop");
                    break;
                }
                Ok(Some(_)) => continue,
                other => panic!("Finished attendu, obtenu {other:?}"),
            }
        }
    }
}

#[cfg(test)]
mod tests_calibrate {
    use super::*;
    use crate::api::ApiClient;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn calibrate_renvoie_un_debit_et_une_concurrence() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/resolve/batch"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_delay(std::time::Duration::from_millis(50))
                    .set_body_json(serde_json::json!({"results": [
                        {"participant_id": "a::1", "exists": true}
                    ]})),
            )
            .mount(&server)
            .await;
        let c = ApiClient::new(&server.uri(), "K", None, None).unwrap();
        let sample: Vec<String> = (0..8).map(|i| format!("0009:{i}")).collect();
        let rep = calibrate(&c, &sample, 1, 8).await;
        assert!(rep.best_concurrency >= 1);
        assert!(rep.addr_per_s > 0.0);
        assert!(!rep.rate_limited);
    }
}
