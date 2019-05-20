use chrono::prelude::{DateTime, Utc};
use failure::{bail, Fallible};

use std::collections::{BinaryHeap, HashMap};

use crate::{
    block::*,
    common::{HashBytes, Slot},
    finalization::*,
    transaction::*,
};

#[derive(Debug)]
pub enum BlockStatus {
    Alive,
    Dead,
    Finalized,
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
    // the blocks whose parent and last finalized blocks are already in the tree
    pub block_tree: HashMap<BlockHash, (BlockPtr, BlockStatus)>,
    // blocks waiting for their parent to be added to the tree; the key is the parent's hash
    orphan_blocks: HashMap<BlockHash, Vec<PendingBlock>>,
    // finalization records along with their finalized blocks; those must already be in the tree
    finalization_list: BinaryHeap<(FinalizationRecord, BlockPtr)>,
    // blocks waiting for their last finalized block to be added to the tree
    awaiting_last_finalized: HashMap<BlockHash, Vec<PendingBlock>>,
    // the pointer to the genesis block; optional only due to SkovData being a lazy_static
    genesis_block_ptr: Option<BlockPtr>,
    // contains transactions
    transaction_table: TransactionTable,

    // focus_block: BlockPtr,
}

impl SkovData {
    pub fn add_genesis(&mut self, genesis_block_ptr: BlockPtr) {
        let genesis_finalization_record = FinalizationRecord::genesis(&genesis_block_ptr);

        self.block_tree.insert(
            genesis_block_ptr.hash.clone(),
            (genesis_block_ptr.clone(), BlockStatus::Finalized),
        );

        self.finalization_list
            .push((genesis_finalization_record, genesis_block_ptr.clone()));

        info!(
            "block tree: [{:?}({:?})]",
            genesis_block_ptr.hash,
            BlockStatus::Finalized,
        );

        self.genesis_block_ptr = Some(genesis_block_ptr);
    }

    pub fn add_block(&mut self, pending_block: PendingBlock) -> Fallible<Option<(BlockPtr, BlockStatus)>> {
        // verify if the pending block's parent block is already in the tree
        let parent_block =
            if let Some(block_ptr) = self.get_block_by_hash(&pending_block.block.pointer) {
                block_ptr.to_owned()
            } else {
                let warning = format!("Couldn't find the parent block ({:?}) of block {:?}; \
                    to the pending list!", pending_block.block.pointer, pending_block.hash);
                self.queue_orphan_block(pending_block);
                bail!(warning);
            };

        // verify if the pending block's last finalized block is already in the tree
        let last_finalized = self.get_last_finalized().to_owned();
        if last_finalized.hash != pending_block.block.last_finalized {
            let warning = format!("Block {:?} points to a finalization record ({:?}) which is not \
                the last one ({:?})",
                pending_block.hash, pending_block.block.last_finalized, last_finalized.hash,
            );
            self.queue_block_wo_last_finalized(pending_block);
            bail!(warning);
        }

        // if the above checks pass, a BlockPtr can be created
        let block_ptr = BlockPtr::new(pending_block, parent_block, last_finalized, Utc::now());

        let ret = self
            .block_tree
            .insert(block_ptr.hash.clone(), (block_ptr, BlockStatus::Alive));
        info!(
            "block tree: {:?}",
            {
                let mut vals = self.block_tree.values().collect::<Vec<_>>();
                vals.sort_by_key(|(ptr, _)| ptr.block.slot());
                vals.into_iter().map(|(ptr, status)| (ptr.hash.to_owned(), status)).collect::<Vec<_>>()
            }
        );

        Ok(ret)
    }

    pub fn get_block_by_hash(&self, hash: &HashBytes) -> Option<&BlockPtr> {
        self.block_tree.get(hash).map(|(ptr, _)| ptr)
    }

    pub fn get_last_finalized(&self) -> &BlockPtr {
        &self.finalization_list.peek().unwrap().1 // safe; the genesis is always available
    }

    pub fn get_last_finalized_slot(&self) -> Slot { self.get_last_finalized().block.slot() }

    pub fn get_last_finalized_height(&self) -> BlockHeight { self.get_last_finalized().height }

    pub fn get_next_finalization_index(&self) -> FinalizationIndex {
        &self.finalization_list.peek().unwrap().0.index + 1 // safe; the genesis is always available
    }

    pub fn add_finalization(&mut self, record: FinalizationRecord) -> bool {
        let block_ptr = if let Some((ref ptr, ref mut status)) =
            self.block_tree.get_mut(&record.block_pointer)
        {
            *status = BlockStatus::Finalized;
            ptr.clone()
        } else {
            panic!(
                "Can't find finalized block {:?} in the block table!",
                record.block_pointer
            );
        };

        // we should be ok with a linear search, as we are expecting only to keep the
        // most recent finalization records
        if self
            .finalization_list
            .iter()
            .find(|&(rec, _)| *rec == record)
            .is_none()
        {
            self.finalization_list.push((record, block_ptr));
            debug!(
                "finalization list: {:?}",
                self.finalization_list
                    .clone()
                    .into_sorted_vec()
                    .iter()
                    .map(|(rec, _)| &rec.block_pointer)
                    .collect::<Vec<_>>()
            );
            true
        } else {
            false
        }
    }

    fn queue_orphan_block(&mut self, pending_block: PendingBlock) {
        let parent = pending_block.block.pointer.to_owned();
        let queued = self.orphan_blocks.entry(parent).or_default();
        queued.push(pending_block);
        info!("pending blocks: {:?}", self.orphan_blocks.iter().map(|(parent, pending)| (parent, pending.iter().map(|pb| pb.hash.to_owned()).collect::<Vec<_>>())).collect::<Vec<_>>());
    }

    fn queue_block_wo_last_finalized(&mut self, pending_block: PendingBlock) {
        let last_finalized = pending_block.block.last_finalized.to_owned();
        let queued = self.awaiting_last_finalized.entry(last_finalized).or_default();
        queued.push(pending_block);
        info!("blocks awaiting last finalized: {:?}", self.awaiting_last_finalized.iter().map(|(last_finalized, pending)| (last_finalized, pending.iter().map(|pb| pb.hash.to_owned()).collect::<Vec<_>>())).collect::<Vec<_>>());
    }
}
