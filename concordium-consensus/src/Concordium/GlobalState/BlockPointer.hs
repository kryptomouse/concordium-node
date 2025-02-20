{-# LANGUAGE DataKinds #-}
{-# LANGUAGE TypeFamilies #-}
module Concordium.GlobalState.BlockPointer where

import Data.Kind
import Data.Serialize
import Data.Word
import Data.Hashable
import Concordium.Types.HashableTo
import Concordium.GlobalState.Block
-- import qualified Concordium.Crypto.SHA256 as Hash
import Concordium.Types
import Data.Time.Clock
import Data.Time.Clock.POSIX

class (Eq bp, Show bp, BlockData bp) => BlockPointerData bp where
    -- |Hash of the block
    bpHash :: bp -> BlockHash
    -- |Height of the block in the tree
    bpHeight :: bp -> BlockHeight
    -- |Time at which the block was first received
    bpReceiveTime :: bp -> UTCTime
    -- |Time at which the block was first considered part of the tree (validated)
    bpArriveTime :: bp -> UTCTime
    -- |Number of transactions in a block
    bpTransactionCount :: bp -> Int
    -- |Energy cost of all transactions in the block.
    bpTransactionsEnergyCost :: bp -> Energy
    -- |Size of the transaction data in bytes.
    bpTransactionsSize :: bp -> Int
    -- |Hash of last-finalized block
    bpLastFinalizedHash :: bp -> BlockHash

-- |Block pointer data. The minimal data that should be the same among all
-- block pointer instantiations.
data BasicBlockPointerData = BasicBlockPointerData {
    -- |Hash of the block
    _bpHash :: !BlockHash,
    -- |Height of the block in the tree
    _bpHeight :: !BlockHeight,
    -- |Time at which the block was first received
    _bpReceiveTime :: !UTCTime,
    -- |Time at which the block was first considered part of the tree (validated)
    _bpArriveTime :: !UTCTime,
    -- |Number of transactions in a block
    _bpTransactionCount :: !Int,
    -- |Energy cost of all transactions in the block.
    _bpTransactionsEnergyCost :: !Energy,
    -- |Size of the transaction data in bytes.
    _bpTransactionsSize :: !Int,
    -- |Hash of last-finalized block
    _bpLastFinalizedHash :: !BlockHash
}

instance Eq BasicBlockPointerData where
    {-# INLINE (==) #-}
    bp1 == bp2 = _bpHash bp1 == _bpHash bp2

instance Ord BasicBlockPointerData where
    {-# INLINE compare #-}
    compare bp1 bp2 = compare (_bpHash bp1) (_bpHash bp2)

instance Hashable BasicBlockPointerData where
    {-# INLINE hashWithSalt #-}
    hashWithSalt s = hashWithSalt s . _bpHash
    {-# INLINE hash #-}
    hash = hash . _bpHash

instance Show BasicBlockPointerData where
    show = show . _bpHash

instance HashableTo BlockHash BasicBlockPointerData where
    {-# INLINE getHash #-}
    getHash = _bpHash

instance Serialize BasicBlockPointerData where
    put BasicBlockPointerData{..} = do
        let
            timeWord :: UTCTime -> Word64
            timeWord = floor . utcTimeToPOSIXSeconds
        put _bpHash
        put _bpHeight
        put (timeWord _bpReceiveTime)
        put (timeWord _bpArriveTime)
        put _bpTransactionCount
        put _bpTransactionsEnergyCost
        put _bpTransactionsSize
        put _bpLastFinalizedHash
    get = do
        let
            wordTime :: Word64 -> UTCTime
            wordTime = posixSecondsToUTCTime . realToFrac
        _bpHash <- get
        _bpHeight <- get
        _bpReceiveTime <- wordTime <$> get
        _bpArriveTime <- wordTime <$> get
        _bpTransactionCount <- get
        _bpTransactionsEnergyCost <- get
        _bpTransactionsSize <- get
        _bpLastFinalizedHash <- get
        return BasicBlockPointerData{..}

-- |The type of a block pointer that was added to the tree and is
-- linked to the blockstate and its parent and last finalized blocks.
--
-- @s@ stands for the blockstate type and @p@ stands for the type of the
-- pointers.
--
-- An in-memory implementation should use `p ~ Identity` to make it
-- work as a normal reference. A disk implementation might consider
-- using `p ~ Weak` to get pointers that don't retain the parent
-- and last finalized blocks. The type @p (BlockPointer t p s)@ will
-- be used inside the `BlockPointerMonad` to resolve the actual blocks.
--
-- All instances of this type will implement automatically:
--
-- * BlockFieldType & BlockTransactionType
-- * BlockData
-- * BlockPointerData
-- * HashableTo BlockHash
data BlockPointer (pv :: ProtocolVersion) ati (p :: Type -> Type) s = BlockPointer {
    -- |Information about the block, e.g., height, transactions, ...
    _bpInfo :: !BasicBlockPointerData,
    -- |Pointer to the parent (circular reference for genesis block)
    _bpParent :: p (BlockPointer pv ati p s),
    -- |Pointer to the last finalized block (circular for genesis)
    _bpLastFinalized :: p (BlockPointer pv ati p s),
    -- |The block itself
    _bpBlock :: !(Block pv),
    -- |The handle for accessing the state (of accounts, contracts, etc.) after execution of the block.
    _bpState :: !s,
    -- |Handle to access the account transaction index; the index of which transactions affect which accounts.
    _bpATI :: !ati
}

type instance BlockFieldType (BlockPointer pv ati p s) = BlockFields

instance Eq (BlockPointer pv ati p s) where
    bp1 == bp2 = _bpInfo bp1 == _bpInfo bp2

instance Ord (BlockPointer pv ati p s) where
    compare bp1 bp2 = compare (_bpInfo bp1) (_bpInfo bp2)

instance Hashable (BlockPointer pv ati p s) where
    hashWithSalt s = hashWithSalt s . _bpInfo
    hash = hash . _bpInfo

instance Show (BlockPointer pv ati p s) where
    show = show . _bpInfo

instance HashableTo BlockHash (BlockPointer pv ati p s) where
    getHash = getHash . _bpInfo

instance BlockData (BlockPointer pv ati p s) where
    blockSlot = blockSlot . _bpBlock
    blockFields = blockFields . _bpBlock
    blockTransactions = blockTransactions . _bpBlock
    blockStateHash = blockStateHash . _bpBlock
    blockTransactionOutcomesHash = blockTransactionOutcomesHash . _bpBlock
    blockSignature = blockSignature . _bpBlock
    verifyBlockSignature = verifyBlockSignature . _bpBlock
    {-# INLINE blockSlot #-}
    {-# INLINE blockFields #-}
    {-# INLINE blockTransactions #-}
    {-# INLINE blockStateHash #-}
    {-# INLINE blockTransactionOutcomesHash #-}
    {-# INLINE blockSignature #-}
    {-# INLINE verifyBlockSignature #-}

instance IsProtocolVersion pv => EncodeBlock pv (BlockPointer pv ati p s) where
    putBlock spv = putBlock spv . _bpBlock

instance BlockPointerData (BlockPointer pv ati p s) where
    bpHash = _bpHash . _bpInfo
    bpHeight = _bpHeight . _bpInfo
    bpReceiveTime = _bpReceiveTime . _bpInfo
    bpArriveTime = _bpArriveTime . _bpInfo
    bpTransactionCount = _bpTransactionCount . _bpInfo
    bpTransactionsEnergyCost = _bpTransactionsEnergyCost . _bpInfo
    bpTransactionsSize = _bpTransactionsSize . _bpInfo
    bpLastFinalizedHash = _bpLastFinalizedHash . _bpInfo
