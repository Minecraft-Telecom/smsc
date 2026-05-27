use std::{
    env,
    net::SocketAddr,
    path::Path,
    str::FromStr,
    time::Duration,
};

use rusmpp::types::COctetString;
use serde::Deserialize;

const DEFAULT_CONFIG_PATH: &str = "smsc.toml";
const DEFAULT_BIND_ADDR: &str = "0.0.0.0:2775";
const DEFAULT_MAX_PDU_LENGTH: usize = 8192;
const DEFAULT_IDLE_TIMEOUT_SECS: u64 = 300;
const DEFAULT_BIND_TIMEOUT_SECS: u64 = 30;
const DEFAULT_DELIVER_RESPONSE_TIMEOUT_SECS: u64 = 30;
const DEFAULT_MAX_CONNECTIONS: usize = 1024;
const DEFAULT_MAX_CONNECTIONS_PER_IP: usize = 32;
const DEFAULT_MAX_BIND_FAILURES: usize = 3;
const DEFAULT_MAX_PENDING_DELIVERIES: usize = 16;
const DEFAULT_BROADCAST_CAPACITY: usize = 1024;
const DEFAULT_SYSTEM_ID: &str = "smsc";
const DEFAULT_LOG_FILTER: &str = "smsc=info,rusmpp=info";
const MIN_MAX_PDU_LENGTH: usize = 256;

fn de_socket_addr<'de, D>(deserializer: D) -> Result<SocketAddr, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    s.parse().map_err(serde::de::Error::custom)
}

fn de_duration_secs<'de, D>(deserializer: D) -> Result<Duration, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let secs = u64::deserialize(deserializer)?;
    if secs == 0 {
        return Err(serde::de::Error::custom("value must be >= 1"));
    }
    Ok(Duration::from_secs(secs))
}

fn de_nonzero_usize<'de, D>(deserializer: D) -> Result<usize, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = usize::deserialize(deserializer)?;
    if value == 0 {
        return Err(serde::de::Error::custom("value must be >= 1"));
    }
    Ok(value)
}

fn de_min_pdu_length<'de, D>(deserializer: D) -> Result<usize, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = usize::deserialize(deserializer)?;
    if value < MIN_MAX_PDU_LENGTH {
        return Err(serde::de::Error::custom(format!(
            "value must be >= {MIN_MAX_PDU_LENGTH}"
        )));
    }
    Ok(value)
}

fn de_server_system_id<'de, D>(deserializer: D) -> Result<COctetString<1, 16>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    COctetString::from_str(&s).map_err(serde::de::Error::custom)
}

fn de_credential_system_id<'de, D>(deserializer: D) -> Result<COctetString<1, 16>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    COctetString::from_str(&s).map_err(serde::de::Error::custom)
}

fn de_credential_password<'de, D>(deserializer: D) -> Result<COctetString<1, 9>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    COctetString::from_str(&s).map_err(serde::de::Error::custom)
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    pub smpp: SmppConfig,
    pub http: HttpConfig,
    pub log_filter: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct SmppConfig {
    #[serde(deserialize_with = "de_socket_addr")]
    pub bind_addr: SocketAddr,

    #[serde(deserialize_with = "de_min_pdu_length")]
    pub max_pdu_length: usize,

    #[serde(deserialize_with = "de_duration_secs")]
    pub idle_timeout: Duration,

    #[serde(deserialize_with = "de_duration_secs")]
    pub bind_timeout: Duration,

    #[serde(deserialize_with = "de_duration_secs")]
    pub deliver_response_timeout: Duration,

    #[serde(deserialize_with = "de_nonzero_usize")]
    pub max_connections: usize,

    #[serde(deserialize_with = "de_nonzero_usize")]
    pub max_connections_per_ip: usize,

    #[serde(deserialize_with = "de_nonzero_usize")]
    pub max_bind_failures: usize,

    #[serde(deserialize_with = "de_nonzero_usize")]
    pub max_pending_deliveries: usize,

    #[serde(deserialize_with = "de_nonzero_usize")]
    pub queue_broadcast_capacity: usize,

    #[serde(deserialize_with = "de_server_system_id")]
    pub server_system_id: COctetString<1, 16>,

    pub credentials: Vec<Credential>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct HttpConfig {
    #[serde(deserialize_with = "de_socket_addr")]
    pub bind_addr: SocketAddr,
}

impl Default for SmppConfig {
    fn default() -> Self {
        Self {
            bind_addr: DEFAULT_BIND_ADDR.parse().expect("valid default bind addr"),
            max_pdu_length: DEFAULT_MAX_PDU_LENGTH,
            idle_timeout: Duration::from_secs(DEFAULT_IDLE_TIMEOUT_SECS),
            bind_timeout: Duration::from_secs(DEFAULT_BIND_TIMEOUT_SECS),
            deliver_response_timeout: Duration::from_secs(DEFAULT_DELIVER_RESPONSE_TIMEOUT_SECS),
            max_connections: DEFAULT_MAX_CONNECTIONS,
            max_connections_per_ip: DEFAULT_MAX_CONNECTIONS_PER_IP,
            max_bind_failures: DEFAULT_MAX_BIND_FAILURES,
            max_pending_deliveries: DEFAULT_MAX_PENDING_DELIVERIES,
            queue_broadcast_capacity: DEFAULT_BROADCAST_CAPACITY,
            server_system_id: COctetString::from_str(DEFAULT_SYSTEM_ID)
                .expect("valid default system id"),
            credentials: Vec::new(),
        }
    }
}

impl Default for HttpConfig {
    fn default() -> Self {
        Self {
            bind_addr: "0.0.0.0:8080".parse().expect("valid config"),
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            smpp: SmppConfig::default(),
            http: HttpConfig::default(),
            log_filter: DEFAULT_LOG_FILTER.to_string(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Credential {
    #[serde(deserialize_with = "de_credential_system_id")]
    pub system_id: COctetString<1, 16>,

    #[serde(deserialize_with = "de_credential_password")]
    pub password: COctetString<1, 9>,
}

impl Credential {
    fn matches(&self, system_id: &str, password: &str) -> bool {
        self.system_id.as_str() == system_id && self.password.as_str() == password
    }
}

impl Config {
    pub fn from_file() -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let path = env::var("SMSC_CONFIG").unwrap_or_else(|_| DEFAULT_CONFIG_PATH.to_string());
        Self::from_path(Path::new(&path))
    }

    pub fn authenticate(&self, system_id: &str, password: &str) -> bool {
        self.smpp.credentials
            .iter()
            .any(|credential| credential.matches(system_id, password))
    }

    fn from_path(path: &Path) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let raw = std::fs::read_to_string(path)?;
        let cfg: Config = toml::from_str(&raw)?;
        Ok(cfg.validate()?)
    }

    fn validate(self) -> Result<Self, std::io::Error> {
        if self.smpp.credentials.is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "credentials must contain at least one entry",
            ));
        }

        Ok(self)
    }
}