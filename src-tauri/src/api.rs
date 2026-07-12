use serde::Deserialize;
use std::time::{Duration, Instant};

#[derive(Clone)]
pub struct ProxyCreds {
    pub username: String,
    pub password: String,
}

/// Debug rédigé : le mot de passe proxy ne doit jamais fuiter dans un log
/// `{creds:?}` (même convention que `config::ProxyConfig`).
impl std::fmt::Debug for ProxyCreds {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProxyCreds")
            .field("username", &"***")
            .field("password", &"***")
            .finish()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("Clé API invalide ou révoquée (HTTP {0}).")]
    Auth(u16),
    #[error("Le proxy demande une authentification (HTTP 407).")]
    ProxyAuth,
    #[error("Rate limit atteint (HTTP 429), Retry-After {retry_after_s}s.")]
    RateLimited { retry_after_s: f64 },
    #[error("Erreur de requête (HTTP {0}) — non retentable.")]
    Client(u16),
    #[error("Erreur serveur (HTTP {0}).")]
    Server(u16),
    #[error("Erreur réseau : {0}")]
    Network(String),
}

impl ApiError {
    /// Code HTTP associé, pour la répartition des codes au dashboard
    /// (0 = erreur réseau sans réponse).
    pub fn http_status(&self) -> u16 {
        match self {
            ApiError::Auth(s) | ApiError::Client(s) | ApiError::Server(s) => *s,
            ApiError::ProxyAuth => 407,
            ApiError::RateLimited { .. } => 429,
            ApiError::Network(_) => 0,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct PaInfo {
    pub code: Option<String>,
    pub name: Option<String>,
    pub country: Option<String>,
}

/// Un item de réponse de l'API (format vérifié dans peppol_api.py) :
/// succès = {participant_id, exists, pa{...}, supports_extended_ctc_fr, note} ;
/// échec  = {participant, error}.
#[derive(Debug, Clone, Deserialize)]
pub struct ApiItem {
    #[serde(default)]
    pub participant_id: Option<String>,
    #[serde(default)]
    pub participant: Option<String>,
    #[serde(default)]
    pub exists: Option<bool>,
    #[serde(default)]
    pub pa: Option<PaInfo>,
    #[serde(default)]
    pub supports_extended_ctc_fr: Option<bool>,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct CallStats {
    pub http_status: u16,
    pub latency_ms: u64,
}

#[derive(Clone)]
pub struct ApiClient {
    http: reqwest::Client,
    base: String,
    key: String,
}

impl ApiClient {
    pub fn new(
        base_url: &str,
        key: &str,
        proxy_url: Option<&str>,
        creds: Option<&ProxyCreds>,
    ) -> Result<Self, String> {
        let mut b = reqwest::Client::builder()
            .timeout(Duration::from_secs(75))
            .connect_timeout(Duration::from_secs(10));
        if let Some(purl) = proxy_url {
            let mut p = reqwest::Proxy::all(purl).map_err(|e| format!("proxy : {e}"))?;
            if let Some(c) = creds {
                p = p.basic_auth(&c.username, &c.password);
            }
            b = b.proxy(p);
        }
        Ok(ApiClient {
            http: b.build().map_err(|e| e.to_string())?,
            base: base_url.trim_end_matches('/').to_string(),
            key: key.to_string(),
        })
    }

    /// Même client (même pool/proxy), nouvelle clé — pour la reprise après 401.
    pub fn with_key(&self, key: &str) -> Self {
        ApiClient {
            key: key.to_string(),
            ..self.clone()
        }
    }

    /// Valeur de header pour la clé API, marquée sensible pour que reqwest
    /// la rédige dans ses logs/Debug.
    fn key_header(&self) -> Result<reqwest::header::HeaderValue, ApiError> {
        let mut v = reqwest::header::HeaderValue::from_str(&self.key)
            .map_err(|e| ApiError::Network(format!("clé API invalide en header HTTP : {e}")))?;
        v.set_sensitive(true);
        Ok(v)
    }

    pub async fn resolve_batch(
        &self,
        pids: &[String],
    ) -> Result<(Vec<ApiItem>, CallStats), ApiError> {
        let t0 = Instant::now();
        let resp = self
            .http
            .post(format!("{}/resolve/batch", self.base))
            .header("X-API-Key", self.key_header()?)
            .json(&serde_json::json!({ "participants": pids, "test": false }))
            .send()
            .await
            .map_err(|e| self.map_send_err(e))?;
        let latency_ms = t0.elapsed().as_millis() as u64;
        let status = resp.status().as_u16();
        match status {
            200 => {
                #[derive(Deserialize)]
                struct R {
                    results: Vec<ApiItem>,
                }
                let r: R = resp
                    .json()
                    .await
                    .map_err(|e| ApiError::Network(e.to_string()))?;
                Ok((
                    r.results,
                    CallStats {
                        http_status: 200,
                        latency_ms,
                    },
                ))
            }
            s => Err(Self::status_to_error(s, resp.headers())),
        }
    }

    /// Test unitaire de la clé : une vraie résolution GET /resolve/<pid>.
    pub async fn test_key(&self) -> Result<CallStats, ApiError> {
        let t0 = Instant::now();
        let resp = self
            .http
            .get(format!("{}/resolve/0009:552100554", self.base))
            .header("X-API-Key", self.key_header()?)
            .send()
            .await
            .map_err(|e| self.map_send_err(e))?;
        let latency_ms = t0.elapsed().as_millis() as u64;
        let status = resp.status().as_u16();
        if status == 200 {
            Ok(CallStats {
                http_status: 200,
                latency_ms,
            })
        } else {
            Err(Self::status_to_error(status, resp.headers()))
        }
    }

    /// Connectivité seule (endpoint public /health, sans clé).
    pub async fn health(&self) -> Result<CallStats, ApiError> {
        let t0 = Instant::now();
        let resp = self
            .http
            .get(format!("{}/health", self.base))
            .send()
            .await
            .map_err(|e| self.map_send_err(e))?;
        let latency_ms = t0.elapsed().as_millis() as u64;
        let status = resp.status().as_u16();
        if status == 200 {
            Ok(CallStats {
                http_status: 200,
                latency_ms,
            })
        } else {
            Err(Self::status_to_error(status, resp.headers()))
        }
    }

    fn status_to_error(status: u16, headers: &reqwest::header::HeaderMap) -> ApiError {
        match status {
            401 | 403 => ApiError::Auth(status),
            407 => ApiError::ProxyAuth,
            429 => {
                let retry_after_s = headers
                    .get("Retry-After")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.parse::<f64>().ok())
                    .map(|v| v.clamp(0.0, 3600.0))
                    .unwrap_or(2.0);
                ApiError::RateLimited { retry_after_s }
            }
            s if (400..500).contains(&s) => ApiError::Client(s),
            s => ApiError::Server(s),
        }
    }

    fn map_send_err(&self, e: reqwest::Error) -> ApiError {
        // Un 407 sur le tunnel CONNECT (proxy + HTTPS) sort en erreur de
        // connexion, sans code HTTP : hyper-util met "proxy authorization
        // required" dans la chaîne de sources. On la parcourt plutôt que de
        // chercher "407" dans le message (l'URL y figure : faux positifs).
        let mut cur: Option<&(dyn std::error::Error + 'static)> = Some(&e);
        while let Some(err) = cur {
            if err.to_string().contains("proxy authorization required") {
                return ApiError::ProxyAuth;
            }
            cur = err.source();
        }
        ApiError::Network(e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn pids(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[tokio::test]
    async fn resolve_batch_parse_la_reponse() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/resolve/batch"))
            .and(header("X-API-Key", "BONNE_CLE"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "results": [
                    {"participant_id": "iso6523-actorid-upis::0009:1", "exists": true,
                     "pa": {"code": "PA0042", "name": "ACME PA", "country": "FR"},
                     "supports_extended_ctc_fr": true, "note": null},
                    {"participant": "0009:zz", "error": "Identifiant invalide."}
                ]
            })))
            .mount(&server)
            .await;

        let c = ApiClient::new(&server.uri(), "BONNE_CLE", None, None).unwrap();
        let (items, stats) = c
            .resolve_batch(&pids(&["0009:1", "0009:zz"]))
            .await
            .unwrap();
        assert_eq!(stats.http_status, 200);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].exists, Some(true));
        assert_eq!(
            items[0].pa.as_ref().unwrap().code.as_deref(),
            Some("PA0042")
        );
        assert_eq!(items[1].error.as_deref(), Some("Identifiant invalide."));
    }

    #[tokio::test]
    async fn erreur_401_typee_auth() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/resolve/batch"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;
        let c = ApiClient::new(&server.uri(), "MAUVAISE", None, None).unwrap();
        assert!(matches!(
            c.resolve_batch(&pids(&["0009:1"])).await,
            Err(ApiError::Auth(401))
        ));
    }

    #[tokio::test]
    async fn erreur_429_lit_retry_after() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/resolve/batch"))
            .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "3"))
            .mount(&server)
            .await;
        let c = ApiClient::new(&server.uri(), "K", None, None).unwrap();
        match c.resolve_batch(&pids(&["0009:1"])).await {
            Err(ApiError::RateLimited { retry_after_s }) => assert_eq!(retry_after_s, 3.0),
            other => panic!("attendu RateLimited, obtenu {other:?}"),
        }
    }

    #[tokio::test]
    async fn erreur_5xx_typee_server() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/resolve/batch"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;
        let c = ApiClient::new(&server.uri(), "K", None, None).unwrap();
        assert!(matches!(
            c.resolve_batch(&pids(&["0009:1"])).await,
            Err(ApiError::Server(503))
        ));
    }

    #[tokio::test]
    async fn erreur_400_typee_client_non_retentable() {
        // Une 4xx non mappée (requête malformée) ne doit pas passer pour une
        // erreur serveur : le moteur ne doit pas backoff dessus.
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/resolve/batch"))
            .respond_with(ResponseTemplate::new(400))
            .mount(&server)
            .await;
        let c = ApiClient::new(&server.uri(), "K", None, None).unwrap();
        assert!(matches!(
            c.resolve_batch(&pids(&["0009:1"])).await,
            Err(ApiError::Client(400))
        ));
    }

    #[tokio::test]
    async fn le_corps_batch_a_la_forme_exacte() {
        // Parité popaul.py : {"participants": [...], "test": false}, rien d'autre.
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/resolve/batch"))
            .and(body_json(serde_json::json!({
                "participants": ["0009:1"],
                "test": false
            })))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"results": []})),
            )
            .mount(&server)
            .await;
        let c = ApiClient::new(&server.uri(), "K", None, None).unwrap();
        assert!(c.resolve_batch(&pids(&["0009:1"])).await.is_ok());
    }

    #[tokio::test]
    async fn connexion_refusee_typee_network() {
        // Port fermé sur 127.0.0.1 : on bind puis on lâche le listener.
        let port = {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        };
        let c = ApiClient::new(&format!("http://127.0.0.1:{port}"), "K", None, None).unwrap();
        match c.resolve_batch(&pids(&["0009:1"])).await {
            Err(ApiError::Network(_)) => {}
            other => panic!("attendu Network, obtenu {other:?}"),
        }
    }

    #[tokio::test]
    async fn url_contenant_407_ne_declenche_pas_proxyauth() {
        // Le message d'erreur reqwest contient l'URL : un port 40777 injoignable
        // ne doit pas être pris pour un 407 proxy.
        let c = ApiClient::new("http://127.0.0.1:40777", "K", None, None).unwrap();
        match c.resolve_batch(&pids(&["0009:1"])).await {
            Err(ApiError::Network(_)) => {}
            other => panic!("attendu Network, obtenu {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_key_ok_sur_resolve_unitaire() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/resolve/0009:552100554"))
            .and(header("X-API-Key", "BONNE_CLE"))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                serde_json::json!({"participant_id": "iso6523-actorid-upis::0009:552100554",
                                   "exists": true}),
            ))
            .mount(&server)
            .await;
        let c = ApiClient::new(&server.uri(), "BONNE_CLE", None, None).unwrap();
        assert!(c.test_key().await.is_ok());
    }
}
