//! Résolution directe SML+SMP — parité avec peppol_resolver.py + la vue
//! simple de peppol_api.py, sans passer par l'API.
//!
//! Pipeline par adressage :
//!   1. hash SHA-256 + base32 de lowercase(value) → hostname SML
//!   2. DNS NAPTR (système ou DoH) → URL du SMP
//!   3. GET ServiceGroup → doctypes (décodés depuis les hrefs)
//!   4. GET d'UN ServiceMetadata ciblé (doctype CTC si supporté, sinon le
//!      premier) → certificat AS4 → PA (CN = code, O = nom, C = pays)
//!
//! Sémantique d'existence (mêmes règles que l'API — jamais de faux négatif) :
//!   NXDOMAIN authentique → exists=false ; NoAnswer/erreur DNS → erreur item
//!   « SML lookup: … » re-tentable en mode reprise.
//!
//! Écart assumé vs peppol_resolver.py : le support CTC est lu dans les hrefs
//! du ServiceGroup (le doctype y est URL-encodé) au lieu de télécharger tous
//! les ServiceMetadata ; un seul ServiceMetadata est chargé pour identifier
//! la PA. Identique au cas nominal (une PA par adressage), et un adressage
//! multi-PA peut remonter une PA différente du choix de l'API.

use crate::api::{ApiError, ApiItem, CallStats, PaInfo, ProxyCreds};
use data_encoding::{BASE32, BASE64, BASE64URL_NOPAD};
use percent_encoding::{percent_decode_str, utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
#[cfg(test)]
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub const SML_PROD: &str = "participant.sml.prod.tech.peppol.org";

/// Cible de la sonde preflight : hôte Peppol public et stable, hors SML/SMP
/// (seul le chemin proxy→HTTPS est testé, pas la cible elle-même).
pub const PROXY_PROBE_URL: &str = "https://directory.peppol.eu/";

/// Doctype de la facture structurée principale (PASR §6.1.c). DOIT rester
/// identique à FR_CTC_PRIMARY_INVOICE de peppol_api.py.
pub const FR_CTC_PRIMARY_INVOICE: &str =
    "urn:oasis:names:specification:ubl:schema:xsd:Invoice-2::Invoice\
     ##urn:cen.eu:en16931:2017#conformant\
     #urn:peppol:france:billing:extended:1.0::2.1";

const DEFAULT_SCHEME: &str = "iso6523-actorid-upis";

/// Parité urllib quote(safe='') : seuls [A-Za-z0-9_.~-] restent en clair.
const PID_ENCODE: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'_')
    .remove(b'.')
    .remove(b'-')
    .remove(b'~');

/// Retries DNS : l'autoritaire SML rend des NoAnswer transitoires sous
/// rafale (constaté en prod le 2026-07-13) — on retente avant de conclure.
const DNS_MAX_RETRIES: u32 = 2;
const DNS_RETRY_BASE_MS: u64 = 400;

/// Rafale DNS par défaut (config `api.dns_concurrency`, réglable dans
/// l'IHM) : borne les lookups simultanés indépendamment de la concurrence
/// des workers (parité _DNS_SEM de peppol_resolver.py). 32 × ~25 ms
/// ≈ 1 250 req/s de plafond : sous le rate-limit de Google Public DNS
/// (~1 500 QPS/IP), et autant de sockets UDP en vol au maximum — une par
/// requête hickory (EMFILE constaté le 2026-07-14 à concurrence 128 sur
/// macOS).
pub const DNS_CONCURRENCY_DEFAULT: u32 = 32;

/// Hostname NAPTR selon la spec SML OpenPeppol (post-nov 2025) :
/// base32(sha256(lowercase(value))) sans padding, en minuscules.
pub fn sml_hostname(scheme: &str, value: &str, zone: &str) -> String {
    let digest = Sha256::digest(value.to_lowercase().as_bytes());
    let b32 = BASE32.encode(&digest);
    let b32 = b32.trim_end_matches('=').to_lowercase();
    format!("{b32}.{scheme}.{zone}")
}

/// Issue d'un lookup NAPTR sur le SML.
#[derive(Debug, Clone)]
pub enum SmlLookup {
    /// Enregistré : URL du SMP (premier enregistrement Meta:SMP).
    Found(String),
    /// NXDOMAIN authentique : non enregistré, verdict définitif.
    NotRegistered,
    /// Échec de consultation (NoAnswer, timeout…) : erreur item, jamais
    /// un verdict d'absence. Le libellé suit les statuts de l'API
    /// (« NoAnswer », « DNS_ERROR:… »).
    Failed(String),
}

/// Extrait l'URL SMP d'un lot d'enregistrements NAPTR (service, regexp),
/// comme resolve_smp_url : service `Meta:SMP*` et regexp `!.*!<url>!`.
fn smp_url_from_naptr(records: &[(String, String)]) -> Option<String> {
    for (service, regexp) in records {
        if !service.starts_with("Meta:SMP") {
            continue;
        }
        let rest = regexp.strip_prefix("!.*!")?;
        if let Some(url) = rest.strip_suffix('!') {
            if !url.is_empty() {
                return Some(url.to_string());
            }
        }
    }
    None
}

/// Résolveur NAPTR : DNS système, DoH (RFC 8484, via reqwest donc compatible
/// proxy), ou table factice pour les tests.
pub enum Dns {
    System(hickory_resolver::TokioAsyncResolver),
    Doh {
        http: reqwest::Client,
        url: String,
        fallback: Option<String>,
    },
    #[cfg(test)]
    Fake(HashMap<String, SmlLookup>),
}

impl Dns {
    pub fn system() -> Result<Self, String> {
        hickory_resolver::TokioAsyncResolver::tokio_from_system_conf()
            .map(Dns::System)
            .map_err(|e| format!("résolveur DNS système : {e}"))
    }

    /// DNS classique (UDP/53, repli TCP) sur un ou plusieurs serveurs choisis
    /// — évite le résolveur du FAI (rate-limiting sous rafale, constaté
    /// 2026-07-14 : 8.8.8.8 tient 1 500 req/s là où le résolveur Free refuse
    /// ~30 %). Le premier serveur est le principal, les suivants des secours
    /// (UserProvidedOrder : hickory garde l'ordre fourni et ne bascule que
    /// sur échec — pas de lissage de charge, c'est voulu : la limite par IP
    /// des résolveurs publics protège la zone SML). Aucun search domain : la
    /// config est construite sans, et les requêtes partent en FQDN absolu.
    pub fn udp(ips: &[std::net::IpAddr]) -> Self {
        use hickory_resolver::config::{
            NameServerConfigGroup, ResolverConfig, ResolverOpts, ServerOrderingStrategy,
        };
        let servers = NameServerConfigGroup::from_ips_clear(ips, 53, true);
        let config = ResolverConfig::from_parts(None, Vec::new(), servers);
        let mut opts = ResolverOpts::default();
        opts.server_ordering_strategy = ServerOrderingStrategy::UserProvidedOrder;
        Dns::System(hickory_resolver::TokioAsyncResolver::tokio(config, opts))
    }

    pub fn doh(url: &str, fallback: Option<&str>, http: reqwest::Client) -> Self {
        Dns::Doh {
            http,
            url: url.to_string(),
            fallback: fallback.map(str::to_string),
        }
    }

    /// Un lookup, sans retry (le retry est dans `naptr_smp_url`).
    async fn lookup_once(&self, host: &str) -> SmlLookup {
        match self {
            Dns::System(resolver) => {
                use hickory_proto::rr::{RData, RecordType};
                use hickory_resolver::error::ResolveErrorKind;
                // Point final = FQDN absolu, sinon hickory tente aussi les
                // search domains système (ex. « home » dérivé du hostname
                // derrière une box) : dès que le résolveur rate-limite la
                // requête principale (REFUSED sous rafale), le NXDOMAIN du
                // routeur sur <host>.home devenait un faux « absent »
                // silencieux (constaté le 2026-07-14 : 8 751 faux négatifs
                // sur 51 092).
                match resolver.lookup(format!("{host}."), RecordType::NAPTR).await {
                    Ok(answers) => {
                        let records: Vec<(String, String)> = answers
                            .iter()
                            .filter_map(|r| match r {
                                RData::NAPTR(n) => Some((
                                    String::from_utf8_lossy(n.services()).into_owned(),
                                    String::from_utf8_lossy(n.regexp()).into_owned(),
                                )),
                                _ => None,
                            })
                            .collect();
                        match smp_url_from_naptr(&records) {
                            Some(url) => SmlLookup::Found(url),
                            None => SmlLookup::Failed("NoAnswer".into()),
                        }
                    }
                    Err(e) => match e.kind() {
                        ResolveErrorKind::NoRecordsFound { response_code, .. } => {
                            if *response_code == hickory_proto::op::ResponseCode::NXDomain {
                                SmlLookup::NotRegistered
                            } else {
                                SmlLookup::Failed("NoAnswer".into())
                            }
                        }
                        _ => SmlLookup::Failed(format!("DNS_ERROR:{e}")),
                    },
                }
            }
            // Failover DoH : le secours n'est consulté que sur échec du
            // principal (Failed — panne, HTTP non-2xx, décodage) ; Found et
            // NXDOMAIN sont des réponses définitives. Même politique que le
            // classique (hickory UserProvidedOrder) : failover pur, et le
            // verdict retourné est celui du dernier interrogé.
            Dns::Doh { http, url, fallback } => {
                let outcome = Self::doh_lookup(http, url, host).await;
                match (&outcome, fallback) {
                    (SmlLookup::Failed(_), Some(fb)) => Self::doh_lookup(http, fb, host).await,
                    _ => outcome,
                }
            }
            #[cfg(test)]
            Dns::Fake(map) => map
                .get(host)
                .cloned()
                .unwrap_or(SmlLookup::NotRegistered),
        }
    }

    /// GET RFC 8484 : ?dns=base64url(message DNS), Accept application/dns-message.
    async fn doh_lookup(http: &reqwest::Client, url: &str, host: &str) -> SmlLookup {
        use hickory_proto::op::{Message, MessageType, OpCode, Query, ResponseCode};
        use hickory_proto::rr::{Name, RData, RecordType};

        let name = match Name::from_utf8(host) {
            Ok(n) => n,
            Err(e) => return SmlLookup::Failed(format!("DNS_ERROR:nom invalide: {e}")),
        };
        let mut msg = Message::new();
        msg.set_message_type(MessageType::Query)
            .set_op_code(OpCode::Query)
            .set_recursion_desired(true)
            .add_query(Query::query(name, RecordType::NAPTR));
        let wire = match msg.to_vec() {
            Ok(v) => v,
            Err(e) => return SmlLookup::Failed(format!("DNS_ERROR:encodage: {e}")),
        };
        let sep = if url.contains('?') { '&' } else { '?' };
        let full = format!("{url}{sep}dns={}", BASE64URL_NOPAD.encode(&wire));
        let resp = match http
            .get(&full)
            .header("Accept", "application/dns-message")
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => return SmlLookup::Failed(format!("DNS_ERROR:DoH: {e}")),
        };
        if !resp.status().is_success() {
            return SmlLookup::Failed(format!("DNS_ERROR:DoH HTTP {}", resp.status().as_u16()));
        }
        let body = match resp.bytes().await {
            Ok(b) => b,
            Err(e) => return SmlLookup::Failed(format!("DNS_ERROR:DoH: {e}")),
        };
        let parsed = match Message::from_vec(&body) {
            Ok(m) => m,
            Err(e) => return SmlLookup::Failed(format!("DNS_ERROR:DoH décodage: {e}")),
        };
        if parsed.response_code() == ResponseCode::NXDomain {
            return SmlLookup::NotRegistered;
        }
        if parsed.response_code() != ResponseCode::NoError {
            return SmlLookup::Failed(format!("DNS_ERROR:{}", parsed.response_code()));
        }
        let records: Vec<(String, String)> = parsed
            .answers()
            .iter()
            .filter_map(|r| match r.data() {
                Some(RData::NAPTR(n)) => Some((
                    String::from_utf8_lossy(n.services()).into_owned(),
                    String::from_utf8_lossy(n.regexp()).into_owned(),
                )),
                _ => None,
            })
            .collect();
        match smp_url_from_naptr(&records) {
            Some(url) => SmlLookup::Found(url),
            None => SmlLookup::Failed("NoAnswer".into()),
        }
    }

    /// Lookup avec retries sur échec transitoire. NXDOMAIN et Found sont
    /// définitifs et sortent immédiatement. `sem` borne la rafale DNS ;
    /// le permis est relâché pendant le backoff entre deux tentatives.
    pub async fn naptr_smp_url(&self, host: &str, sem: &tokio::sync::Semaphore) -> SmlLookup {
        let mut last = SmlLookup::Failed("NoAnswer".into());
        for attempt in 0..=DNS_MAX_RETRIES {
            let outcome = {
                let _permit = sem.acquire().await.expect("sémaphore DNS fermé");
                self.lookup_once(host).await
            };
            match outcome {
                SmlLookup::Failed(status) => {
                    last = SmlLookup::Failed(status);
                    if attempt < DNS_MAX_RETRIES {
                        tokio::time::sleep(Duration::from_millis(
                            DNS_RETRY_BASE_MS * (1 << attempt),
                        ))
                        .await;
                    }
                }
                definitive => return definitive,
            }
        }
        last
    }
}

/// Client de résolution directe — même contrat que l'API pour le moteur :
/// des ApiItem, un CallStats par « appel » (ici : un paquet d'adressages,
/// chacun résolu par son propre pipeline DNS+SMP).
#[derive(Clone)]
pub struct DirectClient {
    http: reqwest::Client,
    dns: Arc<Dns>,
    /// Rafale DNS partagée par tous les clones du client (les workers).
    dns_sem: Arc<tokio::sync::Semaphore>,
    sml_zone: String,
}

/// Forme validée du couple résolveur/secours, sans rien construire —
/// partagée entre la validation des réglages (config::validate_api, pour
/// refuser dès l'enregistrement) et la fabrique `dns_from_spec`.
pub enum ResolverSpec {
    System,
    /// Principal puis secours éventuel (failover, l'ordre compte).
    Classic(Vec<std::net::IpAddr>),
    Doh { url: String, fallback: Option<String> },
}

/// Vide = DNS système ; une IP = DNS classique sur ce serveur ; une URL
/// https = DoH (RFC 8484). Tout le reste est une erreur explicite — jamais
/// de repli silencieux sur le DNS système.
/// `fallback` : résolveur de secours (failover pur, pas de lissage), de même
/// nature que le principal — IP derrière une IP, URL https derrière une URL
/// https ; le panachage est refusé (jamais de changement de transport
/// silencieux). Ignoré en DNS système (pas de principal explicite — le champ,
/// toujours renseigné par l'IHM, n'y a pas de sens), et quand il est vide ou
/// identique au principal.
pub fn parse_resolver_spec(
    spec: Option<&str>,
    fallback: Option<&str>,
) -> Result<ResolverSpec, String> {
    let Some(spec) = spec.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(ResolverSpec::System);
    };
    if spec.starts_with("https://") {
        let fb = match fallback.map(str::trim).filter(|s| !s.is_empty() && *s != spec) {
            Some(fb) if fb.starts_with("https://") => Some(fb.to_string()),
            Some(fb) => {
                return Err(format!(
                    "résolveur de secours « {fb} » : attendu une URL https://… \
                     comme le principal (ou vide) — pas de panachage DoH/DNS classique"
                ))
            }
            None => None,
        };
        return Ok(ResolverSpec::Doh {
            url: spec.to_string(),
            fallback: fb,
        });
    }
    match spec.parse::<std::net::IpAddr>() {
        Ok(ip) => {
            let mut ips = vec![ip];
            if let Some(fb) = fallback.map(str::trim).filter(|s| !s.is_empty()) {
                let fb_ip = fb.parse::<std::net::IpAddr>().map_err(|_| {
                    format!("résolveur de secours « {fb} » : attendu une IP (ou vide)")
                })?;
                if fb_ip != ip {
                    ips.push(fb_ip);
                }
            }
            Ok(ResolverSpec::Classic(ips))
        }
        Err(_) => Err(format!(
            "résolveur « {spec} » : attendu une IP (DNS classique), \
             une URL https://… (DoH), ou vide (DNS système)"
        )),
    }
}

/// Résolveur depuis la config : voir `parse_resolver_spec` pour le contrat.
pub fn dns_from_spec(
    spec: Option<&str>,
    fallback: Option<&str>,
    http: &reqwest::Client,
) -> Result<Dns, String> {
    match parse_resolver_spec(spec, fallback)? {
        ResolverSpec::System => Dns::system(),
        ResolverSpec::Classic(ips) => Ok(Dns::udp(&ips)),
        ResolverSpec::Doh { url, fallback } => {
            Ok(Dns::doh(&url, fallback.as_deref(), http.clone()))
        }
    }
}

impl DirectClient {
    /// `resolver`/`resolver_fallback` : voir `dns_from_spec` ;
    /// `dns_concurrency` : lookups DNS simultanés (config, défaut 32). Le
    /// proxy s'applique aux requêtes SMP et au DoH (le DNS classique, lui,
    /// part en direct sur UDP/53).
    pub fn new(
        resolver: Option<&str>,
        resolver_fallback: Option<&str>,
        dns_concurrency: u32,
        proxy_url: Option<&str>,
        creds: Option<&ProxyCreds>,
    ) -> Result<Self, String> {
        let mut b = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(10))
            // Sans plafond, les connexions inactives (90 s de keep-alive)
            // vers des dizaines de SMP distincts s'accumulent — une des
            // sources de l'EMFILE du 2026-07-14 sur macOS.
            .pool_max_idle_per_host(4);
        if let Some(purl) = proxy_url {
            let mut p = reqwest::Proxy::all(purl).map_err(|e| format!("proxy : {e}"))?;
            if let Some(c) = creds {
                p = p.basic_auth(&c.username, &c.password);
            }
            b = b.proxy(p);
        }
        let http = b.build().map_err(|e| e.to_string())?;
        let dns = dns_from_spec(resolver, resolver_fallback, &http)?;
        if dns_concurrency == 0 {
            // Un sémaphore à 0 permis bloquerait tout run, silencieusement.
            return Err("dns_concurrency doit être ≥ 1".into());
        }
        Ok(DirectClient {
            http,
            dns: Arc::new(dns),
            dns_sem: Arc::new(tokio::sync::Semaphore::new(dns_concurrency as usize)),
            sml_zone: SML_PROD.to_string(),
        })
    }

    #[cfg(test)]
    fn for_tests(dns: Dns, sml_zone: &str) -> Self {
        Self::for_tests_http(dns, sml_zone, reqwest::Client::new())
    }

    #[cfg(test)]
    fn for_tests_http(dns: Dns, sml_zone: &str, http: reqwest::Client) -> Self {
        DirectClient {
            http,
            dns: Arc::new(dns),
            dns_sem: Arc::new(tokio::sync::Semaphore::new(
                DNS_CONCURRENCY_DEFAULT as usize,
            )),
            sml_zone: sml_zone.to_string(),
        }
    }

    /// Sonde le chemin réseau avant un run derrière proxy : un GET dont
    /// toute réponse HTTP (même 4xx/5xx, sauf 407) vaut succès — elle prouve
    /// que le proxy achemine. Un échec d'envoi (tunnel refusé, proxy
    /// injoignable) ou un 407 bloquent le lancement avec la cause, au lieu
    /// de laisser le run labourer tout le fichier en erreurs.
    pub async fn preflight_proxy(&self, probe_url: &str) -> Result<(), String> {
        match self.http.get(probe_url).send().await {
            Ok(resp) if resp.status().as_u16() == 407 => Err(
                "Le proxy refuse les identifiants (407). Vérifiez-les dans ⚙.".into(),
            ),
            Ok(_) => Ok(()),
            Err(e) => Err(format!(
                "Le proxy n'achemine pas les requêtes ({}). Vérifiez l'URL du \
                 proxy et les identifiants dans ⚙.",
                root_cause(&e)
            )),
        }
    }

    /// Résout un paquet : chaque adressage traverse son pipeline. Seule une
    /// authentification proxy manquante fait échouer l'appel entier (elle
    /// bloquerait tout le run) ; tout le reste est une erreur item.
    pub async fn resolve_batch(
        &self,
        pids: &[String],
    ) -> Result<(Vec<ApiItem>, CallStats), ApiError> {
        let t0 = Instant::now();
        let mut items = Vec::with_capacity(pids.len());
        // Statuts réels des GET SMP du paquet (0 = échec de connexion) :
        // l'histogramme HTTP du run reflète les SMP, pas un 200 synthétique.
        let mut smp_http = BTreeMap::new();
        for pid in pids {
            items.push(self.resolve_one(pid, &mut smp_http).await?);
        }
        Ok((
            items,
            CallStats {
                http_status: 200,
                latency_ms: t0.elapsed().as_millis() as u64,
                smp_http: Some(smp_http),
            },
        ))
    }

    async fn resolve_one(
        &self,
        pid: &str,
        smp_http: &mut BTreeMap<u16, u32>,
    ) -> Result<ApiItem, ApiError> {
        let (scheme, value) = match pid.split_once("::") {
            Some((s, v)) if !s.is_empty() && !v.is_empty() => (s, v),
            _ => (DEFAULT_SCHEME, pid),
        };
        let pid_full = format!("{scheme}::{value}");
        let host = sml_hostname(scheme, value, &self.sml_zone);

        let smp_url = match self.dns.naptr_smp_url(&host, &self.dns_sem).await {
            SmlLookup::NotRegistered => {
                return Ok(item_base(&pid_full, Some(false), Some(false)));
            }
            SmlLookup::Failed(status) => {
                // DNS_ERROR embarque le message hickory (nom d'hôte variable
                // par participant) : motif borné pour la télémétrie, détail
                // complet dans la note.
                return Ok(match status.strip_prefix("DNS_ERROR:") {
                    Some(detail) => {
                        let mut it = item_error(&pid_full, "SML lookup: erreur DNS");
                        it.note = Some(format!("{host} : {detail}"));
                        it
                    }
                    None => item_error(&pid_full, &format!("SML lookup: {status}")),
                });
            }
            SmlLookup::Found(url) => url,
        };

        // ServiceGroup : liste des doctypes annoncés (dans les hrefs).
        let sg_url = format!(
            "{}/{}",
            smp_url.trim_end_matches('/'),
            utf8_percent_encode(&pid_full, PID_ENCODE)
        );
        let sg_xml = match self.get_text(&sg_url, smp_http).await? {
            Ok(xml) => xml,
            Err(msg) => {
                // Enregistré mais catalogue illisible : exists=true, on ne
                // peut conclure ni sur CTC ni sur la PA (parité simple_view).
                let mut it = item_base(&pid_full, Some(true), None);
                it.note = Some(format!("ServiceGroup {msg} on {smp_url}"));
                return Ok(it);
            }
        };
        let hrefs = service_metadata_refs(&sg_xml);
        let doctypes: Vec<String> = hrefs.iter().filter_map(|h| doctype_from_href(h)).collect();
        let supports = doctypes.iter().any(|d| d == FR_CTC_PRIMARY_INVOICE);

        let mut item = item_base(&pid_full, Some(true), Some(supports));
        // ServiceMetadata ciblé : le doctype CTC si supporté, sinon le
        // premier — miroir de _pick_primary_ap.
        let target = if supports {
            hrefs
                .iter()
                .zip(doctypes.iter())
                .find(|(_, d)| *d == FR_CTC_PRIMARY_INVOICE)
                .map(|(h, _)| h)
        } else {
            hrefs.first()
        };
        if let Some(href) = target {
            if let Ok(Ok(md_xml)) = self.get_text(href, smp_http).await.map(|r| r) {
                if let Some(cert_b64) = first_certificate(&md_xml) {
                    if let Some(pa) = pa_from_cert(&cert_b64) {
                        item.pa = Some(pa);
                    }
                }
            }
        }
        Ok(item)
    }

    /// GET texte. Err(ApiError) uniquement pour l'auth proxy (bloquant run) ;
    /// Ok(Err(msg)) pour les échecs propres à cet adressage. Chaque tentative
    /// crédite `smp_http` de son statut (0 = échec de connexion).
    async fn get_text(
        &self,
        url: &str,
        smp_http: &mut BTreeMap<u16, u32>,
    ) -> Result<Result<String, String>, ApiError> {
        let resp = match self
            .http
            .get(url)
            .header("Accept", "application/xml")
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                // Même détection que ApiClient::map_send_err : le 407 du
                // tunnel CONNECT sort en erreur de connexion.
                let mut cur: Option<&(dyn std::error::Error + 'static)> = Some(&e);
                while let Some(err) = cur {
                    if err.to_string().contains("proxy authorization required") {
                        return Err(ApiError::ProxyAuth);
                    }
                    cur = err.source();
                }
                // Cause profonde plutôt que le Display reqwest complet : son
                // URL variable rendrait chaque motif/note distinct.
                *smp_http.entry(0).or_insert(0) += 1;
                return Ok(Err(format!("fetch: {}", root_cause(&e))));
            }
        };
        let status = resp.status().as_u16();
        *smp_http.entry(status).or_insert(0) += 1;
        if status == 407 {
            return Err(ApiError::ProxyAuth);
        }
        if status != 200 {
            return Ok(Err(format!("HTTP {status}")));
        }
        match resp.text().await {
            Ok(t) => Ok(Ok(t)),
            Err(e) => Ok(Err(format!("fetch: {e}"))),
        }
    }
}

/// Dernier maillon de la chaîne d'erreurs : la cause réelle (ex. « tunnel
/// error: unsuccessful »), débarrassée des enrobages reqwest/hyper.
fn root_cause(e: &(dyn std::error::Error + 'static)) -> String {
    let mut cur = e;
    while let Some(src) = cur.source() {
        cur = src;
    }
    cur.to_string()
}

fn item_base(pid_full: &str, exists: Option<bool>, supports: Option<bool>) -> ApiItem {
    ApiItem {
        participant_id: Some(pid_full.to_string()),
        participant: None,
        exists,
        pa: None,
        supports_extended_ctc_fr: supports,
        note: None,
        error: None,
    }
}

fn item_error(pid_full: &str, error: &str) -> ApiItem {
    ApiItem {
        participant_id: Some(pid_full.to_string()),
        participant: None,
        exists: None,
        pa: None,
        supports_extended_ctc_fr: None,
        note: None,
        error: Some(error.to_string()),
    }
}

/// hrefs des ServiceMetadataReference d'un ServiceGroup (noms locaux
/// uniquement, comme peppol_resolver._local — les SMP varient les préfixes).
fn service_metadata_refs(xml: &str) -> Vec<String> {
    let Ok(doc) = roxmltree::Document::parse(xml) else {
        return Vec::new();
    };
    doc.descendants()
        .filter(|n| n.tag_name().name() == "ServiceMetadataReference")
        .filter_map(|n| n.attribute("href").map(str::to_string))
        .filter(|h| !h.is_empty())
        .collect()
}

/// Doctype URL-encodé dans le href : dernier segment après « /services/ »,
/// de la forme « {scheme}::{value} » (spec SMP — scheme = busdox-docid-qns).
/// On ne garde que value, seule comparable à FR_CTC_PRIMARY_INVOICE (parité
/// avec le DocumentIdentifier des ServiceMetadata côté Python). Le scheme
/// se reconnaît à l'absence de « : » avant le premier « :: » — un doctype
/// nu commence par « urn:… » et son « :: » interne n'est pas un séparateur.
fn doctype_from_href(href: &str) -> Option<String> {
    let (_, enc) = href.rsplit_once("/services/")?;
    let decoded = percent_decode_str(enc).decode_utf8_lossy().into_owned();
    match decoded.split_once("::") {
        Some((scheme, value)) if !scheme.contains(':') && !value.is_empty() => {
            Some(value.to_string())
        }
        _ => Some(decoded),
    }
}

/// Premier élément <Certificate> (contenu base64) d'un ServiceMetadata.
fn first_certificate(xml: &str) -> Option<String> {
    let doc = roxmltree::Document::parse(xml).ok()?;
    doc.descendants()
        .find(|n| n.tag_name().name() == "Certificate")
        .and_then(|n| n.text())
        .map(str::to_string)
}

/// PA depuis le certificat AS4 : CN = code (ex. PFR000123), O = nom,
/// C = pays. Tolère l'enrobage PEM et les espaces (parité parse_cert).
fn pa_from_cert(b64: &str) -> Option<PaInfo> {
    let body: String = b64
        .lines()
        .filter(|l| !l.trim_start().starts_with("-----"))
        .collect::<Vec<_>>()
        .join("");
    let cleaned: String = body.split_whitespace().collect();
    let der = BASE64.decode(cleaned.as_bytes()).ok()?;
    let (_, cert) = x509_parser::parse_x509_certificate(&der).ok()?;
    let subject = cert.subject();
    let attr = |oid: &x509_parser::der_parser::Oid| -> Option<String> {
        subject
            .iter_by_oid(oid)
            .next()
            .and_then(|a| a.as_str().ok())
            .map(str::to_string)
    };
    use x509_parser::oid_registry;
    Some(PaInfo {
        code: attr(&oid_registry::OID_X509_COMMON_NAME),
        name: attr(&oid_registry::OID_X509_ORGANIZATION_NAME),
        country: attr(&oid_registry::OID_X509_COUNTRY_NAME),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// Certificat autosigné généré pour les tests :
    /// subject C=FR, O=Exemple SAS, CN=PFR000123.
    const TEST_CERT_B64: &str = "MIIC6jCCAdICCQC2BKO7mVBfrTANBgkqhkiG9w0BAQsFADA3MQswCQYDVQQGEwJGUjEUMBIGA1UECgwLRXhlbXBsZSBTQVMxEjAQBgNVBAMMCVBGUjAwMDEyMzAeFw0yNjA3MTMxODM4NTBaFw0zNjA3MTAxODM4NTBaMDcxCzAJBgNVBAYTAkZSMRQwEgYDVQQKDAtFeGVtcGxlIFNBUzESMBAGA1UEAwwJUEZSMDAwMTIzMIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEAp8wvKtv4xV/F7Q1aQ74cIPFVBEotUIARWH4maIGwyDVYJxaAhQiJChRNScsiHWW7KRcckkvzxURF3UbglAQbaqTYRW+Rwkxoe/8PliDjaSV9crDThuKMOdTXYKfnCx4bxRw+a2dRpw1b6cQg1DB8jN9gXFFIAxWkuWM/kXYetWU1AOSnPq2d3/4QZXSdyL/GySYkcd3BiQLGVczFHP5dPwdTcHCPVoUMvZmOlnyDSKbIQ3znci8PxaJok8Uxy+h34UIsyC8C+LmYL9iv2E6qEGujWAKSAgeNhp9lgbE8VWJW0cCYHgZCqaciGLCCp+SmqKQ6snjSZF0yBCAHWM8wTwIDAQABMA0GCSqGSIb3DQEBCwUAA4IBAQAynR1gZbx42nYP+QL7mFsBm6TruJKCnUj3chdXtqk3BFrk1A+0wtz++UR87XePd+P/BV1Q2a/ZEOriEBO05FjhpWqNdVBp6xuetr8WVWZ1LySsFbJpAX53hsszxeTYnXJMt4YOf/rcxm78VHcKZGZ/YDDyo9diI4VC1T6CdxH/Cil9K+wR9x6VicePnCmd/qhYzgW9vtaQWBZXAllySJyI01TbEN2J4tqhQsD+u6vmxHEybHFNAEBePCjeVFEevaMQndeIuG8Od3KmVdHSLGoJcC1kMlVh/hP5y4KrNBSxUlqkKk233CAVakKIQyA0QP2m4wOlPLW+PoF+Gl/WR+PE";

    #[test]
    fn sml_hostname_parite_python() {
        // Golden values générées avec peppol_resolver.sml_hostname.
        assert_eq!(
            sml_hostname("iso6523-actorid-upis", "0225:000122308", SML_PROD),
            "3sb4kv7i2hrpws6k3tiauwp66tcjrhyzptugtoikng3triql7gga.\
             iso6523-actorid-upis.participant.sml.prod.tech.peppol.org"
        );
        // La casse de la valeur est neutralisée avant hachage.
        assert_eq!(
            sml_hostname("iso6523-actorid-upis", "0009:MixedCase", SML_PROD),
            sml_hostname("iso6523-actorid-upis", "0009:mixedcase", SML_PROD)
        );
    }

    #[test]
    fn doctype_decode_depuis_href() {
        // href réel (SMP spec) : segment « {scheme}::{value} » URL-encodé.
        // Seul value est le doctype comparable à FR_CTC_PRIMARY_INVOICE —
        // garder le préfixe busdox-docid-qns:: rendait supports toujours
        // false (0 % de CTC constaté en prod le 2026-07-14).
        let href = "http://smp.example/iso6523-actorid-upis%3A%3A0225%3A1/services/\
                    busdox-docid-qns%3A%3Aurn%3Aoasis%3Anames%3Aspecification%3Aubl%3A\
                    schema%3Axsd%3AInvoice-2%3A%3AInvoice%23%23urn%3Acen.eu%3Aen16931%3A2017";
        assert_eq!(
            doctype_from_href(href).unwrap(),
            "urn:oasis:names:specification:ubl:schema:xsd:\
             Invoice-2::Invoice##urn:cen.eu:en16931:2017"
        );
        // Sans préfixe de schéma (SMP non conforme) : la valeur est gardée
        // telle quelle — le « :: » interne au doctype n'est pas un séparateur.
        let href_nu = "http://smp.example/x/services/\
                       urn%3Aoasis%3Ax%3AInvoice-2%3A%3AInvoice%23%23urn%3Acen.eu%3Aen16931%3A2017";
        assert_eq!(
            doctype_from_href(href_nu).unwrap(),
            "urn:oasis:x:Invoice-2::Invoice##urn:cen.eu:en16931:2017"
        );
        assert!(doctype_from_href("http://smp.example/sans-services").is_none());
    }

    #[test]
    fn resolver_spec_vide_ip_url_et_erreur() {
        let http = reqwest::Client::new();
        // Vide ou absent : DNS système.
        assert!(matches!(dns_from_spec(None, None, &http).unwrap(), Dns::System(_)));
        assert!(matches!(dns_from_spec(Some("  "), None, &http).unwrap(), Dns::System(_)));
        // Une IP (v4 ou v6) : DNS classique sur ce serveur.
        assert!(matches!(
            dns_from_spec(Some("8.8.8.8"), None, &http).unwrap(),
            Dns::System(_)
        ));
        assert!(matches!(
            dns_from_spec(Some("2001:4860:4860::8888"), None, &http).unwrap(),
            Dns::System(_)
        ));
        // Une URL https : DoH, URL conservée telle quelle.
        match dns_from_spec(Some("https://1.1.1.1/dns-query"), None, &http).unwrap() {
            Dns::Doh { url, .. } => assert_eq!(url, "https://1.1.1.1/dns-query"),
            _ => panic!("attendu Dns::Doh"),
        }
        // Tout le reste : erreur explicite (jamais un repli silencieux).
        match dns_from_spec(Some("dns.google"), None, &http) {
            Err(err) => assert!(err.contains("dns.google"), "message : {err}"),
            Ok(_) => panic!("un hostname nu doit être refusé"),
        }
    }

    #[test]
    fn resolver_fallback_failover_du_mode_classique() {
        let http = reqwest::Client::new();
        // Principal IP + secours IP : accepté (failover hickory,
        // UserProvidedOrder — le principal reste préféré).
        assert!(matches!(
            dns_from_spec(Some("8.8.8.8"), Some("1.1.1.1"), &http).unwrap(),
            Dns::System(_)
        ));
        // Secours vide ou identique au principal : accepté (pas de doublon).
        assert!(dns_from_spec(Some("8.8.8.8"), Some("  "), &http).is_ok());
        assert!(dns_from_spec(Some("8.8.8.8"), Some("8.8.8.8"), &http).is_ok());
        // Principal IP + secours invalide : erreur explicite.
        match dns_from_spec(Some("8.8.8.8"), Some("dns.google"), &http) {
            Err(err) => assert!(err.contains("secours"), "message : {err}"),
            Ok(_) => panic!("un secours non-IP doit être refusé"),
        }
        // DNS système : pas de principal explicite, le secours (toujours
        // renseigné par l'IHM, même invalide) est ignoré, jamais une erreur.
        assert!(matches!(
            dns_from_spec(None, Some("n'importe quoi"), &http).unwrap(),
            Dns::System(_)
        ));
    }

    #[test]
    fn resolver_fallback_homogene_en_doh() {
        let http = reqwest::Client::new();
        // Principal DoH + secours DoH : même politique qu'en classique
        // (failover pur, le principal d'abord).
        match dns_from_spec(
            Some("https://a.example/dns-query"),
            Some("https://b.example/dns-query"),
            &http,
        )
        .unwrap()
        {
            Dns::Doh { url, fallback, .. } => {
                assert_eq!(url, "https://a.example/dns-query");
                assert_eq!(fallback.as_deref(), Some("https://b.example/dns-query"));
            }
            _ => panic!("attendu Dns::Doh"),
        }
        // Secours vide ou identique au principal : pas de secours (pas de
        // doublon), comme en classique.
        for fb in [None, Some("  "), Some("https://a.example/dns-query")] {
            match dns_from_spec(Some("https://a.example/dns-query"), fb, &http).unwrap() {
                Dns::Doh { fallback, .. } => {
                    assert!(fallback.is_none(), "secours {fb:?} : attendu aucun")
                }
                _ => panic!("attendu Dns::Doh"),
            }
        }
        // Panachage DoH + IP (ou autre) : erreur explicite — jamais de
        // changement de transport silencieux.
        for fb in ["1.1.1.1", "n'importe quoi"] {
            match dns_from_spec(Some("https://a.example/dns-query"), Some(fb), &http) {
                Err(err) => assert!(err.contains("secours"), "message : {err}"),
                Ok(_) => panic!("un secours non-DoH derrière un principal DoH doit être refusé"),
            }
        }
    }

    /// Réponse DoH binaire (RFC 8484) portant un NAPTR Meta:SMP vers `smp_url`,
    /// ou un NXDOMAIN si `smp_url` est None.
    fn doh_wire_response(host: &str, smp_url: Option<&str>) -> Vec<u8> {
        use hickory_proto::op::{Message, MessageType, ResponseCode};
        use hickory_proto::rr::{rdata::naptr::NAPTR, Name, RData, Record};
        let mut msg = Message::new();
        msg.set_message_type(MessageType::Response);
        match smp_url {
            Some(url) => {
                let name = Name::from_utf8(host).unwrap();
                let naptr = NAPTR::new(
                    100,
                    10,
                    b"U".to_vec().into_boxed_slice(),
                    b"Meta:SMP".to_vec().into_boxed_slice(),
                    format!("!.*!{url}!").into_bytes().into_boxed_slice(),
                    Name::root(),
                );
                msg.add_answer(Record::from_rdata(name, 60, RData::NAPTR(naptr)));
            }
            None => {
                msg.set_response_code(ResponseCode::NXDomain);
            }
        }
        msg.to_vec().unwrap()
    }

    #[tokio::test]
    async fn doh_secours_prend_le_relais_si_principal_en_panne() {
        let principal = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/dns-query"))
            .respond_with(ResponseTemplate::new(500))
            .expect(1)
            .mount(&principal)
            .await;
        let secours = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/dns-query"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                doh_wire_response("h.sml.test", Some("http://smp.example.org")),
                "application/dns-message",
            ))
            .expect(1)
            .mount(&secours)
            .await;
        let dns = Dns::doh(
            &format!("{}/dns-query", principal.uri()),
            Some(&format!("{}/dns-query", secours.uri())),
            reqwest::Client::new(),
        );
        let sem = tokio::sync::Semaphore::new(4);
        match dns.naptr_smp_url("h.sml.test", &sem).await {
            SmlLookup::Found(url) => assert_eq!(url, "http://smp.example.org"),
            autre => panic!("Found attendu via le secours, obtenu {autre:?}"),
        }
    }

    #[tokio::test]
    async fn doh_pas_de_secours_sur_reponse_definitive() {
        // NXDOMAIN du principal = réponse valide et définitive (adressage non
        // enregistré) : le secours ne doit PAS être consulté.
        let principal = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/dns-query"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                doh_wire_response("h.sml.test", None),
                "application/dns-message",
            ))
            .expect(1)
            .mount(&principal)
            .await;
        let secours = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/dns-query"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                doh_wire_response("h.sml.test", Some("http://smp.example.org")),
                "application/dns-message",
            ))
            .expect(0)
            .mount(&secours)
            .await;
        let dns = Dns::doh(
            &format!("{}/dns-query", principal.uri()),
            Some(&format!("{}/dns-query", secours.uri())),
            reqwest::Client::new(),
        );
        let sem = tokio::sync::Semaphore::new(4);
        match dns.naptr_smp_url("h.sml.test", &sem).await {
            SmlLookup::NotRegistered => {}
            autre => panic!("NotRegistered attendu, obtenu {autre:?}"),
        }
    }

    #[test]
    fn naptr_extrait_l_url_meta_smp() {
        let recs = vec![
            ("autre".to_string(), "!.*!http://mauvais!".to_string()),
            (
                "Meta:SMP".to_string(),
                "!.*!http://smp.example.org!".to_string(),
            ),
        ];
        assert_eq!(
            smp_url_from_naptr(&recs).as_deref(),
            Some("http://smp.example.org")
        );
        assert!(smp_url_from_naptr(&[("Meta:SMP".into(), "pas-un-regexp".into())]).is_none());
    }

    #[test]
    fn certificat_donne_la_pa() {
        let pa = pa_from_cert(TEST_CERT_B64).unwrap();
        assert_eq!(pa.code.as_deref(), Some("PFR000123"));
        assert_eq!(pa.name.as_deref(), Some("Exemple SAS"));
        assert_eq!(pa.country.as_deref(), Some("FR"));
        // Enrobage PEM et sauts de ligne tolérés (certains SMP font ça).
        let pem = format!(
            "-----BEGIN CERTIFICATE-----\n{}\n-----END CERTIFICATE-----",
            TEST_CERT_B64
        );
        assert_eq!(pa_from_cert(&pem).unwrap().code.as_deref(), Some("PFR000123"));
    }

    fn sg_xml(smp: &str, doctypes: &[&str]) -> String {
        // Comme les SMP réels : le segment services est « {scheme}::{value} ».
        let refs: String = doctypes
            .iter()
            .map(|d| {
                format!(
                    r#"<smp:ServiceMetadataReference href="{smp}/x/services/{}"/>"#,
                    utf8_percent_encode(&format!("busdox-docid-qns::{d}"), PID_ENCODE)
                )
            })
            .collect();
        format!(
            r#"<?xml version="1.0"?><smp:ServiceGroup xmlns:smp="http://busdox.org/serviceMetadata/publishing/1.0/">
               <smp:ServiceMetadataReferenceCollection>{refs}</smp:ServiceMetadataReferenceCollection>
               </smp:ServiceGroup>"#
        )
    }

    fn md_xml(cert_b64: &str) -> String {
        format!(
            r#"<?xml version="1.0"?><SignedServiceMetadata xmlns="http://busdox.org/serviceMetadata/publishing/1.0/">
               <ServiceMetadata><ServiceInformation>
               <Endpoint><Certificate>{cert_b64}</Certificate></Endpoint>
               </ServiceInformation></ServiceMetadata></SignedServiceMetadata>"#
        )
    }

    fn fake_dns(host: &str, outcome: SmlLookup) -> Dns {
        Dns::Fake(HashMap::from([(host.to_string(), outcome)]))
    }

    const ZONE: &str = "sml.test";
    const PID: &str = "iso6523-actorid-upis::0225:000122308";

    fn host_for_pid() -> String {
        sml_hostname("iso6523-actorid-upis", "0225:000122308", ZONE)
    }

    #[tokio::test]
    async fn pipeline_nominal_exists_ctc_et_pa() {
        let server = MockServer::start().await;
        let sg_path = format!("/{}", utf8_percent_encode(PID, PID_ENCODE));
        Mock::given(method("GET"))
            .and(path(sg_path))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(sg_xml(&server.uri(), &["autre::doctype", FR_CTC_PRIMARY_INVOICE])),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(format!(
                "/x/services/{}",
                utf8_percent_encode(
                    &format!("busdox-docid-qns::{FR_CTC_PRIMARY_INVOICE}"),
                    PID_ENCODE
                )
            )))
            .respond_with(ResponseTemplate::new(200).set_body_string(md_xml(TEST_CERT_B64)))
            .mount(&server)
            .await;

        let c = DirectClient::for_tests(
            fake_dns(&host_for_pid(), SmlLookup::Found(server.uri())),
            ZONE,
        );
        let (items, stats) = c.resolve_batch(&[PID.to_string()]).await.unwrap();
        assert_eq!(stats.http_status, 200);
        let it = &items[0];
        assert_eq!(it.participant_id.as_deref(), Some(PID));
        assert_eq!(it.exists, Some(true));
        assert_eq!(it.supports_extended_ctc_fr, Some(true));
        let pa = it.pa.as_ref().unwrap();
        assert_eq!(pa.code.as_deref(), Some("PFR000123"));
        assert_eq!(pa.name.as_deref(), Some("Exemple SAS"));
        assert_eq!(pa.country.as_deref(), Some("FR"));
        assert!(it.error.is_none());
    }

    /// Smoke test RÉSEAU RÉEL (SML prod + SMP) : hors CI, à lancer à la main
    /// (`cargo test -- --ignored`). Adressage exemple de la doc du resolver
    /// Python (0009:552100554, l'ancien PID du test_key API, est déradié).
    #[tokio::test]
    #[ignore = "réseau réel (SML prod)"]
    async fn resolution_reelle_sur_sml_prod() {
        let c = DirectClient::new(None, None, DNS_CONCURRENCY_DEFAULT, None, None).unwrap();
        let (items, stats) = c
            .resolve_batch(&["iso6523-actorid-upis::0225:000122308".to_string()])
            .await
            .unwrap();
        let it = &items[0];
        assert!(it.error.is_none(), "erreur item : {:?}", it.error);
        assert_eq!(it.exists, Some(true));
        // Ce PID annonce le doctype CTC extended dans son ServiceGroup —
        // garde-fou contre une régression du décodage des hrefs (le préfixe
        // busdox-docid-qns:: rendait supports toujours false, 2026-07-14).
        assert_eq!(it.supports_extended_ctc_fr, Some(true), "item : {it:?}");
        assert!(it.pa.is_some(), "PA attendue, item : {it:?}");
        assert!(stats.latency_ms > 0);
    }

    /// Même smoke test via un DNS public choisi par IP — le chemin
    /// « résolveur custom » (celui qui évite le rate-limiting du FAI).
    #[tokio::test]
    #[ignore = "réseau réel (DNS public + SML prod)"]
    async fn resolution_reelle_via_dns_choisi() {
        let c = DirectClient::new(Some("8.8.8.8"), Some("1.1.1.1"), DNS_CONCURRENCY_DEFAULT, None, None).unwrap();
        let (items, _) = c
            .resolve_batch(&["iso6523-actorid-upis::0225:000122308".to_string()])
            .await
            .unwrap();
        assert_eq!(items[0].exists, Some(true), "item : {:?}", items[0]);
        assert!(items[0].pa.is_some());
    }

    /// Même smoke test via DoH Cloudflare — le chemin « réseau d'entreprise ».
    #[tokio::test]
    #[ignore = "réseau réel (DoH + SML prod)"]
    async fn resolution_reelle_via_doh() {
        let c = DirectClient::new(Some("https://1.1.1.1/dns-query"), None, DNS_CONCURRENCY_DEFAULT, None, None).unwrap();
        let (items, _) = c
            .resolve_batch(&["iso6523-actorid-upis::0225:000122308".to_string()])
            .await
            .unwrap();
        assert_eq!(items[0].exists, Some(true), "item : {:?}", items[0]);
        assert!(items[0].pa.is_some());
    }

    #[tokio::test]
    async fn semaphore_dns_rend_ses_permis_sous_rafale() {
        // 200 lookups concurrents > 8 permis : tous doivent aboutir sans
        // blocage (un permis fuité finirait en deadlock au 9e lookup d'un
        // run ultérieur), et le compteur revient à plein.
        let dns = Arc::new(fake_dns(&host_for_pid(), SmlLookup::NotRegistered));
        let sem = Arc::new(tokio::sync::Semaphore::new(8));
        let host = host_for_pid();
        let tasks: Vec<_> = (0..200)
            .map(|_| {
                let dns = dns.clone();
                let sem = sem.clone();
                let host = host.clone();
                tokio::spawn(async move { dns.naptr_smp_url(&host, &sem).await })
            })
            .collect();
        for t in tasks {
            assert!(matches!(t.await.unwrap(), SmlLookup::NotRegistered));
        }
        assert_eq!(sem.available_permits(), 8);
    }

    #[tokio::test]
    async fn dns_concurrency_configuree_appliquee_et_zero_refuse() {
        let c = DirectClient::new(Some("8.8.8.8"), None, 7, None, None).unwrap();
        assert_eq!(c.dns_sem.available_permits(), 7);
        assert!(DirectClient::new(Some("8.8.8.8"), None, 0, None, None).is_err());
    }

    #[tokio::test]
    async fn nxdomain_donne_exists_false_sans_erreur() {
        let c = DirectClient::for_tests(fake_dns("aucun", SmlLookup::NotRegistered), ZONE);
        let (items, _) = c.resolve_batch(&[PID.to_string()]).await.unwrap();
        assert_eq!(items[0].exists, Some(false));
        assert_eq!(items[0].supports_extended_ctc_fr, Some(false));
        assert!(items[0].error.is_none());
    }

    #[tokio::test]
    async fn echec_dns_donne_une_erreur_item_jamais_absent() {
        // NoAnswer ≠ absent : verdict impossible → erreur re-tentable
        // (règle anti-faux-négatifs de l'API, incident 2026-07-13).
        let c = DirectClient::for_tests(
            fake_dns(&host_for_pid(), SmlLookup::Failed("NoAnswer".into())),
            ZONE,
        );
        let (items, _) = c.resolve_batch(&[PID.to_string()]).await.unwrap();
        assert_eq!(items[0].error.as_deref(), Some("SML lookup: NoAnswer"));
        assert!(items[0].exists.is_none());
    }

    #[tokio::test]
    async fn preflight_bloque_sur_proxy_qui_refuse_le_tunnel() {
        // Proxy répondant 403 au CONNECT (créds faux) : le preflight refuse
        // de lancer le run, avec la cause — au lieu de laisser le run
        // labourer tout le fichier en erreurs (run proxy du 15/07).
        let proxy = fake_proxy_403().await;
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .proxy(
                reqwest::Proxy::all(format!("http://{proxy}"))
                    .unwrap()
                    .basic_auth("user", "faux"),
            )
            .build()
            .unwrap();
        let c = DirectClient::for_tests_http(fake_dns("aucun", SmlLookup::NotRegistered), ZONE, http);
        let err = c.preflight_proxy("https://sonde.exemple/").await.unwrap_err();
        assert!(err.contains("proxy"), "{err}");
        assert!(err.contains("tunnel error: unsuccessful"), "{err}");
    }

    #[tokio::test]
    async fn preflight_passe_sur_toute_reponse_http() {
        // Une réponse HTTP — même 500 — prouve que le chemin réseau
        // fonctionne : seuls les échecs d'acheminement bloquent le run.
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;
        let c = DirectClient::for_tests(fake_dns("aucun", SmlLookup::NotRegistered), ZONE);
        assert!(c.preflight_proxy(&server.uri()).await.is_ok());
    }

    #[tokio::test]
    async fn preflight_bloque_sur_reponse_407() {
        // 407 en réponse directe (proxy http sans CONNECT) : identifiants
        // refusés, même verdict que le tunnel.
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(407))
            .mount(&server)
            .await;
        let c = DirectClient::for_tests(fake_dns("aucun", SmlLookup::NotRegistered), ZONE);
        let err = c.preflight_proxy(&server.uri()).await.unwrap_err();
        assert!(err.contains("proxy"), "{err}");
    }

    #[tokio::test]
    async fn stats_portent_les_statuts_reels_des_get_smp() {
        // L'histogramme HTTP du run affichait un 200 synthétique par paquet
        // quel que soit le sort des GET SMP : les stats portent désormais la
        // carte des statuts réels (ici : ServiceGroup en 403).
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(403))
            .mount(&server)
            .await;
        let c = DirectClient::for_tests(
            fake_dns(&host_for_pid(), SmlLookup::Found(server.uri())),
            ZONE,
        );
        let (_, stats) = c.resolve_batch(&[PID.to_string()]).await.unwrap();
        assert_eq!(
            stats.smp_http,
            Some(std::collections::BTreeMap::from([(403u16, 1u32)]))
        );
    }

    #[tokio::test]
    async fn echec_dns_detaille_motif_borne_et_note() {
        // Les DNS_ERROR embarquent le message hickory, qui contient le nom
        // d'hôte (variable par participant) : en motif de télémétrie ils
        // explosaient le Top erreurs en « (autres) » (run proxy du 15/07).
        // Motif borné, détail dans la note.
        let c = DirectClient::for_tests(
            fake_dns(
                &host_for_pid(),
                SmlLookup::Failed("DNS_ERROR:proto error: timeout sur xyz.sml.test".into()),
            ),
            ZONE,
        );
        let (items, _) = c.resolve_batch(&[PID.to_string()]).await.unwrap();
        assert_eq!(items[0].error.as_deref(), Some("SML lookup: erreur DNS"));
        let note = items[0].note.as_deref().unwrap();
        assert!(note.contains("timeout sur xyz.sml.test"), "{note}");
        assert!(items[0].exists.is_none());
    }

    /// Faux proxy : répond 403 à tout CONNECT (créds refusés) puis ferme.
    async fn fake_proxy_403() -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else {
                    break;
                };
                let mut buf = [0u8; 4096];
                use tokio::io::{AsyncReadExt, AsyncWriteExt};
                let _ = sock.read(&mut buf).await;
                let _ = sock
                    .write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n")
                    .await;
            }
        });
        addr
    }

    #[tokio::test]
    async fn echec_proxy_tunnel_note_cause_profonde() {
        // Proxy répondant 403 au CONNECT (créds faux) : la note porte la
        // cause profonde de la chaîne reqwest (« tunnel error: unsuccessful »)
        // et l'URL du SMP — pas le message reqwest complet, dont l'URL
        // variable rendait chaque motif distinct.
        let proxy = fake_proxy_403().await;
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .proxy(
                reqwest::Proxy::all(format!("http://{proxy}"))
                    .unwrap()
                    .basic_auth("user", "faux"),
            )
            .build()
            .unwrap();
        let c = DirectClient::for_tests_http(
            fake_dns(
                &host_for_pid(),
                SmlLookup::Found("https://smp.exemple".into()),
            ),
            ZONE,
            http,
        );
        let (items, _) = c.resolve_batch(&[PID.to_string()]).await.unwrap();
        let it = &items[0];
        assert_eq!(it.exists, Some(true));
        let note = it.note.as_deref().unwrap();
        assert!(note.contains("tunnel error: unsuccessful"), "{note}");
        assert!(note.contains("https://smp.exemple"), "{note}");
        assert!(!note.contains("error sending request"), "{note}");
    }

    #[tokio::test]
    async fn service_group_indisponible_exists_true_sans_verdict_ctc() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(403))
            .mount(&server)
            .await;
        let c = DirectClient::for_tests(
            fake_dns(&host_for_pid(), SmlLookup::Found(server.uri())),
            ZONE,
        );
        let (items, _) = c.resolve_batch(&[PID.to_string()]).await.unwrap();
        let it = &items[0];
        assert_eq!(it.exists, Some(true));
        assert!(it.supports_extended_ctc_fr.is_none()); // pas de verdict
        assert!(it.note.as_deref().unwrap().contains("HTTP 403"));
        assert!(it.error.is_none());
    }

    #[tokio::test]
    async fn sans_doctype_ctc_supports_false_et_pa_du_premier_href() {
        let server = MockServer::start().await;
        let sg_path = format!("/{}", utf8_percent_encode(PID, PID_ENCODE));
        Mock::given(method("GET"))
            .and(path(sg_path))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(sg_xml(&server.uri(), &["autre::doctype"])),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(format!(
                "/x/services/{}",
                utf8_percent_encode("busdox-docid-qns::autre::doctype", PID_ENCODE)
            )))
            .respond_with(ResponseTemplate::new(200).set_body_string(md_xml(TEST_CERT_B64)))
            .mount(&server)
            .await;
        let c = DirectClient::for_tests(
            fake_dns(&host_for_pid(), SmlLookup::Found(server.uri())),
            ZONE,
        );
        let (items, _) = c.resolve_batch(&[PID.to_string()]).await.unwrap();
        assert_eq!(items[0].exists, Some(true));
        assert_eq!(items[0].supports_extended_ctc_fr, Some(false));
        assert_eq!(
            items[0].pa.as_ref().unwrap().code.as_deref(),
            Some("PFR000123")
        );
    }
}
