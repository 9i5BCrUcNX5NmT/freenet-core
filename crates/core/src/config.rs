use std::{
    convert::TryFrom,
    fs::{self, File},
    future::Future,
    io::Read,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::PathBuf,
    pin::Pin,
    str::FromStr,
    sync::atomic::AtomicBool,
    time::Duration,
};

use directories::ProjectDirs;
use libp2p::{identity, PeerId};
use once_cell::sync::Lazy;
use tokio::runtime::Runtime;

use crate::local_node::OperationMode;

/// Default maximum number of connections for the peer.
pub const DEFAULT_MAX_CONNECTIONS: usize = 20;
/// Default minimum number of connections for the peer.
pub const DEFAULT_MIN_CONNECTIONS: usize = 10;
/// Default threshold for randomizing potential peers for new connections.
///
/// If the hops left for the operation is above or equal to this threshold
/// (of the total DEFAULT_MAX_HOPS_TO_LIVE), then the next potential peer
/// will be selected randomly. Otherwise the optimal peer will be selected
/// by Freenet custom algorithms.
pub const DEFAULT_RANDOM_PEER_CONN_THRESHOLD: usize = 7;
/// Default maximum number of hops to live for any operation
/// (if it applies, e.g. connect requests).
pub const DEFAULT_MAX_HOPS_TO_LIVE: usize = 10;
const DEFAULT_BOOTSTRAP_PORT: u16 = 7800;
const DEFAULT_WEBSOCKET_API_PORT: u16 = 55008;

static CONFIG: std::sync::OnceLock<Config> = std::sync::OnceLock::new();

pub(crate) const PEER_TIMEOUT: Duration = Duration::from_secs(60);
pub(crate) const OPERATION_TTL: Duration = Duration::from_secs(60);

// Initialize the executor once.
static ASYNC_RT: Lazy<Option<Runtime>> = Lazy::new(GlobalExecutor::initialize_async_rt);

const QUALIFIER: &str = "";
const ORGANIZATION: &str = "The Freenet Project Inc";
const APPLICATION: &str = "Freenet";

pub struct Config {
    pub bootstrap_ip: IpAddr,
    pub bootstrap_port: u16,
    pub bootstrap_id: Option<PeerId>,
    pub local_peer_keypair: identity::Keypair,
    pub log_level: tracing::log::LevelFilter,
    config_paths: ConfigPaths,
    local_mode: AtomicBool,

    #[cfg(feature = "websocket")]
    #[allow(unused)]
    pub(crate) ws: WebSocketApiConfig,
}

#[cfg(feature = "websocket")]
#[derive(Debug, Copy, Clone)]
pub(crate) struct WebSocketApiConfig {
    ip: IpAddr,
    port: u16,
}

#[cfg(feature = "websocket")]
impl From<WebSocketApiConfig> for SocketAddr {
    fn from(val: WebSocketApiConfig) -> Self {
        (val.ip, val.port).into()
    }
}

#[cfg(feature = "websocket")]
impl WebSocketApiConfig {
    fn from_config(config: &config::Config) -> Self {
        WebSocketApiConfig {
            ip: IpAddr::from_str(
                &config
                    .get_string("websocket_api_ip")
                    .unwrap_or_else(|_| format!("{}", Ipv4Addr::LOCALHOST)),
            )
            .map_err(|_err| std::io::ErrorKind::InvalidInput)
            .unwrap(),
            port: config
                .get_int("websocket_api_port")
                .map(u16::try_from)
                .unwrap_or(Ok(DEFAULT_WEBSOCKET_API_PORT))
                .map_err(|_err| std::io::ErrorKind::InvalidInput)
                .unwrap(),
        }
    }
}

#[derive(Debug)]
pub struct ConfigPaths {
    contracts_dir: PathBuf,
    delegates_dir: PathBuf,
    secrets_dir: PathBuf,
    db_dir: PathBuf,
    event_log: PathBuf,
}

impl ConfigPaths {
    pub fn app_data_dir() -> std::io::Result<PathBuf> {
        let project_dir = ProjectDirs::from(QUALIFIER, ORGANIZATION, APPLICATION)
            .ok_or(std::io::ErrorKind::NotFound)?;
        let app_data_dir: PathBuf = if cfg!(any(test, debug_assertions)) {
            std::env::temp_dir().join("freenet")
        } else {
            project_dir.data_dir().into()
        };
        Ok(app_data_dir)
    }

    fn new(data_dir: Option<PathBuf>) -> std::io::Result<ConfigPaths> {
        let app_data_dir = data_dir.map(Ok).unwrap_or_else(Self::app_data_dir)?;
        let contracts_dir = app_data_dir.join("contracts");
        let delegates_dir = app_data_dir.join("delegates");
        let secrets_dir = app_data_dir.join("secrets");
        let db_dir = app_data_dir.join("db");

        if !contracts_dir.exists() {
            fs::create_dir_all(&contracts_dir)?;
            fs::create_dir_all(contracts_dir.join("local"))?;
        }

        if !delegates_dir.exists() {
            fs::create_dir_all(&delegates_dir)?;
            fs::create_dir_all(delegates_dir.join("local"))?;
        }

        if !secrets_dir.exists() {
            fs::create_dir_all(&secrets_dir)?;
            fs::create_dir_all(secrets_dir.join("local"))?;
        }

        if !db_dir.exists() {
            fs::create_dir_all(&db_dir)?;
            fs::create_dir_all(db_dir.join("local"))?;
        }

        let event_log = app_data_dir.join("_EVENT_LOG");
        if !event_log.exists() {
            fs::write(&event_log, [])?;
            let mut local_file = event_log.clone();
            local_file.set_file_name("_EVENT_LOG_LOCAL");
            fs::write(local_file, [])?;
        }

        Ok(Self {
            contracts_dir,
            delegates_dir,
            secrets_dir,
            db_dir,
            event_log,
        })
    }
}

impl Config {
    pub fn set_op_mode(mode: OperationMode) {
        let local_mode = matches!(mode, OperationMode::Local);
        Self::conf()
            .local_mode
            .store(local_mode, std::sync::atomic::Ordering::SeqCst);
    }

    pub fn db_dir(&self) -> PathBuf {
        if self.local_mode.load(std::sync::atomic::Ordering::SeqCst) {
            self.config_paths.db_dir.join("local")
        } else {
            self.config_paths.db_dir.to_owned()
        }
    }

    pub fn contracts_dir(&self) -> PathBuf {
        if self.local_mode.load(std::sync::atomic::Ordering::SeqCst) {
            self.config_paths.contracts_dir.join("local")
        } else {
            self.config_paths.contracts_dir.to_owned()
        }
    }

    pub fn delegates_dir(&self) -> PathBuf {
        if self.local_mode.load(std::sync::atomic::Ordering::SeqCst) {
            self.config_paths.delegates_dir.join("local")
        } else {
            self.config_paths.delegates_dir.to_owned()
        }
    }

    pub fn secrets_dir(&self) -> PathBuf {
        if self.local_mode.load(std::sync::atomic::Ordering::SeqCst) {
            self.config_paths.secrets_dir.join("local")
        } else {
            self.config_paths.delegates_dir.to_owned()
        }
    }

    pub fn event_log(&self) -> PathBuf {
        if self.local_mode.load(std::sync::atomic::Ordering::SeqCst) {
            let mut local_file = self.config_paths.event_log.clone();
            local_file.set_file_name("_EVENT_LOG_LOCAL");
            local_file
        } else {
            self.config_paths.event_log.to_owned()
        }
    }

    pub fn conf() -> &'static Config {
        CONFIG.get_or_init(|| match Config::load_conf() {
            Ok(config) => config,
            Err(err) => {
                tracing::error!("failed while loading configuration: {err}");
                panic!("Failed while loading configuration")
            }
        })
    }

    fn load_conf() -> std::io::Result<Config> {
        let settings: config::Config = config::Config::builder()
            .add_source(config::Environment::with_prefix("FREENET"))
            .build()
            .unwrap();
        let local_peer_keypair = if let Ok(path_to_key) = settings
            .get_string("local_peer_key_file")
            .map(PathBuf::from)
        {
            let mut key_file = File::open(&path_to_key).unwrap_or_else(|_| {
                panic!(
                    "Failed to open key file: {}",
                    &path_to_key.to_str().unwrap()
                )
            });
            let mut buf = Vec::new();
            key_file.read_to_end(&mut buf).unwrap();
            Some(
                identity::Keypair::from_protobuf_encoding(&buf)
                    .map_err(|_| std::io::ErrorKind::InvalidData)?,
            )
        } else {
            None
        };
        let log_level = settings
            .get_string("log")
            .map(|lvl| lvl.parse().ok())
            .ok()
            .flatten()
            .unwrap_or(tracing::log::LevelFilter::Info);
        let (bootstrap_ip, bootstrap_port, bootstrap_id) = Config::get_bootstrap_host(&settings)?;

        let data_dir = settings.get_string("data_dir").ok().map(PathBuf::from);
        let config_paths = ConfigPaths::new(data_dir)?;

        let local_mode = settings.get_string("network_mode").is_err();

        Ok(Config {
            bootstrap_ip,
            bootstrap_port,
            bootstrap_id,
            local_peer_keypair: local_peer_keypair
                .unwrap_or_else(identity::Keypair::generate_ed25519),
            log_level,
            config_paths,
            local_mode: AtomicBool::new(local_mode),
            #[cfg(feature = "websocket")]
            ws: WebSocketApiConfig::from_config(&settings),
        })
    }

    fn get_bootstrap_host(
        settings: &config::Config,
    ) -> std::io::Result<(IpAddr, u16, Option<PeerId>)> {
        let bootstrap_ip = IpAddr::from_str(
            &settings
                .get_string("bootstrap_host")
                .unwrap_or_else(|_| format!("{}", Ipv4Addr::LOCALHOST)),
        )
        .map_err(|_err| std::io::ErrorKind::InvalidInput)?;

        let bootstrap_port = settings
            .get_int("bootstrap_port")
            .ok()
            .map(u16::try_from)
            .unwrap_or(Ok(DEFAULT_BOOTSTRAP_PORT))
            .map_err(|_err| std::io::ErrorKind::InvalidInput)?;

        let id_str = if let Some(id_str) = settings
            .get_string("bootstrap_id")
            .ok()
            .map(|id| id.parse().map_err(|_err| std::io::ErrorKind::InvalidInput))
        {
            Some(id_str?)
        } else {
            None
        };

        Ok((bootstrap_ip, bootstrap_port, id_str))
    }
}

pub(crate) struct GlobalExecutor;

impl GlobalExecutor {
    /// Returns the runtime handle if it was initialized or none if it was already
    /// running on the background.
    pub(crate) fn initialize_async_rt() -> Option<Runtime> {
        if tokio::runtime::Handle::try_current().is_ok() {
            None
        } else {
            let mut builder = tokio::runtime::Builder::new_multi_thread();
            builder.enable_all().thread_name("freenet-node");
            if cfg!(debug_assertions) {
                builder.worker_threads(2).max_blocking_threads(2);
            }
            Some(builder.build().expect("failed to build tokio runtime"))
        }
    }

    #[inline]
    pub fn spawn<R: Send + 'static>(
        f: impl Future<Output = R> + Send + 'static,
    ) -> tokio::task::JoinHandle<R> {
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(f)
        } else if let Some(rt) = &*ASYNC_RT {
            rt.spawn(f)
        } else {
            unreachable!("the executor must have been initialized")
        }
    }
}

impl libp2p::swarm::Executor for GlobalExecutor {
    fn exec(&self, future: Pin<Box<dyn Future<Output = ()> + 'static + Send>>) {
        GlobalExecutor::spawn(future);
    }
}

pub fn set_logger() {
    #[cfg(feature = "trace")]
    {
        static LOGGER_SET: AtomicBool = AtomicBool::new(false);
        if LOGGER_SET
            .compare_exchange(
                false,
                true,
                std::sync::atomic::Ordering::Release,
                std::sync::atomic::Ordering::SeqCst,
            )
            .is_err()
        {
            return;
        }

        crate::tracing::tracer::init_tracer().expect("failed tracing initialization")
    }
}
