use app_dirs2::*;
use failure::Fallible;
use preferences::{Preferences, PreferencesMap};
use std::{
    fs::{File, OpenOptions},
    io::{BufReader, BufWriter, Write},
    path::PathBuf,
};
use structopt::StructOpt;

pub const APP_INFO: AppInfo = AppInfo {
    name:   "ConcordiumP2P",
    author: "Concordium",
};

// a list of peer client versions applicable for connections; it doesn't
// contain CARGO_PKG_VERSION (or any other dynamic components) so that
// it is impossible to omit manual inspection upon future updates
pub const COMPATIBLE_CLIENT_VERSIONS: [&str; 2] = ["0.2.1", "0.2.0"];

const APP_PREFERENCES_MAIN: &str = "main.config";
pub const APP_PREFERENCES_KEY_VERSION: &str = "VERSION";
pub const APP_PREFERENCES_PERSISTED_NODE_ID: &str = "PERSISTED_NODE_ID";

// maximum time allowed for a peer to catch up with in milliseconds
pub const MAX_CATCH_UP_TIME: u64 = 300_000;

// queue depths
pub const EVENT_LOG_QUEUE_DEPTH: usize = 100;
pub const DUMP_QUEUE_DEPTH: usize = 100;
pub const DUMP_SWITCH_QUEUE_DEPTH: usize = 0;

// connection-related consts
pub const MAX_FAILED_PACKETS_ALLOWED: u32 = 50;
pub const UNREACHABLE_EXPIRATION_SECS: u64 = 86_400;
pub const MAX_BOOTSTRAPPER_KEEP_ALIVE: u64 = 300_000;
pub const MAX_NORMAL_KEEP_ALIVE: u64 = 1_200_000;
pub const MAX_PREHANDSHAKE_KEEP_ALIVE: u64 = 120_000;
pub const SOFT_BAN_DURATION_SECS: u64 = 300;

#[cfg(feature = "instrumentation")]
#[derive(StructOpt, Debug)]
/// Flags related to Prometheus
pub struct PrometheusConfig {
    #[structopt(
        long = "prometheus-listen-addr",
        help = "IP to listen for prometheus requests on",
        default_value = "127.0.0.1"
    )]
    pub prometheus_listen_addr: String,
    #[structopt(
        long = "prometheus-listen-port",
        help = "Port for prometheus to listen on",
        default_value = "9090"
    )]
    pub prometheus_listen_port: u16,
    #[structopt(long = "prometheus-server", help = "Enable prometheus server for metrics")]
    pub prometheus_server: bool,
    #[structopt(long = "prometheus-push-gateway", help = "Enable prometheus via push gateway")]
    pub prometheus_push_gateway: Option<String>,
    #[structopt(
        long = "prometheus-job-name",
        help = "Job name to send to push gateway",
        default_value = "p2p_node_push"
    )]
    pub prometheus_job_name: String,
    #[structopt(long = "prometheus-instance-name", help = "If not present node_id will be used")]
    pub prometheus_instance_name: Option<String>,
    #[structopt(
        long = "prometheus-push-gateway-username",
        help = "Username to use for push gateway, if either username or password is omitted \
                authentication isn't used"
    )]
    pub prometheus_push_username: Option<String>,
    #[structopt(
        long = "prometheus-push-gateway-password",
        help = "Password to use for push gateway, if either username or password is omitted \
                authentication isn't used"
    )]
    pub prometheus_push_password: Option<String>,
    #[structopt(
        long = "prometheus-push-gateway-interval",
        help = "Interval in seconds between pushes",
        default_value = "2"
    )]
    pub prometheus_push_interval: u64,
}

#[cfg(feature = "benchmark")]
#[derive(StructOpt, Debug)]
/// Flags related to TPS (only used in Cli)
pub struct TpsConfig {
    #[structopt(long = "enable-tps-test-recv", help = "Enable TPS test recv")]
    pub enable_tps_test: bool,
    #[structopt(long = "tps-test-recv-id", help = "Receiver of TPS test")]
    pub tps_test_recv_id: Option<String>,
    #[structopt(
        long = "tps-stats-save-amount",
        help = "Amount of stats to save for TPS statistics",
        default_value = "10000"
    )]
    pub tps_stats_save_amount: u64,
    #[structopt(
        long = "tps-message-count",
        help = "Amount of messages to be sent and received",
        default_value = "1000"
    )]
    pub tps_message_count: u64,
}

#[derive(StructOpt, Debug)]
/// Flags related to Baking (only used in Cli)
pub struct BakerConfig {
    #[structopt(long = "baker-id", help = "Baker ID")]
    pub baker_id: Option<u64>,
    #[cfg(feature = "profiling")]
    #[structopt(
        long = "heap-profiling",
        help = "Profile the heap [(`cost`,-hc), (`type`, -hy), (`module`, -hm), (`description`, \
                -hd)] in the Haskell subsystem",
        default_value = "none"
    )]
    pub heap_profiling: String,
    #[cfg(feature = "profiling")]
    #[structopt(long = "time-profiling", help = "Profile the time in the Haskell subsystem")]
    pub time_profiling: bool,
    #[cfg(feature = "profiling")]
    #[structopt(
        long = "backtraces",
        help = "Show bactraces generated by exceptions in the Haskell subsystem"
    )]
    pub backtraces_profiling: bool,
    #[cfg(feature = "profiling")]
    #[structopt(
        long = "profiling-sampling-interval",
        help = "Profile sampling interval in seconds",
        default_value = "0.1"
    )]
    pub profiling_sampling_interval: String,
    #[structopt(long = "haskell-gc-logging", help = "Enable Haskell garbage collection logging")]
    pub gc_logging: Option<String>,
    #[structopt(long = "persist-global-state", help = "Persist the the global state store")]
    pub persist_global_state: bool,
    #[structopt(
        long = "maximum-block-size",
        help = "Maximum block size in bytes",
        default_value = "12582912"
    )]
    pub maximum_block_size: u32,
    #[structopt(
        long = "scheduler-outcome-logging",
        help = "Enable outcome of finalized baked blocks from the scheduler"
    )]
    pub scheduler_outcome_logging: bool,
}

#[derive(StructOpt, Debug)]
/// Flags related to the RPC (onl`y used in Cli)
pub struct RpcCliConfig {
    #[structopt(long = "no-rpc-server", help = "Disable the built-in RPC server")]
    pub no_rpc_server: bool,
    #[structopt(long = "rpc-server-port", help = "RPC server port", default_value = "10000")]
    pub rpc_server_port: u16,
    #[structopt(
        long = "rpc-server-addr",
        help = "RPC server listen address",
        default_value = "127.0.0.1"
    )]
    pub rpc_server_addr: String,
    #[structopt(
        long = "rpc-server-token",
        help = "RPC server access token",
        default_value = "rpcadmin"
    )]
    pub rpc_server_token: String,
}

#[derive(StructOpt, Debug)]
/// Flags related to connection matters
pub struct ConnectionConfig {
    #[structopt(
        long = "desired-nodes",
        help = "Desired nodes to always have",
        default_value = "7"
    )]
    pub desired_nodes: u16,
    #[structopt(
        long = "max-resend-attempts",
        help = "Maximum number of times a packet is attempted to be resent",
        default_value = "5"
    )]
    pub max_resend_attempts: u8,
    #[structopt(long = "max-allowed-nodes", help = "Maximum nodes to allow a connection to")]
    pub max_allowed_nodes: Option<u16>,
    #[structopt(
        long = "max-allowed-nodes-percentage",
        help = "Maximum nodes to allow a connection to is set as a percentage of desired-nodes \
                (minimum 100, to set it to desired-nodes",
        default_value = "150"
    )]
    pub max_allowed_nodes_percentage: u16,
    #[structopt(long = "no-bootstrap", help = "Do not bootstrap via DNS")]
    pub no_bootstrap_dns: bool,
    #[structopt(
        long = "relay-broadcast-percentage",
        help = "The percentage of peers to relay broadcasted messages to",
        default_value = "1.0"
    )]
    pub relay_broadcast_percentage: f64,
    #[structopt(
        long = "bootstrap-server",
        help = "DNS name to resolve bootstrap nodes from",
        default_value = "bootstrap.p2p.concordium.com"
    )]
    pub bootstrap_server: String,
    #[structopt(
        long = "global-state-catch-up-requests",
        help = "Should global state produce catch-up requests"
    )]
    pub global_state_catch_up_requests: bool,
    #[structopt(
        long = "connect-to",
        short = "c",
        help = "Peer to connect to upon startup (host/ip:port)"
    )]
    pub connect_to: Vec<String>,
    #[structopt(
        long = "no-dnssec",
        help = "Do not perform DNSsec tests for lookups. If flag is set, then no DNSSEC \
                validation will be performed"
    )]
    pub dnssec_disabled: bool,
    #[structopt(long = "dns-resolver", help = "DNS resolver to use")]
    pub dns_resolver: Vec<String>,
    #[structopt(
        long = "bootstrap-node",
        help = "Bootstrap nodes to use upon startup host/ip:port (this disables DNS bootstrapping)"
    )]
    pub bootstrap_nodes: Vec<String>,
    #[structopt(
        long = "resolv-conf",
        help = "Location of resolv.conf",
        default_value = "/etc/resolv.conf"
    )]
    pub resolv_conf: String,
    #[structopt(
        long = "housekeeping-interval",
        help = "The connection housekeeping interval in seconds",
        default_value = "60"
    )]
    pub housekeeping_interval: u64,
    #[structopt(
        long = "bootstrapping-interval",
        help = "The bootstrapping interval in seconds",
        default_value = "7200"
    )]
    pub bootstrapping_interval: u64,
    #[structopt(long = "max-latency", help = "The maximum allowed connection latency in ms")]
    pub max_latency: Option<u64>,
    #[structopt(
        long = "hard-connection-limit",
        help = "Maximum connections to keep open at any time"
    )]
    pub hard_connection_limit: Option<u16>,
    #[structopt(
        long = "catch-up-batch-limit",
        help = "The maximum batch size for a catch-up round (0 = no limit)",
        default_value = "50"
    )]
    pub catch_up_batch_limit: u64,
    #[structopt(
        long = "thread-pool-size",
        help = "The size of the threadpool processing connection events in parallel",
        default_value = "4"
    )]
    pub thread_pool_size: usize,
    #[structopt(
        long = "dedup-size-long",
        help = "The size of the long deduplication queues",
        default_value = "65536"
    )]
    pub dedup_size_long: usize,
    #[structopt(
        long = "dedup-size-short",
        help = "The size of the short deduplication queues",
        default_value = "4096"
    )]
    pub dedup_size_short: usize,
    #[structopt(
        long = "socket-write-size",
        help = "The desired size of single socket writes; must be no bigger than socket_read_size",
        default_value = "16384"
    )]
    pub socket_write_size: usize,
    #[structopt(
        long = "socket-read-size",
        help = "The desired size of single socket reads; must be >= 65535 (max noise message size)",
        default_value = "131072"
    )]
    pub socket_read_size: usize,
}

#[derive(StructOpt, Debug)]
/// Common configuration for the three modes
pub struct CommonConfig {
    #[structopt(long = "external-ip", help = "Own external IP")]
    pub external_ip: Option<String>,
    #[structopt(long = "external-port", help = "Own external port")]
    pub external_port: Option<u16>,
    #[structopt(
        long = "id",
        short = "i",
        help = "Set forced node id (64 bit unsigned integer in zero padded HEX. Must be 16 \
                characters long)"
    )]
    pub id: Option<String>,
    #[structopt(
        long = "listen-port",
        short = "p",
        help = "Port to listen on",
        default_value = "8888"
    )]
    pub listen_port: u16,
    #[structopt(long = "listen-address", short = "l", help = "Address to listen on")]
    pub listen_address: Option<String>,
    #[structopt(long = "debug", short = "d", help = "Debug mode")]
    pub debug: bool,
    #[structopt(long = "trace", help = "Trace mode")]
    pub trace: bool,
    #[structopt(long = "info", help = "Info mode")]
    pub info: bool,
    #[structopt(
        long = "network-id",
        short = "n",
        help = "Enable network id",
        default_value = "1000"
    )]
    pub network_ids: Vec<u16>,
    #[structopt(long = "override-config-dir", help = "Override location of configuration files")]
    pub config_dir: Option<String>,
    #[structopt(long = "override-data-dir", help = "Override location of data files")]
    pub data_dir: Option<String>,
    #[structopt(long = "no-log-timestamp", help = "Do not output timestamp in log output")]
    pub no_log_timestamp: bool,
    #[structopt(long = "no-trust-bans", help = "Don't blindly trust ban/unban requests")]
    pub no_trust_bans: bool,
    #[structopt(
        long = "minimum-peers-bucket",
        help = "Minimum peers to keep in each bucket always",
        default_value = "100"
    )]
    pub min_peers_bucket: usize,
    #[structopt(long = "print-config", help = "Print out config struct")]
    pub print_config: bool,
    #[structopt(
        long = "bucket-cleanup-interval",
        help = "Try to timeout entries in the buckets every set interval (in ms)",
        default_value = "600000"
    )]
    pub bucket_cleanup_interval: u64,
}

#[derive(StructOpt, Debug)]
pub struct CliConfig {
    #[structopt(long = "no-network", help = "Disable network")]
    pub no_network: bool,
    #[structopt(
        long = "poll-interval",
        help = "The polling interval in milliseconds",
        default_value = "100"
    )]
    pub poll_interval: u64,
    #[structopt(flatten)]
    pub baker: BakerConfig,
    #[cfg(feature = "benchmark")]
    #[structopt(flatten)]
    pub tps: TpsConfig,
    #[structopt(flatten)]
    pub rpc: RpcCliConfig,
    #[cfg(feature = "elastic_logging")]
    #[structopt(long = "elastic-logging", help = "Enable logging to Elastic Search")]
    pub elastic_logging_enabled: bool,
    #[cfg(feature = "elastic_logging")]
    #[structopt(
        long = "elastic-logging-url",
        help = "URL to use for logging to Elastic Search",
        default_value = "http://127.0.0.1:9200"
    )]
    pub elastic_logging_url: String,
    #[cfg(feature = "beta")]
    #[structopt(long = "beta-token", help = "Beta client token")]
    pub beta_token: String,
    #[structopt(
        long = "timeout-bucket-entry-period",
        help = "Timeout an entry in the buckets after a given period (in ms), 0 means never",
        default_value = "0"
    )]
    pub timeout_bucket_entry_period: u64,
    #[structopt(
        long = "no-rebroadcast-consensus-validation",
        help = "Disable consensus controlling whether to rebroadcast or not"
    )]
    pub no_rebroadcast_consensus_validation: bool,
    #[structopt(
        long = "drop-rebroadcast-probability",
        help = "Drop a message from being rebroadcasted by a certain probability"
    )]
    pub drop_rebroadcast_probability: Option<f64>,
    #[structopt(
        long = "breakage-type",
        help = "Break for test purposes; spam - send duplicate messages / fuzz - mangle messages \
                [fuzz|spam]"
    )]
    pub breakage_type: Option<String>,
    #[structopt(
        long = "breakage-target",
        help = "Used together with breakage-type; 0/1/2/3/4/99 - blocks/txs/fin msgs/fin \
                recs/catch-up msgs/everything [0|1|2|3|4|99]"
    )]
    pub breakage_target: Option<u8>,
    #[structopt(
        long = "breakage-level",
        help = "Used together with breakage-type; either the number of spammed duplicates or \
                mangled bytes"
    )]
    pub breakage_level: Option<usize>,
}

#[derive(StructOpt, Debug)]
#[structopt(name = "BootstrapperNode")]
pub struct BootstrapperConfig {
    #[structopt(
        long = "max-nodes",
        help = "Max nodes allowed to connect",
        default_value = "10000"
    )]
    pub max_nodes: u16,
    #[structopt(
        long = "wait-until-minimum-nodes",
        help = "Wait until a minumum number of nodes have been obtained before sending out peer \
                lists to peers",
        default_value = "0"
    )]
    pub wait_until_minimum_nodes: u16,
    #[structopt(
        long = "bootstrapper-timeout-bucket-entry-period",
        help = "Timeout an entry in the buckets after a given period (in ms), 0 means never",
        default_value = "7200000"
    )]
    pub bootstrapper_timeout_bucket_entry_period: u64,
    #[structopt(
        long = "partition-network-for-time",
        help = "Partition the network for a set amount of time since startup (in ms)"
    )]
    pub partition_network_for_time: Option<usize>,
}

#[derive(StructOpt, Debug)]
pub struct Config {
    #[structopt(flatten)]
    pub common: CommonConfig,
    #[cfg(feature = "instrumentation")]
    #[structopt(flatten)]
    pub prometheus: PrometheusConfig,
    #[structopt(flatten)]
    pub connection: ConnectionConfig,
    #[structopt(flatten)]
    pub cli: CliConfig,
    #[structopt(flatten)]
    pub bootstrapper: BootstrapperConfig,
}

impl Config {
    pub fn add_options(
        mut self,
        listen_address: Option<String>,
        listen_port: u16,
        network_ids: Vec<u16>,
        min_peers_bucket: usize,
    ) -> Self {
        self.common.listen_address = listen_address;
        self.common.listen_port = listen_port;
        self.common.network_ids = network_ids;
        self.common.min_peers_bucket = min_peers_bucket;
        self
    }
}

pub fn parse_config() -> Fallible<Config> {
    use crate::network::PROTOCOL_MAX_MESSAGE_SIZE;
    let conf = Config::from_args();

    ensure!(
        conf.connection.max_allowed_nodes_percentage >= 100,
        "Can't provide a lower percentage than 100, as that would limit the maximum amount of \
         nodes to less than the desired nodes is set to"
    );

    if let Some(max_allowed_nodes) = conf.connection.max_allowed_nodes {
        ensure!(
            max_allowed_nodes >= conf.connection.desired_nodes,
            "Desired nodes set to {}, but max allowed nodes is set to {}. Max allowed nodes must \
             be greater or equal to desired amounnt of nodes"
        );
    }

    if let Some(hard_connection_limit) = conf.connection.hard_connection_limit {
        ensure!(
            hard_connection_limit >= conf.connection.desired_nodes,
            "Hard connection limit can't be less than what desired nodes is set to"
        );
    }

    ensure!(
        conf.connection.relay_broadcast_percentage >= 0.0
            && conf.connection.relay_broadcast_percentage <= 1.0,
        "Percentage of peers to relay broadcasted packets to, must be between 0.0 and 1.0"
    );

    ensure!(
        conf.cli.baker.maximum_block_size <= 4_000_000_000
            && ((f64::from(conf.cli.baker.maximum_block_size) * 0.9).ceil()) as u32
                <= PROTOCOL_MAX_MESSAGE_SIZE,
        "Maximum block size set higher than 90% of network protocol max size ({})",
        PROTOCOL_MAX_MESSAGE_SIZE
    );

    ensure!(
        conf.connection.socket_read_size >= 65535,
        "Socket read size must be set to at least 65535"
    );

    ensure!(
        conf.connection.socket_read_size >= conf.connection.socket_write_size,
        "Socket read size must be greater or equal to the write size"
    );

    ensure!(
        conf.cli.breakage_type.is_some()
            && conf.cli.breakage_target.is_some()
            && conf.cli.breakage_level.is_some()
            || conf.cli.breakage_type.is_none()
                && conf.cli.breakage_target.is_none()
                && conf.cli.breakage_level.is_none(),
        "The 3 breakage options (breakage-type, breakage-target, breakage-level) must be enabled \
         or disabled together"
    );

    if let Some(ref breakage_type) = conf.cli.breakage_type {
        ensure!(["spam", "fuzz"].contains(&breakage_type.as_str()), "Unsupported breakage-type");
        if let Some(breakage_target) = conf.cli.breakage_target {
            ensure!([0, 1, 2, 3, 4, 99].contains(&breakage_target), "Unsupported breakage-target");
        }
    }

    Ok(conf)
}

#[derive(Debug)]
pub struct AppPreferences {
    preferences_map:     PreferencesMap<String>,
    override_data_dir:   Option<String>,
    override_config_dir: Option<String>,
}

impl AppPreferences {
    pub fn new(override_conf: Option<String>, override_data: Option<String>) -> Self {
        let file_path = Self::calculate_config_file_path(&override_conf, APP_PREFERENCES_MAIN);
        let mut new_prefs = match OpenOptions::new().read(true).write(true).open(&file_path) {
            Ok(file) => {
                let mut reader = BufReader::new(&file);
                let load_result = PreferencesMap::<String>::load_from(&mut reader);
                let prefs = load_result.unwrap_or_else(|_| PreferencesMap::<String>::new());

                AppPreferences {
                    preferences_map:     prefs,
                    override_data_dir:   override_data,
                    override_config_dir: override_conf,
                }
            }
            _ => match File::create(&file_path) {
                Ok(_) => {
                    let prefs = PreferencesMap::<String>::new();
                    AppPreferences {
                        preferences_map:     prefs,
                        override_data_dir:   override_data,
                        override_config_dir: override_conf,
                    }
                }
                _ => panic!("Can't write to config file!"),
            },
        };
        new_prefs.set_config(APP_PREFERENCES_KEY_VERSION, Some(super::VERSION.to_string()));
        new_prefs
    }

    fn calculate_config_path(override_path: &Option<String>) -> PathBuf {
        match override_path {
            Some(ref path) => PathBuf::from(path),
            None => app_root(AppDataType::UserConfig, &APP_INFO)
                .expect("Filesystem error encountered when creating app_root"),
        }
    }

    fn calculate_data_path(override_path: &Option<String>) -> PathBuf {
        match override_path {
            Some(ref path) => PathBuf::from(path),
            None => app_root(AppDataType::UserData, &APP_INFO)
                .expect("Filesystem error encountered when creating app_root"),
        }
    }

    fn calculate_config_file_path(override_config_path: &Option<String>, key: &str) -> PathBuf {
        match override_config_path {
            Some(ref path) => {
                let mut new_path = PathBuf::from(path);
                new_path.push(&format!("{}.json", key));
                new_path
            }
            None => {
                let mut path = Self::calculate_config_path(&None);
                path.push(&format!("{}.json", key));
                path
            }
        }
    }

    pub fn set_config(&mut self, key: &str, value: Option<String>) -> bool {
        match value {
            Some(val) => self.preferences_map.insert(key.to_string(), val),
            _ => self.preferences_map.remove(&key.to_string()),
        };
        let file_path =
            Self::calculate_config_file_path(&self.override_config_dir, APP_PREFERENCES_MAIN);
        match OpenOptions::new().read(true).write(true).open(&file_path) {
            Ok(ref mut file) => {
                let mut writer = BufWriter::new(file);
                if self.preferences_map.save_to(&mut writer).is_err() {
                    error!("Couldn't save config file changes");
                    return false;
                }
                writer.flush().ok();
                true
            }
            _ => {
                error!("Couldn't save config file changes");
                false
            }
        }
    }

    pub fn get_config(&self, key: &str) -> Option<String> { self.preferences_map.get(key).cloned() }

    pub fn get_user_app_dir(&self) -> PathBuf { Self::calculate_data_path(&self.override_data_dir) }

    pub fn get_user_config_dir(&self) -> PathBuf {
        Self::calculate_config_path(&self.override_config_dir)
    }
}
