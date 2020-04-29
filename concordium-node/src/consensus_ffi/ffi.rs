use crate::{catch_up::*, consensus::*, messaging::*};
use byteorder::{NetworkEndian, ReadBytesExt};
use concordium_common::{ConsensusFfiResponse, ConsensusIsInCommitteeResponse, PacketType};
use crypto_common::Serial;
use failure::{bail, format_err, Fallible};
use std::{
    convert::TryFrom,
    ffi::{CStr, CString},
    io::{Cursor, Write},
    os::raw::{c_char, c_int},
    path::PathBuf,
    slice,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Once,
    },
};
extern "C" {
    pub fn hs_init(argc: *mut c_int, argv: *mut *mut *mut c_char);
    pub fn hs_init_with_rtsopts(argc: &c_int, argv: *const *const *const c_char);
    pub fn hs_exit();
}

static START_ONCE: Once = Once::new();
static STOP_ONCE: Once = Once::new();
static STOPPED: AtomicBool = AtomicBool::new(false);
pub type Delta = u64;

/// Initialize the Haskell runtime. This function is safe to call more than
/// once, and will do nothing on subsequent calls.
///
/// The runtime will automatically be shutdown at program exit, or you can stop
/// it earlier with `stop`.
#[cfg(all(not(windows), feature = "profiling"))]
pub fn start_haskell(
    heap: &str,
    stack: bool,
    time: bool,
    exceptions: bool,
    gc_log: Option<String>,
    profile_sampling_interval: &str,
    rts_flags: &[String],
) {
    START_ONCE.call_once(|| {
        start_haskell_init(
            heap,
            stack,
            time,
            exceptions,
            gc_log,
            profile_sampling_interval,
            rts_flags,
        );
        unsafe {
            ::libc::atexit(stop_nopanic);
        }
    });
}

#[cfg(not(feature = "profiling"))]
pub fn start_haskell(rts_flags: &[String]) {
    START_ONCE.call_once(|| {
        start_haskell_init(rts_flags);
        unsafe {
            ::libc::atexit(stop_nopanic);
        }
    });
}

#[cfg(all(not(windows), feature = "profiling"))]
fn start_haskell_init(
    heap: &str,
    stack: bool,
    time: bool,
    exceptions: bool,
    gc_log: Option<String>,
    profile_sampling_interval: &str,
    rts_flags: &[String],
) {
    let program_name = std::env::args().take(1).next().unwrap();
    let mut args = vec![program_name];

    // We don't check for stack here because it should only be enabled if
    // heap profiling is enabled.
    if heap != "none" || time || gc_log.is_some() || profile_sampling_interval != "0.1" {
        args.push("+RTS".to_owned());
        args.push("-L100".to_owned());
    }

    match heap {
        "cost" => {
            args.push("-hc".to_owned());
            if stack {
                args.push("-xt".to_owned());
            }
        }
        "module" => {
            args.push("-hm".to_owned());
            if stack {
                args.push("-xt".to_owned());
            }
        }
        "description" => {
            args.push("-hd".to_owned());
            if stack {
                args.push("-xt".to_owned());
            }
        }
        "type" => {
            args.push("-hy".to_owned());
            if stack {
                args.push("-xt".to_owned());
            }
        }
        "none" => {}
        _ => {
            error!("Wrong heap profiling option provided: {}", heap);
        }
    }

    if profile_sampling_interval != "0.1" {
        args.push(format!("-i{}", profile_sampling_interval));
    }

    if time {
        args.push("-p".to_owned());
    }

    if let Some(log) = gc_log {
        args.push(format!("-S{}", log));
    }

    if exceptions {
        if args.len() == 1 {
            args.push("+RTS".to_owned());
        }
        args.push("-xc".to_owned());
    }

    if args.len() == 1 {
        args.push("+RTS".to_owned())
    }

    for flag in rts_flags {
        if !flag.trim().is_empty() {
            args.push(flag.to_owned());
        }
    }

    if args.len() > 1 {
        args.push("-RTS".to_owned());
    }

    info!(
        "Starting consensus with the following profiling arguments {:?}",
        args
    );
    let args = args
        .iter()
        .map(|arg| CString::new(arg.as_bytes()).unwrap())
        .collect::<Vec<CString>>();
    let c_args = args
        .iter()
        .map(|arg| arg.as_ptr())
        .collect::<Vec<*const c_char>>();
    let ptr_c_argc = &(c_args.len() as c_int);
    let ptr_c_argv = &c_args.as_ptr();
    unsafe {
        hs_init_with_rtsopts(ptr_c_argc, ptr_c_argv as *const *const *const c_char);
    }
}

#[cfg(all(not(windows), not(feature = "profiling")))]
fn start_haskell_init(rts_flags: &[String]) {
    let program_name = std::env::args().take(1).next().unwrap();
    let mut args = vec![program_name];
    if !rts_flags.is_empty() {
        args.push("+RTS".to_owned());
        for flag in rts_flags {
            if !flag.trim().is_empty() {
                args.push(flag.to_owned());
            }
        }
        args.push("-RTS".to_owned());
    }
    let args = args
        .iter()
        .map(|arg| CString::new(arg.as_bytes()).unwrap())
        .collect::<Vec<CString>>();
    let c_args = args
        .iter()
        .map(|arg| arg.as_ptr())
        .collect::<Vec<*const c_char>>();
    let ptr_c_argc = &(c_args.len() as c_int);
    let ptr_c_argv = &c_args.as_ptr();
    unsafe {
        hs_init_with_rtsopts(ptr_c_argc, ptr_c_argv as *const *const *const c_char);
    }
}

#[cfg(windows)]
fn start_haskell_init(_: bool, _: bool, _: bool) {
    // GHC on Windows ignores hs_init arguments and uses GetCommandLineW instead
    // See https://hackage.haskell.org/package/base-4.9.0.0/docs/src/GHC.Environment.html
    let mut argv0 = *b"\0";
    let mut argv = [argv0.as_mut_ptr() as *mut c_char, ptr::null_mut()];
    let mut argc = 1;
    unsafe {
        hs_init(&mut argc, &mut argv.as_mut_ptr());
    }
}

/// Stop the Haskell runtime before the program exits. This function may only be
/// called once during a program's execution.
///
/// It is safe, but not useful, to call this before the runtime has started.
///
/// # Panics
///
/// Will panic if called more than once.
pub fn stop_haskell() {
    if STOPPED.swap(true, Ordering::SeqCst) {
        panic!("The GHC runtime may only be stopped once. See \
                https://downloads.haskell.org/%7Eghc/latest/docs/html/users_guide\
                /ffi-chap.html#id1 ");
    }
    stop_nopanic();
}

extern "C" fn stop_nopanic() {
    STOP_ONCE.call_once(|| {
        unsafe { hs_exit() }; // does nothing if hs_init_count <= 0
    });
}

#[repr(C)]
pub struct consensus_runner {
    private: [u8; 0],
}

type LogCallback = extern "C" fn(c_char, c_char, *const u8);
type BroadcastCallback = extern "C" fn(i64, *const u8, i64);
type CatchUpStatusCallback = extern "C" fn(*const u8, i64);
type DirectMessageCallback =
    extern "C" fn(peer_id: PeerId, message_type: i64, msg: *const c_char, msg_len: i64);

#[allow(improper_ctypes)]
extern "C" {
    pub fn startConsensus(
        max_block_size: u64,
        insertions_before_purging: u64,
        transaction_keep_alive: u64,
        genesis_data: *const u8,
        genesis_data_len: i64,
        private_data: *const u8,
        private_data_len: i64,
        broadcast_callback: BroadcastCallback,
        catchup_status_callback: CatchUpStatusCallback,
        maximum_log_level: u8,
        log_callback: LogCallback,
        appdata_dir: *const u8,
        appdata_dir_len: i64,
        database_connection_url: *const u8,
        database_connection_url_len: i64,
    ) -> *mut consensus_runner;
    pub fn startConsensusPassive(
        max_block_size: u64,
        insertions_before_purging: u64,
        transaction_keep_alive: u64,
        genesis_data: *const u8,
        genesis_data_len: i64,
        catchup_status_callback: CatchUpStatusCallback,
        maximum_log_level: u8,
        log_callback: LogCallback,
        appdata_dir: *const u8,
        appdata_dir_len: i64,
        database_connection_url: *const u8,
        database_connection_url_len: i64,
    ) -> *mut consensus_runner;
    #[allow(improper_ctypes)]
    pub fn startBaker(consensus: *mut consensus_runner);
    pub fn receiveBlock(
        consensus: *mut consensus_runner,
        block_data: *const u8,
        data_length: i64,
    ) -> i64;
    pub fn receiveFinalization(
        consensus: *mut consensus_runner,
        finalization_data: *const u8,
        data_length: i64,
    ) -> i64;
    pub fn receiveFinalizationRecord(
        consensus: *mut consensus_runner,
        finalization_data: *const u8,
        data_length: i64,
    ) -> i64;
    pub fn receiveTransaction(
        consensus: *mut consensus_runner,
        tx: *const u8,
        data_length: i64,
    ) -> i64;
    pub fn stopBaker(consensus: *mut consensus_runner);
    pub fn stopConsensus(consensus: *mut consensus_runner);

    // Consensus queries
    pub fn getConsensusStatus(consensus: *mut consensus_runner) -> *const c_char;
    pub fn getBlockInfo(consensus: *mut consensus_runner, block_hash: *const u8) -> *const c_char;
    pub fn getAncestors(
        consensus: *mut consensus_runner,
        block_hash: *const u8,
        amount: u64,
    ) -> *const c_char;
    pub fn getBranches(consensus: *mut consensus_runner) -> *const c_char;

    // State queries
    pub fn getAccountList(consensus: *mut consensus_runner, block_hash: *const u8)
        -> *const c_char;
    pub fn getInstances(consensus: *mut consensus_runner, block_hash: *const u8) -> *const c_char;
    pub fn getAccountInfo(
        consensus: *mut consensus_runner,
        block_hash: *const u8,
        account_address: *const u8,
    ) -> *const c_char;
    pub fn getInstanceInfo(
        consensus: *mut consensus_runner,
        block_hash: *const u8,
        contract_address: *const u8,
    ) -> *const c_char;
    pub fn getRewardStatus(
        consensus: *mut consensus_runner,
        block_hash: *const u8,
    ) -> *const c_char;
    pub fn getBirkParameters(
        consensus: *mut consensus_runner,
        block_hash: *const u8,
    ) -> *const c_char;
    pub fn getModuleList(consensus: *mut consensus_runner, block_hash: *const u8) -> *const c_char;
    pub fn getModuleSource(
        consensus: *mut consensus_runner,
        block_hash: *const u8,
        module_ref: *const u8,
    ) -> *const u8;
    pub fn getBlock(consensus: *mut consensus_runner, block_hash: *const u8) -> *const u8;
    pub fn getBlockDelta(
        consensus: *mut consensus_runner,
        block_hash: *const u8,
        delta: Delta,
    ) -> *const u8;
    pub fn freeCStr(hstring: *const c_char);
    pub fn getCatchUpStatus(consensus: *mut consensus_runner) -> *const u8;
    pub fn receiveCatchUpStatus(
        consensus: *mut consensus_runner,
        peer_id: PeerId,
        msg: *const u8,
        msg_len: i64,
        object_limit: i64,
        direct_callback: DirectMessageCallback,
    ) -> i64;
    pub fn checkIfWeAreBaker(consensus: *mut consensus_runner) -> u8;
    pub fn checkIfWeAreFinalizer(consensus: *mut consensus_runner) -> u8;
    pub fn getAccountNonFinalizedTransactions(
        consensus: *mut consensus_runner,
        account_address: *const u8,
    ) -> *const c_char;
    pub fn getBlockSummary(
        consensus: *mut consensus_runner,
        block_hash: *const u8,
    ) -> *const c_char;
    pub fn getTransactionStatus(
        consensus: *mut consensus_runner,
        transaction_hash: *const u8,
    ) -> *const c_char;
    pub fn getTransactionStatusInBlock(
        consensus: *mut consensus_runner,
        transaction_hash: *const u8,
        block_hash: *const u8,
    ) -> *const c_char;
    pub fn getNextAccountNonce(
        consensus: *mut consensus_runner,
        account_address: *const u8,
    ) -> *const c_char;
    pub fn importBlocks(
        consensus: *mut consensus_runner,
        import_file_path: *const u8,
        import_file_path_len: i64,
    ) -> u8;
}

// TODO : Simplify arguments to function, or group with struct
#[allow(clippy::too_many_arguments)]
pub fn get_consensus_ptr(
    max_block_size: u64,
    insertions_before_purging: u64,
    transaction_keep_alive: u64,
    genesis_data: Vec<u8>,
    private_data: Option<Vec<u8>>,
    maximum_log_level: ConsensusLogLevel,
    appdata_dir: &PathBuf,
    database_connection_url: &str,
) -> Fallible<*mut consensus_runner> {
    let genesis_data_len = genesis_data.len();

    let c_string_genesis = unsafe { CString::from_vec_unchecked(genesis_data) };

    let consensus_ptr = match private_data {
        Some(ref private_data_bytes) => {
            let private_data_len = private_data_bytes.len();
            let appdata_buf = appdata_dir.as_path().to_str().unwrap();
            unsafe {
                let c_string_private_data =
                    CString::from_vec_unchecked(private_data_bytes.to_owned());

                startConsensus(
                    max_block_size,
                    insertions_before_purging,
                    transaction_keep_alive,
                    c_string_genesis.as_ptr() as *const u8,
                    genesis_data_len as i64,
                    c_string_private_data.as_ptr() as *const u8,
                    private_data_len as i64,
                    broadcast_callback,
                    catchup_status_callback,
                    maximum_log_level as u8,
                    on_log_emited,
                    appdata_buf.as_ptr() as *const u8,
                    appdata_buf.len() as i64,
                    database_connection_url.as_ptr() as *const u8,
                    database_connection_url.len() as i64,
                )
            }
        }
        None => {
            let appdata_buf = appdata_dir.as_path().to_str().unwrap();
            unsafe {
                {
                    startConsensusPassive(
                        max_block_size,
                        insertions_before_purging,
                        transaction_keep_alive,
                        c_string_genesis.as_ptr() as *const u8,
                        genesis_data_len as i64,
                        catchup_status_callback,
                        maximum_log_level as u8,
                        on_log_emited,
                        appdata_buf.as_ptr() as *const u8,
                        appdata_buf.len() as i64,
                        database_connection_url.as_ptr() as *const u8,
                        database_connection_url.len() as i64,
                    )
                }
            }
        }
    };

    if consensus_ptr.is_null() {
        bail!("Could not start consensus")
    } else {
        Ok(consensus_ptr)
    }
}

impl ConsensusContainer {
    pub fn send_block(&self, block: &[u8]) -> ConsensusFfiResponse {
        wrap_send_data_to_c!(self, block, receiveBlock)
    }

    pub fn send_finalization(&self, msg: &[u8]) -> ConsensusFfiResponse {
        wrap_send_data_to_c!(self, msg, receiveFinalization)
    }

    pub fn send_finalization_record(&self, rec: &[u8]) -> ConsensusFfiResponse {
        wrap_send_data_to_c!(self, rec, receiveFinalizationRecord)
    }

    pub fn send_transaction(&self, data: &[u8]) -> ConsensusFfiResponse {
        let consensus = self.consensus.load(Ordering::SeqCst);
        let len = data.len();

        let result = unsafe {
            receiveTransaction(
                consensus,
                CString::from_vec_unchecked(data.to_vec()).as_ptr() as *const u8,
                len as i64,
            )
        };

        let return_code = ConsensusFfiResponse::try_from(result);

        return_code.unwrap_or_else(|code| panic!("Unknown FFI return code: {}", code))
    }

    pub fn get_consensus_status(&self) -> String {
        wrap_c_call_string!(self, consensus, |consensus| getConsensusStatus(consensus))
    }

    pub fn get_block_info(&self, block_hash: &str) -> String {
        let c_str = CString::new(block_hash).unwrap();
        wrap_c_call_string!(self, consensus, |consensus| getBlockInfo(
            consensus,
            c_str.as_ptr() as *const u8
        ))
    }

    pub fn get_ancestors(&self, block_hash: &str, amount: u64) -> String {
        let c_str = CString::new(block_hash).unwrap();
        wrap_c_call_string!(self, consensus, |consensus| getAncestors(
            consensus,
            c_str.as_ptr() as *const u8,
            amount
        ))
    }

    pub fn get_branches(&self) -> String {
        wrap_c_call_string!(self, consensus, |consensus| getBranches(consensus))
    }

    pub fn get_account_list(&self, block_hash: &str) -> String {
        let block_hash = CString::new(block_hash).unwrap();
        wrap_c_call_string!(self, consensus, |consensus| getAccountList(
            consensus,
            block_hash.as_ptr() as *const u8
        ))
    }

    pub fn get_instances(&self, block_hash: &str) -> String {
        let block_hash = CString::new(block_hash).unwrap();
        wrap_c_call_string!(self, consensus, |consensus| getInstances(
            consensus,
            block_hash.as_ptr() as *const u8
        ))
    }

    pub fn get_account_info(&self, block_hash: &str, account_address: &str) -> String {
        let block_hash = CString::new(block_hash).unwrap();
        let account_address = CString::new(account_address).unwrap();
        wrap_c_call_string!(self, consensus, |consensus| getAccountInfo(
            consensus,
            block_hash.as_ptr() as *const u8,
            account_address.as_ptr() as *const u8
        ))
    }

    pub fn get_instance_info(&self, block_hash: &str, contract_address: &str) -> String {
        let block_hash = CString::new(block_hash).unwrap();
        let contract_address = CString::new(contract_address).unwrap();
        wrap_c_call_string!(self, consensus, |consensus| getInstanceInfo(
            consensus,
            block_hash.as_ptr() as *const u8,
            contract_address.as_ptr() as *const u8
        ))
    }

    pub fn get_reward_status(&self, block_hash: &str) -> String {
        let block_hash = CString::new(block_hash).unwrap();
        wrap_c_call_string!(self, consensus, |consensus| getRewardStatus(
            consensus,
            block_hash.as_ptr() as *const u8
        ))
    }

    pub fn get_birk_parameters(&self, block_hash: &str) -> String {
        let block_hash = CString::new(block_hash).unwrap();
        wrap_c_call_string!(self, consensus, |consensus| getBirkParameters(
            consensus,
            block_hash.as_ptr() as *const u8
        ))
    }

    pub fn get_module_list(&self, block_hash: &str) -> String {
        let block_hash = CString::new(block_hash).unwrap();
        wrap_c_call_string!(self, consensus, |consensus| getModuleList(
            consensus,
            block_hash.as_ptr() as *const u8
        ))
    }

    pub fn get_module_source(&self, block_hash: &str, module_ref: &str) -> Vec<u8> {
        let block_hash = CString::new(block_hash).unwrap();
        let module_ref = CString::new(module_ref).unwrap();
        wrap_c_call_bytes!(self, |consensus| getModuleSource(
            consensus,
            block_hash.as_ptr() as *const u8,
            module_ref.as_ptr() as *const u8
        ))
    }

    pub fn get_block(&self, _block_hash: &[u8]) -> Vec<u8> {
        wrap_c_call_bytes!(self, |consensus| getBlock(consensus, _block_hash.as_ptr()))
    }

    pub fn get_block_by_delta(&self, _block_hash: &[u8], delta: Delta) -> Vec<u8> {
        wrap_c_call_bytes!(self, |consensus| getBlockDelta(
            consensus,
            _block_hash.as_ptr(),
            delta
        ))
    }

    pub fn get_catch_up_status(&self) -> Arc<[u8]> {
        wrap_c_call_payload!(
            self,
            |consensus| getCatchUpStatus(consensus),
            &(PacketType::CatchUpStatus as u8).to_be_bytes()
        )
    }

    pub fn receive_catch_up_status(
        &self,
        request: &[u8],
        peer_id: PeerId,
        object_limit: i64,
    ) -> ConsensusFfiResponse {
        wrap_c_call!(self, |consensus| receiveCatchUpStatus(
            consensus,
            peer_id,
            request.as_ptr(),
            request.len() as i64,
            object_limit,
            direct_callback
        ))
    }

    pub fn in_baking_committee(&self) -> ConsensusIsInCommitteeResponse {
        wrap_c_committee_call!(self, |consensus| checkIfWeAreBaker(consensus))
    }

    pub fn in_finalization_committee(&self) -> bool {
        wrap_c_bool_call!(self, |consensus| checkIfWeAreFinalizer(consensus))
    }

    pub fn get_account_non_finalized_transactions(&self, account_address: &str) -> String {
        let account_address = CString::new(account_address).unwrap();
        wrap_c_call_string!(self, consensus, |consensus| {
            getAccountNonFinalizedTransactions(consensus, account_address.as_ptr() as *const u8)
        })
    }

    pub fn get_block_summary(&self, block_hash: &str) -> String {
        let block_hash = CString::new(block_hash).unwrap();
        wrap_c_call_string!(self, consensus, |consensus| getBlockSummary(
            consensus,
            block_hash.as_ptr() as *const u8
        ))
    }

    pub fn get_transaction_status(&self, transaction_hash: &str) -> String {
        let transaction_hash = CString::new(transaction_hash).unwrap();
        wrap_c_call_string!(self, consensus, |consensus| getTransactionStatus(
            consensus,
            transaction_hash.as_ptr() as *const u8
        ))
    }

    pub fn get_transaction_status_in_block(
        &self,
        transaction_hash: &str,
        block_hash: &str,
    ) -> String {
        let transaction_hash = CString::new(transaction_hash).unwrap();
        let block_hash = CString::new(block_hash).unwrap();
        wrap_c_call_string!(self, consensus, |consensus| getTransactionStatusInBlock(
            consensus,
            transaction_hash.as_ptr() as *const u8,
            block_hash.as_ptr() as *const u8
        ))
    }

    pub fn get_next_account_nonce(&self, account_address: &str) -> String {
        let account_address = CString::new(account_address).unwrap();
        wrap_c_call_string!(self, consensus, |consensus| getNextAccountNonce(
            consensus,
            account_address.as_ptr() as *const u8
        ))
    }

    pub fn import_blocks(&self, import_file_path: &[u8]) -> u8 {
        let consensus = self.consensus.load(Ordering::SeqCst);
        let len = import_file_path.len();

        unsafe {
            importBlocks(
                consensus,
                CString::from_vec_unchecked(import_file_path.to_vec()).as_ptr() as *const u8,
                len as i64,
            )
        }
    }
}

pub enum CallbackType {
    Block = 0,
    FinalizationMessage,
    FinalizationRecord,
    CatchUpStatus,
}

impl TryFrom<u8> for CallbackType {
    type Error = failure::Error;

    fn try_from(byte: u8) -> Fallible<Self> {
        match byte {
            0 => Ok(CallbackType::Block),
            1 => Ok(CallbackType::FinalizationMessage),
            2 => Ok(CallbackType::FinalizationRecord),
            3 => Ok(CallbackType::CatchUpStatus),
            _ => Err(format_err!("Received invalid callback type: {}", byte)),
        }
    }
}

pub extern "C" fn on_finalization_message_catchup_out(peer_id: PeerId, data: *const u8, len: i64) {
    unsafe {
        let msg_variant = PacketType::FinalizationMessage;
        let payload = slice::from_raw_parts(data as *const u8, len as usize);
        let mut full_payload = Vec::with_capacity(1 + payload.len());
        (msg_variant as u8).serial(&mut full_payload);

        full_payload.write_all(&payload).unwrap(); // infallible
        let full_payload = Arc::from(full_payload);

        let msg = ConsensusMessage::new(
            MessageType::Outbound(Some(peer_id)),
            PacketType::FinalizationMessage,
            full_payload,
            vec![],
            None,
        );

        match CALLBACK_QUEUE.send_out_blocking_msg(msg) {
            Ok(_) => trace!("Queueing a {} of {} bytes", msg_variant, len),
            Err(e) => error!("Couldn't queue a {} properly: {}", msg_variant, e),
        };
    }
}

macro_rules! sending_callback {
    ($target:expr, $msg_type:expr, $msg:expr, $msg_length:expr, $omit_status:expr) => {
        unsafe {
            let callback_type = match CallbackType::try_from($msg_type as u8) {
                Ok(ct) => ct,
                Err(e) => {
                    error!("{}", e);
                    return;
                }
            };

            let msg_variant = match callback_type {
                CallbackType::Block => PacketType::Block,
                CallbackType::FinalizationMessage => PacketType::FinalizationMessage,
                CallbackType::FinalizationRecord => PacketType::FinalizationRecord,
                CallbackType::CatchUpStatus => PacketType::CatchUpStatus,
            };

            let payload = slice::from_raw_parts($msg as *const u8, $msg_length as usize);
            let mut full_payload = Vec::with_capacity(1 + payload.len());
            (msg_variant as u8).serial(&mut full_payload);
            full_payload.write_all(&payload).unwrap(); // infallible
            let full_payload = Arc::from(full_payload);

            let msg = ConsensusMessage::new(
                MessageType::Outbound($target),
                msg_variant,
                full_payload,
                vec![],
                $omit_status,
            );

            match CALLBACK_QUEUE.send_out_blocking_msg(msg) {
                Ok(_) => trace!("Queueing a {} of {} bytes", msg_variant, $msg_length),
                Err(e) => error!("Couldn't queue a {} properly: {}", msg_variant, e),
            };
        }
    };
}

pub extern "C" fn broadcast_callback(msg_type: i64, msg: *const u8, msg_length: i64) {
    trace!("Broadcast callback hit - queueing message");
    sending_callback!(None, msg_type, msg, msg_length, None);
}

pub extern "C" fn direct_callback(
    peer_id: PeerId,
    msg_type: i64,
    msg: *const c_char,
    msg_length: i64,
) {
    trace!("Direct callback hit - queueing message");
    sending_callback!(Some(peer_id), msg_type, msg, msg_length, None);
}

pub extern "C" fn catchup_status_callback(msg: *const u8, msg_length: i64) {
    trace!("Catch-up status callback hit - queueing message");
    sending_callback!(
        None,
        CallbackType::CatchUpStatus,
        msg,
        msg_length,
        Some(PeerStatus::Pending)
    );
}

/// Following the implementation of the log crate, error = 1, warning = 2, info
/// = 3, 4 = debug, any other option is considered as trace.
pub extern "C" fn on_log_emited(identifier: c_char, log_level: c_char, log_message: *const u8) {
    fn identifier_to_string(id: c_char) -> &'static str {
        match id {
            1 => "Runner",
            2 => "Afgjort",
            3 => "Birk",
            4 => "Crypto",
            5 => "Kontrol",
            6 => "Skov",
            7 => "Baker",
            _ => "External",
        }
    }

    let msg = unsafe { CStr::from_ptr(log_message as *const c_char) }
        .to_str()
        .expect("log_callback: unable to decode as UTF-8");
    let id = identifier_to_string(identifier);

    match log_level as u8 {
        1 => error!("{}: {}", id, msg),
        2 => warn!("{}: {}", id, msg),
        3 => info!("{}: {}", id, msg),
        4 => debug!("{}: {}", id, msg),
        _ => trace!("{}: {}", id, msg),
    };
}
