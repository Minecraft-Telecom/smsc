use std::{env, net::SocketAddr, path::Path, str::FromStr, time::Duration};

use rusmpp::types::COctetString;
use serde::Deserialize;

const DEFAULT_CONFIG_PATH: &str = "smsc.toml";
const DEFAULT_BIND_ADDR: &str = "0.0.0.0:2775";
const DEFAULT_MAX_PDU_LENGTH: usize = 8192;
const DEFAULT_IDLE_TIMEOUT_SECS: u64 = 300;
const DEFAULT_MAX_CONNECTIONS: usize = 1024;
const DEFAULT_SYSTEM_ID: &str = "smsc";
const DEFAULT_PASSWORD: &str = "password";
const DEFAULT_LOG_FILTER: &str = "smsc=info,rusmpp=info";

#[derive(Debug, Clone)]
pub struct Config {
    pub bind_addr: SocketAddr,
    pub max_pdu_length: usize,
    pub idle_timeout: Duration,
    pub max_connections: usize,
    pub server_system_id: COctetString<1, 16>,
    pub credentials: Vec<Credential>,
    pub log_filter: String,
}

#[derive(Debug, Clone)]
pub struct Credential {
    pub system_id: COctetString<1, 16>,
    pub password: COctetString<1, 9>,
}

impl Credential {
    fn matches(&self, system_id: &str, password: &str) -> bool {
        self.system_id.as_str() == system_id && self.password.as_str() == password
    }
}

#[derive(Debug)]
pub enum ConfigError {
    ReadConfig { path: String, source: std::io::Error },
    ParseConfig { path: String, source: toml::de::Error },
    InvalidBindAddr(String),
    InvalidMaxPduLength(usize),
    InvalidServerSystemId(String),
    MissingCredentials,
    InvalidCredential {
        index: usize,
        field: &'static str,
        value: String,
    },
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::ReadConfig { path, source } => {
                write!(f, "failed to read config file {path}: {source}")
            }
            ConfigError::ParseConfig { path, source } => {
                write!(f, "failed to parse config file {path}: {source}")
            }
            ConfigError::InvalidBindAddr(value) => {
                write!(f, "invalid bind address: {value}")
            }
            ConfigError::InvalidMaxPduLength(value) => {
                write!(f, "max PDU length must be >= 16, got {value}")
            }
            ConfigError::InvalidServerSystemId(value) => {
                write!(f, "invalid server_system_id: {value}")
            }
            ConfigError::MissingCredentials => {
                write!(f, "credentials must contain at least one entry")
            }
            ConfigError::InvalidCredential {
                index,
                field,
                value,
            } => {
                write!(f, "invalid credentials[{index}].{field}: {value}")
            }
        }
    }
}

impl std::error::Error for ConfigError {}

#[derive(Debug, Deserialize)]
struct ConfigFile {
    bind_addr: Option<String>,
    max_pdu_length: Option<usize>,
    idle_timeout_secs: Option<u64>,
    max_connections: Option<usize>,
    server_system_id: Option<String>,
    credentials: Option<Vec<CredentialFile>>,
    log_filter: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CredentialFile {
    system_id: String,
    password: String,
}

impl Config {
    pub fn from_file() -> Result<Self, ConfigError> {
        let path = env::var("SMSC_CONFIG").unwrap_or_else(|_| DEFAULT_CONFIG_PATH.to_string());
        Self::from_path(Path::new(&path))
    }

    pub fn authenticate(&self, system_id: &str, password: &str) -> bool {
        self.credentials
            .iter()
            .any(|credential| credential.matches(system_id, password))
    }

    fn from_path(path: &Path) -> Result<Self, ConfigError> {
        let path_string = path.to_string_lossy().to_string();
        let raw = std::fs::read_to_string(path)
            .map_err(|err| ConfigError::ReadConfig { path: path_string.clone(), source: err })?;
        let file: ConfigFile = toml::from_str(&raw)
            .map_err(|err| ConfigError::ParseConfig { path: path_string, source: err })?;

        let bind_addr_raw = file.bind_addr.unwrap_or_else(|| DEFAULT_BIND_ADDR.to_string());
        let bind_addr = bind_addr_raw
            .parse()
            .map_err(|_| ConfigError::InvalidBindAddr(bind_addr_raw))?;

        let max_pdu_length = file.max_pdu_length.unwrap_or(DEFAULT_MAX_PDU_LENGTH);
        if max_pdu_length < 16 {
            return Err(ConfigError::InvalidMaxPduLength(max_pdu_length));
        }

        let idle_timeout_secs = file.idle_timeout_secs.unwrap_or(DEFAULT_IDLE_TIMEOUT_SECS);
        let idle_timeout = Duration::from_secs(idle_timeout_secs);

        let max_connections = file.max_connections.unwrap_or(DEFAULT_MAX_CONNECTIONS);

        let server_system_id_raw = file
            .server_system_id
            .unwrap_or_else(|| DEFAULT_SYSTEM_ID.to_string());
        let server_system_id = COctetString::from_str(&server_system_id_raw)
            .map_err(|_| ConfigError::InvalidServerSystemId(server_system_id_raw))?;

        let credentials_is_set = file.credentials.is_some();
        let credentials_raw = file.credentials.unwrap_or_default();
        let credentials = if credentials_raw.is_empty() {
            if credentials_is_set {
                return Err(ConfigError::MissingCredentials);
            }
            let system_id = COctetString::from_str(DEFAULT_SYSTEM_ID)
                .map_err(|_| ConfigError::InvalidServerSystemId(DEFAULT_SYSTEM_ID.to_string()))?;
            let password = COctetString::from_str(DEFAULT_PASSWORD)
                .map_err(|_| ConfigError::InvalidCredential {
                    index: 0,
                    field: "password",
                    value: DEFAULT_PASSWORD.to_string(),
                })?;
            vec![Credential { system_id, password }]
        } else {
            let mut parsed = Vec::with_capacity(credentials_raw.len());
            for (index, entry) in credentials_raw.into_iter().enumerate() {
                let system_id = COctetString::from_str(&entry.system_id).map_err(|_| {
                    ConfigError::InvalidCredential {
                        index,
                        field: "system_id",
                        value: entry.system_id.clone(),
                    }
                })?;
                let password = COctetString::from_str(&entry.password).map_err(|_| {
                    ConfigError::InvalidCredential {
                        index,
                        field: "password",
                        value: entry.password.clone(),
                    }
                })?;
                parsed.push(Credential { system_id, password });
            }
            parsed
        };

        if credentials.is_empty() {
            return Err(ConfigError::MissingCredentials);
        }

        let log_filter = file
            .log_filter
            .unwrap_or_else(|| DEFAULT_LOG_FILTER.to_string());

        Ok(Self {
            bind_addr,
            max_pdu_length,
            idle_timeout,
            max_connections,
            server_system_id,
            credentials,
            log_filter,
        })
    }
}
