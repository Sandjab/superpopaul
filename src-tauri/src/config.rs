use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub version: u32,
    pub api: ApiConfig,
    pub input: InputConfig,
    pub output: OutputConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApiConfig {
    pub url: String,
    pub key: String,
    /// Backend de résolution : l'API Popaul (batch) ou la résolution
    /// directe SML+SMP. Absent des YAML d'avant cette option → Api,
    /// et non écrit en mode Api (les configs existantes gardent leur forme).
    #[serde(default, skip_serializing_if = "ApiMode::is_api")]
    pub mode: ApiMode,
    /// Résolveur DNS du mode direct. Vide : DNS système. Une IP : DNS
    /// classique (UDP/53) sur ce serveur. Une URL https : DoH (RFC 8484,
    /// pour les réseaux d'entreprise qui bloquent l'UDP/53 — passe par le
    /// proxy). Alias doh_url : nom du champ quand il n'acceptait que le DoH
    /// (les YAML sauvegardés avant restent lisibles).
    #[serde(default, alias = "doh_url", skip_serializing_if = "Option::is_none")]
    pub resolver: Option<String>,
    pub batch_size: u32,
    pub concurrency: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy: Option<ProxyConfig>,
    pub refresh_days: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ApiMode {
    #[default]
    Api,
    Direct,
}

impl ApiMode {
    fn is_api(&self) -> bool {
        *self == ApiMode::Api
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProxyConfig {
    pub url: String,
    /// Identifiants proxy : mémoire uniquement, JAMAIS sérialisés (spec).
    #[serde(skip)]
    pub username: Option<String>,
    #[serde(skip)]
    pub password: Option<String>,
}

/// Debug rédigé : `#[serde(skip)]` ne protège pas des logs `{cfg:?}`,
/// on masque donc les identifiants ici aussi.
impl std::fmt::Debug for ProxyConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProxyConfig")
            .field("url", &self.url)
            .field("username", &self.username.as_ref().map(|_| "***"))
            .field("password", &self.password.as_ref().map(|_| "***"))
            .finish()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputConfig {
    pub path: String,
    /// Décoratifs depuis la spec sortie du 2026-07-12 : l'entrée est toujours
    /// sniffée. Tolérés en lecture (vieux YAML), plus jamais écrits.
    #[serde(default, skip_serializing)]
    pub delimiter: String,
    #[serde(default, skip_serializing)]
    pub encoding: String,
    pub pid_column: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutputConfig {
    pub path: String,
    pub timestamp_suffix: bool,
    /// Les défauts reproduisent le comportement historique (UTF-8+BOM,
    /// séparateur de l'entrée) : un YAML sans ces champs sort à l'identique.
    #[serde(default)]
    pub encoding: OutputEncoding,
    #[serde(default)]
    pub separator: OutputSeparator,
    pub columns: Vec<ColumnSpec>,
}

/// Encodage du fichier de sortie. Le défaut BOM cible Excel FR par
/// double-clic (accents cassés sans lui).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Default)]
pub enum OutputEncoding {
    #[default]
    #[serde(rename = "utf-8-bom")]
    Utf8Bom,
    #[serde(rename = "utf-8")]
    Utf8,
    #[serde(rename = "windows-1252")]
    Windows1252,
}

/// Séparateur du fichier de sortie. `Auto` = celui sniffé sur l'entrée.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Default)]
pub enum OutputSeparator {
    #[default]
    #[serde(rename = "auto")]
    Auto,
    #[serde(rename = ";")]
    Semicolon,
    #[serde(rename = ",")]
    Comma,
    #[serde(rename = "|")]
    Pipe,
    #[serde(rename = "\t")]
    Tab,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "source", rename_all = "lowercase")]
pub enum ColumnSpec {
    Input { name: String },
    Peppol { field: PeppolField },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PeppolField {
    Exists,
    PaCode,
    PaName,
    PaCountry,
    ExtendedCtcFr,
}

impl Config {
    pub fn validate(&self) -> Result<(), String> {
        if !(1..=500).contains(&self.api.batch_size) {
            return Err("batch_size doit être entre 1 et 500".into());
        }
        if self.api.concurrency < 1 {
            return Err("concurrency doit être ≥ 1".into());
        }
        if self.output.columns.is_empty() {
            return Err("output.columns ne doit pas être vide".into());
        }
        Ok(())
    }
}

pub fn to_yaml(cfg: &Config) -> Result<String, String> {
    serde_yaml::to_string(cfg).map_err(|e| e.to_string())
}

pub fn from_yaml(s: &str) -> Result<Config, String> {
    serde_yaml::from_str(s).map_err(|e| e.to_string())
}

pub fn load(path: &Path) -> Result<Config, String> {
    let s = std::fs::read_to_string(path).map_err(|e| format!("lecture {path:?} : {e}"))?;
    let cfg = from_yaml(&s).map_err(|e| format!("config {path:?} : {e}"))?;
    cfg.validate()?;
    Ok(cfg)
}

pub fn save(path: &Path, cfg: &Config) -> Result<(), String> {
    cfg.validate()?;
    // Écriture atomique : fichier temporaire du même répertoire puis rename,
    // pour ne jamais corrompre la config existante en cas de crash.
    let tmp = path.with_extension("yaml.tmp");
    std::fs::write(&tmp, to_yaml(cfg)?).map_err(|e| format!("écriture {tmp:?} : {e}"))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("écriture {path:?} : {e}"))
}

/// Résout un chemin de la config relativement au répertoire du fichier YAML.
pub fn resolve_relative(yaml_path: &Path, p: &str) -> PathBuf {
    let pb = PathBuf::from(p);
    if pb.is_absolute() {
        pb
    } else {
        yaml_path.parent().unwrap_or(Path::new(".")).join(pb)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config_exemple() -> Config {
        Config {
            version: 1,
            api: ApiConfig {
                url: "https://peppol.example.org".into(),
                key: "MA_CLE".into(),
                mode: ApiMode::Api,
                resolver: None,
                batch_size: 50,
                concurrency: 8,
                proxy: Some(ProxyConfig {
                    url: "http://proxy:3128".into(),
                    username: Some("jp".into()),
                    password: Some("SECRET".into()),
                }),
                refresh_days: 30,
            },
            input: InputConfig {
                path: "./clients.csv".into(),
                delimiter: ";".into(),
                encoding: "utf-8".into(),
                pid_column: "siren".into(),
            },
            output: OutputConfig {
                path: "./clients_enrichis.csv".into(),
                timestamp_suffix: true,
                encoding: OutputEncoding::Utf8Bom,
                separator: OutputSeparator::Auto,
                columns: vec![
                    ColumnSpec::Input {
                        name: "siren".into(),
                    },
                    ColumnSpec::Peppol {
                        field: PeppolField::Exists,
                    },
                    ColumnSpec::Peppol {
                        field: PeppolField::PaCode,
                    },
                ],
            },
        }
    }

    #[test]
    fn proxy_creds_never_serialized() {
        // Encode l'intention de sécurité de la spec : le YAML ne doit JAMAIS
        // contenir les identifiants proxy, même s'ils sont en mémoire.
        let yaml = to_yaml(&config_exemple()).unwrap();
        assert!(!yaml.contains("SECRET"));
        assert!(!yaml.contains("username"));
        assert!(!yaml.contains("password"));
        assert!(yaml.contains("http://proxy:3128")); // l'URL, elle, est persistée
    }

    #[test]
    fn round_trip_yaml() {
        let cfg = config_exemple();
        let back = from_yaml(&to_yaml(&cfg).unwrap()).unwrap();
        assert_eq!(back.api.key, "MA_CLE");
        assert_eq!(back.api.batch_size, 50);
        assert_eq!(back.output.columns, cfg.output.columns);
        // Les credentials n'ont pas survécu au round-trip : c'est voulu.
        assert_eq!(back.api.proxy.as_ref().unwrap().username, None);
    }

    #[test]
    fn mode_api_par_defaut_et_direct_en_aller_retour() {
        // Un YAML d'avant le mode direct doit rester lisible : mode absent
        // -> Api, et un YAML en mode Api n'écrit ni mode ni resolver (les
        // configs existantes ne changent pas de forme).
        let yaml = to_yaml(&config_exemple()).unwrap();
        assert!(!yaml.contains("mode:"));
        assert!(!yaml.contains("resolver:"));
        let parsed = from_yaml(&yaml).unwrap();
        assert_eq!(parsed.api.mode, ApiMode::Api);

        let mut cfg = config_exemple();
        cfg.api.mode = ApiMode::Direct;
        cfg.api.resolver = Some("https://1.1.1.1/dns-query".into());
        let parsed = from_yaml(&to_yaml(&cfg).unwrap()).unwrap();
        assert_eq!(parsed.api.mode, ApiMode::Direct);
        assert_eq!(parsed.api.resolver.as_deref(), Some("https://1.1.1.1/dns-query"));
    }

    #[test]
    fn doh_url_des_anciens_yaml_lu_comme_resolver() {
        // Le champ s'appelait doh_url avant de se généraliser (IP ou URL) :
        // un YAML sauvegardé avec l'ancien nom doit continuer à charger.
        let mut cfg = config_exemple();
        cfg.api.resolver = Some("https://1.1.1.1/dns-query".into());
        let ancien = to_yaml(&cfg)
            .unwrap()
            .replace("resolver:", "doh_url:");
        let parsed = from_yaml(&ancien).unwrap();
        assert_eq!(parsed.api.resolver.as_deref(), Some("https://1.1.1.1/dns-query"));
    }

    #[test]
    fn validate_rejette_batch_size_hors_bornes() {
        let mut cfg = config_exemple();
        cfg.api.batch_size = 501;
        assert!(cfg.validate().is_err());
        cfg.api.batch_size = 0;
        assert!(cfg.validate().is_err());
        cfg.api.batch_size = 1;
        assert!(cfg.validate().is_ok());
        // Le plafond suit celui du serveur (/resolve/batch : 500 max).
        cfg.api.batch_size = 500;
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn validate_rejette_colonnes_vides() {
        // L'UI (drop zone, garde « min 1 colonne ») garantit ≥ 1 colonne ; un
        // YAML columns: [] chargerait vers un tableau sans ligne d'en-têtes —
        // aucune cible de drop, utilisateur coincé.
        let mut cfg = config_exemple();
        cfg.output.columns.clear();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn champ_inconnu_rejete() {
        // Un YAML édité à la main avec un champ typo (`usernme:` sous proxy)
        // ne doit pas être avalé silencieusement.
        let yaml = to_yaml(&config_exemple()).unwrap();
        let bad = yaml.replace(
            "url: http://proxy:3128",
            "url: http://proxy:3128\n    usernme: jp",
        );
        assert_ne!(bad, yaml, "l'injection du champ inconnu doit avoir eu lieu");
        assert!(from_yaml(&bad).is_err());
    }

    #[test]
    fn debug_ne_fuit_pas_les_secrets() {
        // #[serde(skip)] ne protège pas des logs `{cfg:?}` : le Debug de
        // ProxyConfig doit masquer les identifiants.
        let proxy = config_exemple().api.proxy.unwrap();
        let dbg = format!("{proxy:?}");
        assert!(!dbg.contains("SECRET"));
        assert!(!dbg.contains("\"jp\""));
        assert!(dbg.contains("***"));
        assert!(dbg.contains("http://proxy:3128")); // l'URL reste visible
    }

    /// YAML « ancien format » : input.delimiter/encoding présents,
    /// output.encoding/separator absents (avant la spec sortie du 2026-07-12).
    fn yaml_ancien() -> &'static str {
        "version: 1\n\
         api:\n  url: https://x\n  key: K\n  batch_size: 50\n  concurrency: 8\n  \
         proxy: null\n  refresh_days: 30\n\
         input:\n  path: ./a.csv\n  delimiter: \";\"\n  encoding: utf-8\n  pid_column: siren\n\
         output:\n  path: ./b.csv\n  timestamp_suffix: true\n  columns:\n    \
         - source: input\n      name: siren\n"
    }

    #[test]
    fn yaml_ancien_charge_avec_defauts_de_sortie() {
        // Compat : les champs input.delimiter/encoding (décoratifs) restent
        // tolérés en lecture, et l'absence d'encoding/separator de sortie
        // donne les défauts = comportement historique (UTF-8+BOM, séparateur
        // de l'entrée). Un vieux YAML produit donc exactement la même sortie.
        let cfg = from_yaml(yaml_ancien()).unwrap();
        assert_eq!(cfg.output.encoding, OutputEncoding::Utf8Bom);
        assert_eq!(cfg.output.separator, OutputSeparator::Auto);
    }

    #[test]
    fn delimiter_encoding_d_entree_plus_jamais_ecrits() {
        // Les champs décoratifs ne doivent plus apparaître dans les nouveaux
        // YAML : seul output porte désormais un encoding (et un separator).
        let yaml = to_yaml(&config_exemple()).unwrap();
        let v: serde_yaml::Value = serde_yaml::from_str(&yaml).unwrap();
        assert!(v["input"].get("delimiter").is_none());
        assert!(v["input"].get("encoding").is_none());
        assert_eq!(v["output"]["encoding"].as_str(), Some("utf-8-bom"));
        assert_eq!(v["output"]["separator"].as_str(), Some("auto"));
    }

    #[test]
    fn encodage_de_sortie_inconnu_rejete() {
        // utf-16 n'est pas supporté : serde doit refuser, pas avaler.
        let bad = to_yaml(&config_exemple())
            .unwrap()
            .replace("encoding: utf-8-bom", "encoding: utf-16");
        assert!(from_yaml(&bad).is_err());
    }

    #[test]
    fn chemins_resolus_relativement_au_yaml() {
        let p = resolve_relative(
            std::path::Path::new("/tmp/projet/conf.yaml"),
            "./clients.csv",
        );
        assert_eq!(p, std::path::PathBuf::from("/tmp/projet/./clients.csv"));
        let abs = resolve_relative(std::path::Path::new("/tmp/projet/conf.yaml"), "/data/x.csv");
        assert_eq!(abs, std::path::PathBuf::from("/data/x.csv"));
    }
}
