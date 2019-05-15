use chrono::prelude::{DateTime, Utc};

use std::collections::HashMap;

use crate::{block::*, finalization::*, transaction::*};

#[derive(Debug)]
pub enum BlockStatus {
    Alive,
    Dead,
    Finalized(FinalizationRecord),
}

#[allow(dead_code)]
#[derive(Debug, Default)]
struct ConsensusStatistics {
    blocks_received:                 u64,
    blocks_verified:                 u64,
    block_last_received:             Option<DateTime<Utc>>,
    block_receive_latency_ema:       f64,
    block_receive_latency_ema_emvar: f64,
    block_receive_period_ema:        Option<f64>,
    block_receive_period_ema_emvar:  Option<f64>,
    block_last_arrived:              Option<DateTime<Utc>>,
    block_arrive_latency_ema:        f64,
    block_arrive_latency_ema_emvar:  f64,
    block_arrive_period_ema:         Option<f64>,
    block_arrive_period_ema_emvar:   Option<f64>,
    transactions_per_block_ema:      f64,
    transactions_per_block_emvar:    f64,
    finalization_count:              u64,
    last_finalized_time:             Option<DateTime<Utc>>,
    finalization_period_ema:         Option<f64>,
    finalization_period_emvar:       Option<f64>,
}

#[derive(Debug, Default)]
pub struct SkovData {
    block_table:            HashMap<BlockHash, (BlockPtr, BlockStatus)>,
    possibly_pending_table: HashMap<BlockHash, Vec<PendingBlock>>,
    // possibly_pending_queue: , // TODO: decide on a priority queue impl based on use
    // awaiting_last_finalized: , // ditto
    finalization_list:    Vec<(FinalizationRecord, BlockPtr)>,
    finalization_pool:    Vec<(FinalizationIndex, Vec<FinalizationRecord>)>,
    // branches:             Vec<BlockPtr>,
    genesis_block_ptr:    Option<BlockPtr>,
    // focus_block:          BlockPtr,
    // pending_transactions: PendingTransactionTable,
    transaction_table:    TransactionTable,
    // statistics:           ConsensusStatistics,
}

impl SkovData {
    pub fn add_genesis(&mut self, genesis_block_ptr: BlockPtr) {
        let genesis_finalization_record = FinalizationRecord::genesis(&genesis_block_ptr);
        let genesis_status = BlockStatus::Finalized(genesis_finalization_record.clone());

        self.block_table.insert(
            genesis_block_ptr.hash.clone(),
            (genesis_block_ptr.clone(), genesis_status)
        );

        self.finalization_list.push((genesis_finalization_record, genesis_block_ptr.clone()));

        self.genesis_block_ptr = Some(genesis_block_ptr);
    }
}
