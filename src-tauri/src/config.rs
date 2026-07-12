use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub version: u32,
    pub api: ApiConfig,
    pub input: InputConfig,
    pub output: OutputConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiConfig {
    pub url: String,
    pub key: String,
    pub batch_size: u32,
    pub concurrency: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy: Option<ProxyConfig>,
    pub refresh_days: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyConfig {
    pub url: String,
    /// Identifiants proxy : mémoire uniquement, JAMAIS sérialisés (spec).
    #[serde(skip)]
    pub username: Option<String>,
    #[serde(skip)]
    pub password: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputConfig {
    pub path: String,
    pub delimiter: String,
    pub encoding: String,
    pub pid_column: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputConfig {
    pub path: String,
    pub timestamp_suffix: bool,
    pub columns: Vec<ColumnSpec>,
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
        if !(1..=50).contains(&self.api.batch_size) {
            return Err("batch_size doit être entre 1 et 50".into());
        }
        if self.api.concurrency < 1 {
            return Err("concurrency doit être ≥ 1".into());
        }
        if self.input.delimiter.len() != 1 {
            return Err("delimiter doit être un caractère unique".into());
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
    let cfg = from_yaml(&s)?;
    cfg.validate()?;
    Ok(cfg)
}

pub fn save(path: &Path, cfg: &Config) -> Result<(), String> {
    cfg.validate()?;
    std::fs::write(path, to_yaml(cfg)?).map_err(|e| format!("écriture {path:?} : {e}"))
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
        assert_eq!(back.output.columns.len(), 3);
        // Les credentials n'ont pas survécu au round-trip : c'est voulu.
        assert_eq!(back.api.proxy.as_ref().unwrap().username, None);
    }

    #[test]
    fn validate_rejette_batch_size_hors_bornes() {
        let mut cfg = config_exemple();
        cfg.api.batch_size = 51;
        assert!(cfg.validate().is_err());
        cfg.api.batch_size = 0;
        assert!(cfg.validate().is_err());
        cfg.api.batch_size = 1;
        assert!(cfg.validate().is_ok());
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
