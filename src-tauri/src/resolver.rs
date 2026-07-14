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
/// avec un backoff 30 s doublé à chaque ouverture. Le décalage est plafonné
/// à 3 (30, 60, 120, 240 s), donc le plafond effectif est 240 s : le
/// `.min(300)` du calcul n'est qu'une ceinture qui ne joue jamais.
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
use crate::telemetry::{LineWeights, Snapshot, Telemetry};
use std::collections::{BTreeMap, HashMap, VecDeque};
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
        /// Durée active du run en secondes (pauses et suspensions exclues).
        active_s: f64,
    },
}

pub struct EngineParams {
    pub batch_size: usize,
    pub concurrency: u32,
}

/// Mise à jour poussée aux workers via un `watch`, pour reprendre un run
/// suspendu sans le relancer. `Key` couvre le 401 (seule la clé API
/// change) ; `Client` couvre le 407 (les creds proxy vivent dans le
/// builder reqwest, il faut donc un client entier neuf).
#[derive(Clone)]
pub enum ClientUpdate {
    Key(String),
    Client(ApiClient),
}

pub struct RunHandle {
    /// Pause demandée par l'utilisateur (bouton Pause) — indépendante des
    /// suspensions système, pour qu'une reprise automatique (timer du
    /// breaker) ne puisse pas annuler une pause utilisateur.
    user_paused: Arc<AtomicBool>,
    /// Pause système (suspension auth ou breaker ouvert).
    sys_paused: Arc<AtomicBool>,
    suspended: Arc<AtomicBool>,
    /// Génération des timers de reprise : incrémentée à chaque armement de
    /// timer et par resume_system. Un timer capture sa génération à
    /// l'armement et reste muet au réveil si elle n'est plus la dernière
    /// (reprise manuelle ou nouvelle suspension entre-temps).
    resume_gen: Arc<AtomicU32>,
    stop: Arc<AtomicBool>,
    update_tx: watch::Sender<Option<ClientUpdate>>,
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
    /// NB : la prod (commands::update_api_key) passe par update_client pour
    /// porter l'état complet ; update_key reste l'API légère (testée par
    /// cle_invalide_suspend_puis_nouvelle_cle_reprend).
    pub fn update_key(&self, key: &str) {
        let _ = self
            .update_tx
            .send(Some(ClientUpdate::Key(key.to_string())));
        self.sys_paused.store(false, Ordering::Relaxed);
        self.suspended.store(false, Ordering::Relaxed);
    }
    /// Nouveau client API complet (reconstruit avec les creds proxy après
    /// un 407) : même mécanique que `update_key`, mais remplace tout le
    /// client — les creds proxy vivent dans le builder reqwest, pas dans un
    /// champ modifiable après coup.
    pub fn update_client(&self, client: ApiClient) {
        let _ = self.update_tx.send(Some(ClientUpdate::Client(client)));
        self.sys_paused.store(false, Ordering::Relaxed);
        self.suspended.store(false, Ordering::Relaxed);
    }
    pub fn is_paused(&self) -> bool {
        self.user_paused.load(Ordering::Relaxed)
    }
    /// Reprise anticipée d'une suspension système (bouton « Réessayer
    /// maintenant » sur la bannière server_down) : lève sys_paused et
    /// suspended. Diffère du timer de backoff sur deux points : ne vérifie
    /// pas `stop` (inutile — les workers re-testent le flag en tête de
    /// boucle) et n'émet pas Resumed (le front masque la bannière lui-même).
    /// Incrémente la génération pour invalider le timer encore armé, sinon
    /// son réveil lèverait prématurément une suspension ULTÉRIEURE et
    /// émettrait un Resumed parasite qui masquerait sa bannière.
    /// Ne touche pas à la pause utilisateur.
    pub fn resume_system(&self) {
        self.resume_gen.fetch_add(1, Ordering::Relaxed);
        self.sys_paused.store(false, Ordering::Relaxed);
        self.suspended.store(false, Ordering::Relaxed);
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
    /// `line_counts` : lignes du fichier d'entrée par PID canonique — un PID
    /// absent de la carte pèse 1 ligne (repli).
    pub fn start(
        client: ApiClient,
        params: EngineParams,
        todo: Vec<String>,
        line_counts: HashMap<String, u64>,
        store: Arc<Mutex<Store>>,
        tx: mpsc::Sender<EngineEvent>,
    ) -> RunHandle {
        let total = todo.len() as u64;
        let line_counts = Arc::new(line_counts);
        let telemetry = Arc::new(Telemetry::new(total));
        let user_paused = Arc::new(AtomicBool::new(false));
        let sys_paused = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let aimd = Arc::new(Aimd::new(params.concurrency));
        let breaker = Arc::new(Mutex::new(Breaker::new(5)));
        let suspended = Arc::new(AtomicBool::new(false));
        let resume_gen = Arc::new(AtomicU32::new(0));
        let (update_tx, update_rx) = watch::channel(None::<ClientUpdate>);
        // La clé/le client initial vit dans `client` ; le canal ne sert
        // qu'aux mises à jour en cours de run (401 → nouvelle clé, 407 →
        // nouveau client).

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
                resume_gen,
                tx,
                mut update_rx,
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
                resume_gen.clone(),
                tx.clone(),
                update_rx.clone(),
            );
            let in_flight = in_flight.clone();
            let line_counts = line_counts.clone();
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
                    // Mise à jour disponible ? (reprise après 401 ou 407)
                    if update_rx.has_changed().unwrap_or(false) {
                        let update = update_rx.borrow_and_update().clone();
                        match update {
                            Some(ClientUpdate::Key(k)) => {
                                client = client.with_key(&k);
                                suspended.store(false, Ordering::Relaxed);
                            }
                            Some(ClientUpdate::Client(c)) => {
                                client = c;
                                suspended.store(false, Ordering::Relaxed);
                            }
                            None => {}
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
                            let mut pa_counts: BTreeMap<String, u32> = BTreeMap::new();
                            let mut error_counts: BTreeMap<String, u32> = BTreeMap::new();
                            let mut lines = LineWeights::default();
                            let mut resolutions = Vec::with_capacity(items.len());
                            for (i, item) in items.iter().enumerate() {
                                let sent = chunk.get(i).map(String::as_str).unwrap_or("");
                                // Poids en lignes de fichier de cet adressage.
                                let w = line_counts.get(sent).copied().unwrap_or(1);
                                lines.done += w;
                                let r = to_resolution(item, sent, at);
                                if r.api_status == "ok" {
                                    if r.exists_in_peppol == Some(true) {
                                        ex += 1;
                                        lines.exists += w;
                                    }
                                    if r.extended_ctc_fr == Some(true) {
                                        ctc += 1;
                                        lines.ctc += w;
                                    }
                                    // Repli sur le code si l'API n'a pas de nom.
                                    if let Some(pa) =
                                        r.pa_name.as_ref().or(r.pa_code.as_ref())
                                    {
                                        *pa_counts.entry(pa.clone()).or_insert(0) += 1;
                                    }
                                } else {
                                    failed += 1;
                                    let motif = r
                                        .api_status
                                        .strip_prefix("error:")
                                        .unwrap_or(&r.api_status);
                                    *error_counts.entry(motif.to_string()).or_insert(0) += 1;
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
                                &pa_counts,
                                lines,
                                &error_counts,
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
                            let lines_failed: u64 = chunk
                                .iter()
                                .map(|p| line_counts.get(p).copied().unwrap_or(1))
                                .sum();
                            {
                                let st = store.lock().unwrap();
                                let _ = st.upsert_batch(&resolutions);
                            }
                            telemetry.record_error(code, addr_failed, lines_failed);
                        }
                        Err(e) => {
                            telemetry.record_error(e.http_status(), 0, 0);
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
                                    // Si une mise à jour (clé ou client) est
                                    // arrivée pendant que cette requête
                                    // (partie avec l'ancien client) était en
                                    // vol, ce 401/407 est périmé : on adopte
                                    // la mise à jour et on retente, sans
                                    // re-suspendre un run déjà repris.
                                    let stale = update_rx.has_changed().unwrap_or(false) && {
                                        let update = update_rx.borrow_and_update().clone();
                                        match update {
                                            Some(ClientUpdate::Key(k)) => {
                                                client = client.with_key(&k);
                                                suspended.store(false, Ordering::Relaxed);
                                                true
                                            }
                                            Some(ClientUpdate::Client(c)) => {
                                                client = c;
                                                suspended.store(false, Ordering::Relaxed);
                                                true
                                            }
                                            None => false,
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
                                            // backoff. Le timer capture sa
                                            // génération : s'il n'est plus le
                                            // dernier au réveil (resume_system
                                            // manuel, ou nouveau timer armé
                                            // par une suspension ultérieure),
                                            // il reste muet — sinon il
                                            // écourterait le backoff suivant
                                            // et son Resumed masquerait la
                                            // bannière en cours.
                                            let gen =
                                                resume_gen.fetch_add(1, Ordering::Relaxed) + 1;
                                            let sys_paused2 = sys_paused.clone();
                                            let suspended2 = suspended.clone();
                                            let resume_gen2 = resume_gen.clone();
                                            let stop2 = stop.clone();
                                            let tx2 = tx.clone();
                                            tokio::spawn(async move {
                                                tokio::time::sleep(d).await;
                                                if stop2.load(Ordering::Relaxed) {
                                                    return; // run stoppé entre-temps
                                                }
                                                if resume_gen2.load(Ordering::Relaxed) != gen {
                                                    return; // timer périmé
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
            let (user_paused, suspended) = (user_paused.clone(), suspended.clone());
            tokio::spawn(async move {
                for w in workers {
                    let _ = w.await;
                }
                // Tick final : sans lui, un run plus court que la période du
                // superviseur (250 ms) aurait une durée active nulle.
                telemetry.tick_active(
                    !user_paused.load(Ordering::Relaxed) && !suspended.load(Ordering::Relaxed),
                );
                let s = telemetry.snapshot();
                let _ = tx
                    .send(EngineEvent::Finished {
                        done: s.done,
                        failed: s.failed,
                        stopped: stop.load(Ordering::Relaxed),
                        active_s: s.active_s,
                    })
                    .await;
            });
        }
        {
            let (telemetry, tx, stop) = (telemetry.clone(), tx, stop.clone());
            let (user_paused, suspended) = (user_paused.clone(), suspended.clone());
            let aimd = aimd.clone();
            let concurrency_max = params.concurrency.max(1);
            tokio::spawn(async move {
                loop {
                    tokio::time::sleep(Duration::from_millis(250)).await;
                    // Échantillonnage du temps actif : la boucle tourne aussi
                    // pendant les pauses/suspensions, l'intervalle n'est alors
                    // pas compté (durée affichée en fin de run).
                    telemetry.tick_active(
                        !user_paused.load(Ordering::Relaxed) && !suspended.load(Ordering::Relaxed),
                    );
                    // La concurrence vit dans le moteur, pas dans la
                    // télémétrie : on complète le snapshot à l'émission.
                    let mut s = telemetry.snapshot();
                    s.concurrency_allowed = aimd.allowed();
                    s.concurrency_max = concurrency_max;
                    if tx.send(EngineEvent::Telemetry(s)).await.is_err() {
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
            resume_gen,
            stop,
            update_tx,
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

    /// Corps d'écho commun à EchoResolver et sa variante lente : chaque
    /// participant reçu existe dans Peppol.
    fn echo_body(req: &Request) -> serde_json::Value {
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
        serde_json::json!({"results": results})
    }

    /// Répond 200 en faisant écho : chaque participant reçu existe dans Peppol.
    struct EchoResolver;
    impl Respond for EchoResolver {
        fn respond(&self, req: &Request) -> ResponseTemplate {
            ResponseTemplate::new(200).set_body_json(echo_body(req))
        }
    }

    /// Comme EchoResolver, avec un délai artificiel avant la réponse — pour
    /// observer un run en cours (ex. pause utilisateur) sans qu'il se
    /// termine avant l'assertion.
    struct SlowEchoResolver(Duration);
    impl Respond for SlowEchoResolver {
        fn respond(&self, req: &Request) -> ResponseTemplate {
            ResponseTemplate::new(200)
                .set_body_json(echo_body(req))
                .set_delay(self.0)
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
            HashMap::new(),
            store.clone(),
            tx,
        );
        (handle, rx, store)
    }

    async fn wait_finished(rx: &mut tokio::sync::mpsc::Receiver<EngineEvent>) -> (u64, u64) {
        loop {
            match tokio::time::timeout(std::time::Duration::from_secs(10), rx.recv()).await {
                Ok(Some(EngineEvent::Finished { done, failed, active_s, .. })) => {
                    // Tick final : même un run éclair (< 250 ms de superviseur)
                    // doit porter une durée active non nulle.
                    assert!(active_s > 0.0, "durée active nulle : {active_s}");
                    return (done, failed);
                }
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
        // Câblage PA → télémétrie : les 53 adressages portent la même PA.
        let snap = handle.telemetry.snapshot();
        assert_eq!(snap.pa.len(), 1);
        assert_eq!((snap.pa[0].name.as_str(), snap.pa[0].count), ("PA UN", 53));
        // Sans carte de multiplicités (HashMap vide), 1 ligne par adressage.
        assert_eq!(snap.done_lines, 53);
    }

    /// Répond 200 mais un participant sur deux est en erreur item
    /// « timeout SMP » — le cas réel d'une API saturée.
    struct HalfErrorResolver;
    impl Respond for HalfErrorResolver {
        fn respond(&self, req: &Request) -> ResponseTemplate {
            let body: serde_json::Value = serde_json::from_slice(&req.body).unwrap();
            let results: Vec<serde_json::Value> = body["participants"]
                .as_array()
                .unwrap()
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    if i % 2 == 0 {
                        serde_json::json!({"participant_id": p, "error": "timeout SMP"})
                    } else {
                        serde_json::json!({"participant_id": p, "exists": true,
                            "supports_extended_ctc_fr": false})
                    }
                })
                .collect();
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"results": results}))
        }
    }

    #[tokio::test]
    async fn erreurs_item_remontees_par_motif_dans_la_telemetrie() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/resolve/batch"))
            .respond_with(HalfErrorResolver)
            .mount(&server)
            .await;
        let (handle, mut rx, _store) = run_engine(&server, "K", pids(10)).await;
        let (done, failed) = wait_finished(&mut rx).await;
        assert_eq!((done, failed), (10, 5));
        let errors = handle.telemetry.snapshot().errors;
        assert_eq!(errors.len(), 1);
        assert_eq!((errors[0].name.as_str(), errors[0].count), ("timeout SMP", 5));
    }

    #[tokio::test]
    async fn telemetrie_expose_la_concurrence_aimd() {
        let server = MockServer::start().await;
        // Réponses lentes : le premier tick de télémétrie (250 ms) part
        // pendant que le run tourne encore.
        Mock::given(method("POST"))
            .and(path("/resolve/batch"))
            .respond_with(SlowEchoResolver(Duration::from_millis(300)))
            .mount(&server)
            .await;
        let (handle, mut rx, _store) = run_engine(&server, "K", pids(40)).await;
        let snap = loop {
            match tokio::time::timeout(std::time::Duration::from_secs(10), rx.recv()).await {
                Ok(Some(EngineEvent::Telemetry(s))) => break s,
                Ok(Some(_)) => continue,
                other => panic!("Telemetry attendu, obtenu {other:?}"),
            }
        };
        // run_engine démarre avec concurrency = 4 ; sans 429, l'AIMD reste
        // au plafond.
        assert_eq!(snap.concurrency_max, 4);
        assert_eq!(snap.concurrency_allowed, 4);
        wait_finished(&mut rx).await;
        let _ = handle;
    }

    #[tokio::test]
    async fn lignes_ponderees_par_multiplicite_du_fichier() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/resolve/batch"))
            .respond_with(EchoResolver)
            .mount(&server)
            .await;
        let store = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
        let client = ApiClient::new(&server.uri(), "K", None, None).unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::channel(256);
        let todo = pids(2);
        // Le premier PID couvre 3 lignes du fichier ; le second, absent de la
        // carte, pèse 1 ligne (repli).
        let line_counts = HashMap::from([(todo[0].clone(), 3u64)]);
        let handle = Engine::start(
            client,
            EngineParams {
                batch_size: 10,
                concurrency: 1,
            },
            todo,
            line_counts,
            store,
            tx,
        );
        wait_finished(&mut rx).await;
        let s = handle.telemetry.snapshot();
        // EchoResolver : tout existe et tout est CTC → 2 adressages, 4 lignes.
        assert_eq!((s.done, s.done_lines), (2, 4));
        assert_eq!((s.exists_lines, s.ctc_lines), (4, 4));
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

    /// Le 407 (proxy) exige un client entier neuf (les creds proxy vivent
    /// dans le builder reqwest) — on simule le proxy via le header X-API-Key
    /// pour isoler le mécanisme de swap de client, sans dépendre de reqwest.
    #[tokio::test]
    async fn proxy_407_suspend_puis_nouveau_client_reprend() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/resolve/batch"))
            .and(header("X-API-Key", "AVEC_PROXY_OK"))
            .respond_with(EchoResolver)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/resolve/batch"))
            .respond_with(ResponseTemplate::new(407))
            .mount(&server)
            .await;

        let (handle, mut rx, _store) = run_engine(&server, "SANS_CREDS", pids(20)).await;
        loop {
            match tokio::time::timeout(std::time::Duration::from_secs(10), rx.recv()).await {
                Ok(Some(EngineEvent::Suspended { reason, .. })) => {
                    assert_eq!(reason, "auth_proxy");
                    break;
                }
                Ok(Some(_)) => continue,
                other => panic!("Suspended attendu, obtenu {other:?}"),
            }
        }
        let client_ok = ApiClient::new(&server.uri(), "AVEC_PROXY_OK", None, None).unwrap();
        handle.update_client(client_ok);
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
            HashMap::new(),
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

    /// Reprise anticipée : resume_system (bouton « Réessayer maintenant »)
    /// relance le run sans attendre le backoff de 30 s, et le timer devenu
    /// périmé doit rester muet — pas de Resumed parasite qui masquerait la
    /// bannière d'une suspension ultérieure (compteur de génération).
    /// Concurrence 1 pour un scénario déterministe : au moment du Suspended
    /// (5e échec consécutif), plus aucune requête n'est en vol, donc aucun
    /// échec tardif ne peut ré-ouvrir le breaker après la reprise.
    #[tokio::test(start_paused = true)]
    async fn resume_system_reprend_avant_le_backoff() {
        let server = MockServer::start().await;
        // 5 × 503 (= seuil du breaker) puis tout passe.
        Mock::given(method("POST"))
            .and(path("/resolve/batch"))
            .respond_with(ResponseTemplate::new(503))
            .up_to_n_times(5)
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
        let handle = Engine::start(
            client,
            EngineParams {
                batch_size: 5,
                concurrency: 1,
            },
            pids(10),
            HashMap::new(),
            store,
            tx,
        );
        loop {
            match tokio::time::timeout(Duration::from_secs(60), rx.recv()).await {
                Ok(Some(EngineEvent::Suspended { reason, .. })) => {
                    assert_eq!(reason, "server_down");
                    break;
                }
                Ok(Some(_)) => continue,
                other => panic!("Suspended attendu, obtenu {other:?}"),
            }
        }
        handle.resume_system();
        let t0 = tokio::time::Instant::now();
        let done = loop {
            match tokio::time::timeout(Duration::from_secs(60), rx.recv()).await {
                Ok(Some(EngineEvent::Resumed)) => {
                    panic!(
                        "Resumed parasite : la reprise doit venir de resume_system, pas du timer"
                    )
                }
                Ok(Some(EngineEvent::Finished { done, .. })) => break done,
                Ok(Some(_)) => continue,
                other => panic!("Finished attendu, obtenu {other:?}"),
            }
        };
        assert_eq!(done, 10);
        assert!(
            t0.elapsed() < Duration::from_secs(30),
            "le run doit aboutir avant l'expiration du backoff (reprise anticipée)"
        );
        // Laisse le backoff (30 s virtuelles) s'écouler : le timer périmé ne
        // doit émettre aucun Resumed après coup.
        tokio::time::sleep(Duration::from_secs(40)).await;
        loop {
            match rx.try_recv() {
                Ok(EngineEvent::Resumed) => panic!("Resumed parasite du timer périmé"),
                Ok(_) => continue,
                Err(_) => break,
            }
        }
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

    /// Pause utilisateur (set_paused) : les paquets déjà en vol se
    /// terminent, mais aucun nouveau paquet n'est pris tant que la pause
    /// est active — la progression doit se stabiliser. La reprise
    /// (set_paused(false)) doit amener le run à son terme.
    #[tokio::test]
    async fn pause_utilisateur_suspend_puis_reprend() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/resolve/batch"))
            .respond_with(SlowEchoResolver(Duration::from_millis(50)))
            .mount(&server)
            .await;

        let store = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
        let client = ApiClient::new(&server.uri(), "K", None, None).unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::channel(256);
        let handle = Engine::start(
            client,
            EngineParams {
                batch_size: 1,
                concurrency: 2,
            },
            pids(20),
            HashMap::new(),
            store,
            tx,
        );

        // Laisse le run démarrer : chaque worker part sur un paquet.
        tokio::time::sleep(Duration::from_millis(80)).await;
        handle.set_paused(true);

        // Le paquet déjà en vol par worker (au plus 2) se termine malgré la
        // pause ; ensuite, plus aucun nouveau paquet n'est pris. Deux
        // snapshots espacés doivent donc afficher la même progression.
        tokio::time::sleep(Duration::from_millis(150)).await;
        let snap1 = handle.telemetry.snapshot();
        tokio::time::sleep(Duration::from_millis(200)).await;
        let snap2 = handle.telemetry.snapshot();
        assert_eq!(
            snap1.done, snap2.done,
            "aucune progression attendue pendant la pause utilisateur"
        );
        assert!(
            snap2.done < 20,
            "le run ne doit pas déjà être fini avant la reprise (pause inefficace ?)"
        );

        handle.set_paused(false);
        let (done, _) = wait_finished(&mut rx).await;
        assert_eq!(done, 20);
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
