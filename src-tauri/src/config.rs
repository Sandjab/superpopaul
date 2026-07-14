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
    /// Mode direct : lookups DNS simultanés (indépendant de `concurrency`,
    /// qui pilote les workers). 32 × ~25 ms ≈ 1 250 req/s, sous le
    /// rate-limit des résolveurs publics (~1 500 QPS/IP chez Google) et
    /// autant de sockets UDP en vol au maximum. Absent des YAML d'avant
    /// l'option → 32, et non écrit à la valeur par défaut.
    #[serde(default = "dns_concurrency_default", skip_serializing_if = "dns_concurrency_is_default")]
    pub dns_concurrency: u32,
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

fn dns_concurrency_default() -> u32 {
    32
}

fn dns_concurrency_is_default(v: &u32) -> bool {
    *v == dns_concurrency_default()
}

fn suffix_default() -> String {
    "_enrichi".into()
}

fn suffix_is_default(v: &String) -> bool {
    *v == suffix_default()
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
    /// Répertoire de sortie. Vide : répertoire du fichier d'entrée.
    /// Le nom du fichier est dérivé de l'entrée + `suffix` (le chemin
    /// complet n'est plus saisi depuis la page Réglages).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub dir: String,
    /// Suffixe ajouté au nom du fichier d'entrée (clients.csv →
    /// clients_enrichi.csv). Absent des YAML d'avant l'option → défaut,
    /// et non écrit à la valeur par défaut (comme dns_concurrency).
    #[serde(default = "suffix_default", skip_serializing_if = "suffix_is_default")]
    pub suffix: String,
    /// Legacy : chemin de sortie complet des YAML d'avant la page Réglages.
    /// Toléré en lecture (`from_yaml` le migre en `dir`), plus jamais écrit.
    #[serde(default, skip_serializing)]
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

/// Bornes des paramètres API — partagées entre la config runtime (set_config)
/// et les réglages persistés (superpopaul.yaml), pour ne jamais diverger.
fn validate_api(api: &ApiConfig) -> Result<(), String> {
    if !(1..=500).contains(&api.batch_size) {
        return Err("batch_size doit être entre 1 et 500".into());
    }
    if api.concurrency < 1 {
        return Err("concurrency doit être ≥ 1".into());
    }
    if !(1..=256).contains(&api.dns_concurrency) {
        return Err("dns_concurrency doit être entre 1 et 256".into());
    }
    Ok(())
}

fn validate_suffix(suffix: &str) -> Result<(), String> {
    if suffix.contains(['/', '\\']) {
        return Err("le suffixe de sortie ne doit pas contenir / ou \\".into());
    }
    Ok(())
}

impl Config {
    pub fn validate(&self) -> Result<(), String> {
        // L'absence de colonnes n'est plus bloquante ici : la config est posée
        // (set_config) dès l'ouverture des réglages, avant tout choix de
        // fichier — Tester/Calibrer doivent marcher. La garde vit dans
        // Profile::validate et output::generate.
        validate_api(&self.api)?;
        validate_suffix(&self.output.suffix)
    }
}

// --- Réglages persistants (superpopaul.yaml, dossier données de l'app) --------
// Lus au démarrage, écrits à la fermeture du panneau ⚙ : tout ce qui ne dépend
// pas du fichier traité (API, proxy, forme de la sortie). Jamais les
// identifiants proxy (ProxyConfig les skippe déjà).

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub version: u32,
    pub api: ApiConfig,
    pub output: OutputSettings,
}

/// La partie « forme » d'OutputConfig, sans les colonnes (qui appartiennent
/// au profil) ni le champ legacy `path`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutputSettings {
    /// Vide : répertoire du fichier d'entrée.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub dir: String,
    #[serde(default = "suffix_default", skip_serializing_if = "suffix_is_default")]
    pub suffix: String,
    pub timestamp_suffix: bool,
    #[serde(default)]
    pub encoding: OutputEncoding,
    #[serde(default)]
    pub separator: OutputSeparator,
}

impl Settings {
    pub fn validate(&self) -> Result<(), String> {
        validate_api(&self.api)?;
        validate_suffix(&self.output.suffix)
    }
}

pub fn save_settings_file(path: &Path, s: &Settings) -> Result<(), String> {
    s.validate()?;
    atomic_write(path, &serde_yaml::to_string(s).map_err(|e| e.to_string())?)
}

/// `Ok(None)` si le fichier n'existe pas (premier lancement : défauts UI) ;
/// `Err` s'il existe mais est illisible — à montrer, pas à avaler.
pub fn load_settings_file(path: &Path) -> Result<Option<Settings>, String> {
    let s = match std::fs::read_to_string(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("lecture {path:?} : {e}")),
        Ok(s) => s,
    };
    let settings: Settings =
        serde_yaml::from_str(&s).map_err(|e| format!("réglages {path:?} : {e}"))?;
    settings.validate()?;
    Ok(Some(settings))
}

// --- Profils de chargement (sauvegarde/chargement explicites) -----------------
// Ce qui décrit COMMENT traiter un fichier : le fichier lui-même, la colonne
// des adressages, le mapping des colonnes de sortie. Ni clé API ni réglages.

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub version: u32,
    pub input: ProfileInput,
    pub columns: Vec<ColumnSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileInput {
    pub path: String,
    pub pid_column: String,
}

impl Profile {
    pub fn validate(&self) -> Result<(), String> {
        if self.input.pid_column.is_empty() {
            return Err("le profil doit indiquer la colonne des adressages".into());
        }
        // Un profil columns: [] chargerait vers un tableau sans ligne
        // d'en-têtes — aucune cible de drop, utilisateur coincé.
        if self.columns.is_empty() {
            return Err("le profil doit contenir au moins une colonne de sortie".into());
        }
        Ok(())
    }
}

/// Lit un profil ; le booléen indique un ancien format (config complète
/// d'avant la séparation réglages/profils), dont seuls fichier, colonne
/// d'adressage et colonnes sont repris.
pub fn profile_from_yaml(s: &str) -> Result<(Profile, bool), String> {
    let err_new = match serde_yaml::from_str::<Profile>(s) {
        Ok(p) => {
            p.validate()?;
            return Ok((p, false));
        }
        Err(e) => e,
    };
    if let Ok(cfg) = from_yaml(s) {
        let p = Profile {
            version: 1,
            input: ProfileInput {
                path: cfg.input.path,
                pid_column: cfg.input.pid_column,
            },
            columns: cfg.output.columns,
        };
        p.validate()?;
        return Ok((p, true));
    }
    Err(format!("profil : {err_new}"))
}

pub fn save_profile_file(path: &Path, p: &Profile) -> Result<(), String> {
    p.validate()?;
    atomic_write(path, &serde_yaml::to_string(p).map_err(|e| e.to_string())?)
}

pub fn load_profile_file(path: &Path) -> Result<(Profile, bool), String> {
    let s = std::fs::read_to_string(path).map_err(|e| format!("lecture {path:?} : {e}"))?;
    profile_from_yaml(&s).map_err(|e| format!("{path:?} : {e}"))
}

pub fn to_yaml(cfg: &Config) -> Result<String, String> {
    serde_yaml::to_string(cfg).map_err(|e| e.to_string())
}

pub fn from_yaml(s: &str) -> Result<Config, String> {
    let mut cfg: Config = serde_yaml::from_str(s).map_err(|e| e.to_string())?;
    // Migration des YAML d'avant la page Réglages : output.path (chemin
    // complet) n'en garde que le répertoire — le nom du fichier est désormais
    // dérivé de l'entrée + suffixe. Un path sans répertoire (« b.csv ») laisse
    // dir vide = répertoire du fichier d'entrée.
    if cfg.output.dir.is_empty() && !cfg.output.path.is_empty() {
        if let Some(parent) = Path::new(&cfg.output.path).parent() {
            cfg.output.dir = parent.to_string_lossy().into_owned();
        }
    }
    Ok(cfg)
}

/// Écriture atomique : fichier temporaire du même répertoire puis rename,
/// pour ne jamais corrompre le fichier existant en cas de crash.
fn atomic_write(path: &Path, contents: &str) -> Result<(), String> {
    let tmp = path.with_extension("yaml.tmp");
    std::fs::write(&tmp, contents).map_err(|e| format!("écriture {tmp:?} : {e}"))?;
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
                dns_concurrency: 32,
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
                dir: "./sorties".into(),
                suffix: "_enrichi".into(),
                path: String::new(),
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
    fn dns_concurrency_defaut_32_et_absent_du_yaml_par_defaut() {
        // Un YAML d'avant l'option doit charger (défaut 32), et un YAML à
        // la valeur par défaut ne change pas de forme (comme mode/resolver).
        let yaml = to_yaml(&config_exemple()).unwrap();
        assert!(!yaml.contains("dns_concurrency:"));
        assert_eq!(from_yaml(&yaml).unwrap().api.dns_concurrency, 32);

        let mut cfg = config_exemple();
        cfg.api.dns_concurrency = 16;
        let parsed = from_yaml(&to_yaml(&cfg).unwrap()).unwrap();
        assert_eq!(parsed.api.dns_concurrency, 16);
    }

    #[test]
    fn validate_rejette_dns_concurrency_hors_bornes() {
        let mut cfg = config_exemple();
        cfg.api.dns_concurrency = 0;
        assert!(cfg.validate().is_err());
        cfg.api.dns_concurrency = 257;
        assert!(cfg.validate().is_err());
        cfg.api.dns_concurrency = 256;
        assert!(cfg.validate().is_ok());
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
    fn validate_tolere_colonnes_vides() {
        // La config est posée dès l'ouverture des réglages, avant tout choix
        // de fichier : Tester/Calibrer ne doivent pas buter sur « pas de
        // colonnes ». La garde vit dans Profile::validate et output::generate.
        let mut cfg = config_exemple();
        cfg.output.columns.clear();
        assert!(cfg.validate().is_ok());
    }

    fn settings_exemple() -> Settings {
        let cfg = config_exemple();
        Settings {
            version: 1,
            api: cfg.api,
            output: OutputSettings {
                dir: cfg.output.dir,
                suffix: cfg.output.suffix,
                timestamp_suffix: cfg.output.timestamp_suffix,
                encoding: cfg.output.encoding,
                separator: cfg.output.separator,
            },
        }
    }

    #[test]
    fn settings_fichier_aller_retour_absent_et_corrompu() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("superpopaul.yaml");
        // Absent (premier lancement) : None, pas une erreur.
        assert_eq!(load_settings_file(&p).unwrap().map(|s| s.version), None);
        save_settings_file(&p, &settings_exemple()).unwrap();
        let back = load_settings_file(&p).unwrap().unwrap();
        assert_eq!(back.api.key, "MA_CLE");
        assert_eq!(back.output.suffix, "_enrichi");
        // Corrompu : erreur montrée, pas avalée.
        std::fs::write(&p, "version: [oops").unwrap();
        assert!(load_settings_file(&p).is_err());
    }

    #[test]
    fn settings_ne_serialisent_jamais_les_creds_proxy() {
        // Même intention de sécurité que proxy_creds_never_serialized : le
        // fichier auto-écrit ne doit jamais contenir les identifiants.
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("superpopaul.yaml");
        save_settings_file(&p, &settings_exemple()).unwrap();
        let yaml = std::fs::read_to_string(&p).unwrap();
        assert!(!yaml.contains("SECRET"));
        assert!(!yaml.contains("username"));
        assert!(yaml.contains("http://proxy:3128"));
    }

    fn profile_exemple() -> Profile {
        Profile {
            version: 1,
            input: ProfileInput {
                path: "./clients.csv".into(),
                pid_column: "siren".into(),
            },
            columns: config_exemple().output.columns,
        }
    }

    #[test]
    fn profil_aller_retour_et_champ_inconnu_rejete() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("clients.profil.yaml");
        save_profile_file(&p, &profile_exemple()).unwrap();
        let (back, legacy) = load_profile_file(&p).unwrap();
        assert!(!legacy);
        assert_eq!(back.input.pid_column, "siren");
        assert_eq!(back.columns, profile_exemple().columns);
        // Ni clé API ni réglages dans le fichier.
        let yaml = std::fs::read_to_string(&p).unwrap();
        assert!(!yaml.contains("key"));
        assert!(!yaml.contains("api"));
        // Typo dans un profil neuf : rejet (pas d'aspiration par le fallback
        // legacy, qui exige la forme complète api/input/output).
        let bad = serde_yaml::to_string(&back).unwrap().replace("pid_column:", "pid_colum:");
        assert!(profile_from_yaml(&bad).is_err());
    }

    #[test]
    fn profil_depuis_yaml_ancien_format_complet() {
        // Charger une vieille config complète reste possible : on n'en reprend
        // que le profil (fichier, colonne d'adressage, colonnes) — l'API et la
        // sortie vivent désormais dans superpopaul.yaml.
        let (p, legacy) = profile_from_yaml(yaml_ancien()).unwrap();
        assert!(legacy);
        assert_eq!(p.input.path, "./a.csv");
        assert_eq!(p.input.pid_column, "siren");
        assert_eq!(p.columns.len(), 1);
    }

    #[test]
    fn profil_rejette_colonnes_vides_et_pid_manquant() {
        let mut p = profile_exemple();
        p.columns.clear();
        assert!(p.validate().is_err());
        let mut p = profile_exemple();
        p.input.pid_column.clear();
        assert!(p.validate().is_err());
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
    fn output_path_legacy_migre_en_dir() {
        // Un YAML d'avant la page Réglages porte un chemin de sortie complet :
        // on n'en garde que le répertoire, le nom du fichier étant désormais
        // dérivé de l'entrée + suffixe (défaut _enrichi).
        let cfg = from_yaml(yaml_ancien()).unwrap();
        assert_eq!(cfg.output.dir, ".");
        assert_eq!(cfg.output.suffix, "_enrichi");
    }

    #[test]
    fn output_path_legacy_plus_jamais_ecrit() {
        let yaml = to_yaml(&config_exemple()).unwrap();
        let v: serde_yaml::Value = serde_yaml::from_str(&yaml).unwrap();
        assert!(v["output"].get("path").is_none());
        assert_eq!(v["output"]["dir"].as_str(), Some("./sorties"));
        // Suffixe à la valeur par défaut : non écrit (comme dns_concurrency).
        assert!(v["output"].get("suffix").is_none());
        let mut cfg = config_exemple();
        cfg.output.suffix = "_peppol".into();
        let v: serde_yaml::Value =
            serde_yaml::from_str(&to_yaml(&cfg).unwrap()).unwrap();
        assert_eq!(v["output"]["suffix"].as_str(), Some("_peppol"));
    }

    #[test]
    fn validate_rejette_suffixe_avec_separateur() {
        // Un suffixe « ../x » déplacerait la sortie hors du répertoire choisi.
        let mut cfg = config_exemple();
        cfg.output.suffix = "../x".into();
        assert!(cfg.validate().is_err());
        cfg.output.suffix = "a\\b".into();
        assert!(cfg.validate().is_err());
        cfg.output.suffix = String::new(); // vide : autorisé (date/heure ou autre répertoire)
        assert!(cfg.validate().is_ok());
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
