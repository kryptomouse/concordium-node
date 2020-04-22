{-# LANGUAGE
    ViewPatterns,
    TypeApplications,
    ScopedTypeVariables #-}
module Concordium.Skov.CatchUp where

import qualified Data.Set as Set
import qualified Data.Sequence as Seq
import qualified Data.List as List
import Data.Function
import Data.Foldable
import Data.ByteString (ByteString)
import Data.Serialize
import Data.Proxy
import Control.Monad

import Concordium.GlobalState.BlockPointer hiding (BlockPointer)
import Concordium.GlobalState.BlockMonads
import Concordium.GlobalState.Finalization
import Concordium.GlobalState.TreeState hiding (getGenesisData)
import Concordium.Types

import Concordium.Logger
import Concordium.Skov.CatchUp.Types
import Concordium.Skov.Monad
import Concordium.Kontrol.BestBlock
import Concordium.Afgjort.Finalize

makeCatchUpStatus :: (BlockPointerMonad m) => Bool -> Bool -> (BlockPointerType m) -> [BlockPointerType m] -> [BlockPointerType m] -> m CatchUpStatus
makeCatchUpStatus cusIsRequest cusIsResponse lfb leaves branches = return CatchUpStatus{..}
    where
        cusLastFinalizedBlock = bpHash lfb
        cusLastFinalizedHeight = bpHeight lfb
        cusLeaves = bpHash <$> leaves
        cusBranches = bpHash <$> branches

-- |Given a list of lists representing branches (ordered by height),
-- produce a pair of lists @(leaves, branches)@, which partions
-- those blocks that are leaves (@leaves@) from those that are not
-- (@branches@).
leavesBranches :: forall m. (BlockPointerMonad m) => [[BlockPointerType m]] -> m ([BlockPointerType m], [BlockPointerType m])
leavesBranches = lb ([], [])
    where
        lb :: ([BlockPointerType m], [BlockPointerType m]) -> [[BlockPointerType m]] -> m ([BlockPointerType m], [BlockPointerType m])
        lb lsbs [] = return lsbs
        lb (ls, bs) [ls'] = return (ls ++ ls', bs)
        lb (ls, bs) (s:r@(n:_)) = do
          parent <- mapM bpParent n
          let (bs', ls') = List.partition (`elem` parent) s
          lb (ls ++ ls', bs ++ bs') r

doGetCatchUpStatus :: (TreeStateMonad m, SkovQueryMonad m) => Bool -> m CatchUpStatus
doGetCatchUpStatus cusIsRequest = do
        lfb <- lastFinalizedBlock
        br <- toList <$> getBranches
        (leaves, branches) <- leavesBranches br
        makeCatchUpStatus cusIsRequest False lfb leaves (if cusIsRequest then branches else [])

merge :: forall m . (BlockPointerData (BlockPointerType m))
      => Proxy m
      -> [(FinalizationRecord, BlockHeight)]
      -> Seq.Seq (BlockPointerType m)
      -> [(MessageType, ByteString)]
merge Proxy = go
  where encodeBlock b = (MessageBlock, runPut (putBlock b))
        encodeFinRec fr = (MessageFinalizationRecord, encode fr)
        -- Note: since the returned list can be truncated, we have to be careful about the
        -- order that finalization records are interleaved with blocks.
        -- Specifically, we send a finalization record as soon as possible after
        -- the corresponding block; and where the block is not being sent, we
        -- send the finalization record before all other blocks.  We also send
        -- finalization records and blocks in order.
        go :: [(FinalizationRecord, BlockHeight)] -> Seq.Seq (BlockPointerType m) -> [(MessageType, ByteString)]
        go [] bs = encodeBlock <$> toList bs
        go fs Seq.Empty = encodeFinRec <$> toList fs
        go fs0@((f, fh) : fs1) bs0@(b Seq.:<| bs1)
            | fh < bpHeight b = encodeFinRec f : go fs1 bs0
            | otherwise = encodeBlock b : go fs0 bs1

doHandleCatchUp :: forall m. (TreeStateMonad m, SkovQueryMonad m, FinalizationMonad m, LoggerMonad m)
                => CatchUpStatus
                -> Int -- ^How many blocks + finalization records should be sent.
                -> m (Maybe ([(MessageType, ByteString)], CatchUpStatus), UpdateResult)
doHandleCatchUp peerCUS limit = do
        let resultDoCatchUp = if cusIsResponse peerCUS then ResultPendingBlock else ResultContinueCatchUp
        lfb <- lastFinalizedBlock
        if cusLastFinalizedHeight peerCUS > bpHeight lfb then do
            -- Our last finalized height is below the peer's last finalized height
            response <-
                if cusIsRequest peerCUS then do
                    myCUS <- doGetCatchUpStatus False
                    return $ Just ([], myCUS {cusIsResponse = True})
                else
                    return Nothing
            -- We are behind, so we mark the peer as pending, unless it is in progress
            -- and the message is not a response.
            return (response, if cusIsResponse peerCUS then ResultPendingBlock else ResultContinueCatchUp)
        else
            -- Our last finalized height is at least the peer's last finalized height
            -- Check if the peer's last finalized block is recognised
            getBlockStatus (cusLastFinalizedBlock peerCUS) >>= \case
                Just (BlockFinalized peerFinBP peerFinRec) -> do
                    -- Determine if we need to catch up: i.e. if the peer has some
                    -- leaf block we do not recognise.
                    let
                        testLeaves [] = return False
                        testLeaves (l:ls) = resolveBlock l >>= \case
                            Nothing -> return True
                            Just _ -> testLeaves ls
                    catchUpWithPeer <- testLeaves (cusLeaves peerCUS)
                    let catchUpResult = if catchUpWithPeer then resultDoCatchUp else ResultSuccess

                    if cusIsRequest peerCUS then do
                        -- Get the unsettled finalization records
                        frs0 <- finalizationUnsettledRecords (finalizationIndex peerFinRec)
                        -- and filter for only the ones where we know the block, recording the height of the block.
                        -- (This should mean dropping at most one finalization record, since we can only validate
                        -- the next finalization record if we know the previously finalised block, and all of these
                        -- finalization records will have been validated.)
                        let resolveFinRecs Seq.Empty = return []
                            resolveFinRecs (fr Seq.:<| frs) =
                                resolveBlock (finalizationBlockPointer fr) >>= \case
                                    Nothing -> return []
                                    Just fb -> ((fr, bpHeight fb) :) <$> resolveFinRecs frs
                        frs <- resolveFinRecs frs0
                        let peerKnownBlocks = Set.insert (cusLastFinalizedBlock peerCUS) $
                                Set.fromList (cusLeaves peerCUS) `Set.union` Set.fromList (cusBranches peerCUS)
                        let
                            -- get the next batch of finalized blocks (from given finalized index)
                            nextBatch :: FinalizationIndex -> BlockHash -> m (Seq.Seq (BlockPointerType m))
                            nextBatch idx finBh = do
                              getFinalizedAtIndex (idx + 1) >>= \case
                                Nothing -> return Seq.Empty
                                Just bpFin ->
                                   let loop :: Seq.Seq (BlockPointerType m) -> BlockPointerType m -> m (Seq.Seq (BlockPointerType m))
                                       -- extra precaution for genesis block, although this can only really be reachable if
                                       -- the baker's databases are corrupted/don't maintain the right invariants.
                                       loop acc bp | bpHash bp == finBh || bpHeight bp == 0 = return acc
                                                   | otherwise = do
                                          parent <- bpParent bp
                                          loop (bp Seq.<| acc) parent
                                   in loop Seq.Empty bpFin

                            -- extend branches forward from a given finalized block.
                            -- this finalized block is assumed to be the finalized block at the given finalization index
                            extendForwardBranches acc idx finBh | Seq.length acc >= limit = return acc
                                                                | otherwise = do
                              nextBatch idx finBh >>= \case
                                Seq.Empty -> return acc
                                bps@(_ Seq.:|> b) -> do
                                    extendForwardBranches (acc <> bps) (idx + 1) (bpHash b)

                        -- get the last block known to the peer that __we__ know is finalized
                        -- NOTE: This is potentially problematic since somebody could make us waste a lot of effort
                        -- if the given lists (leaves and branches) are big.
                        (peerLastKnownFinalizedBlock, peerLastKnownFinRec) <-
                          foldM (\acc@(curBP, _) bh ->
                                   getBlockStatus bh >>= \case
                                     Just (BlockFinalized finBP finFR) ->
                                       if bpHeight finBP > bpHeight curBP then return (finBP, finFR) else return acc
                                     _ -> return acc
                                )
                                (peerFinBP, peerFinRec)
                                (cusLeaves peerCUS <> cusBranches peerCUS)

                        unknownFinTrunk <- extendForwardBranches Seq.Empty (finalizationIndex peerLastKnownFinRec) (bpHash peerLastKnownFinalizedBlock)

                        -- Take the branches; filter out all blocks that the client claims knowledge of; extend branches back
                        -- to include finalized blocks until the parent is known.
                        myBranches <- getBranches
                        let
                            -- Filter out blocks that are known to the peer
                            filterUnknown :: [BlockPointerType m] -> [BlockPointerType m]
                            filterUnknown = filter ((`Set.notMember` peerKnownBlocks) . bpHash)
                            -- Given a branches structure, filter out the blocks known to the peer and split off
                            -- the oldest block (not known to the peer).  These are returned as two sequences
                            filterTakeOldest Seq.Empty = (Seq.Empty, Seq.Empty)
                            filterTakeOldest (l Seq.:<| bs)
                                | null l' = filterTakeOldest bs
                                | otherwise = filterTakeOldest' m (Seq.singleton $ List.delete m l') (Seq.singleton l') bs
                                where
                                    l' = filterUnknown l
                                    m = minimumBy (compare `on` bpArriveTime) l'
                            filterTakeOldest' oldest sansOldest _ Seq.Empty = (Seq.singleton oldest, sansOldest)
                            filterTakeOldest' oldest sansOldest withOldest r@(l Seq.:<| bs)
                                -- If all blocks at this level are no older than the oldest block so far,
                                -- then that is the oldest block, and we can just filter the remaining blocks
                                | all (\b -> bpArriveTime b >= bpArriveTime oldest) l = (Seq.singleton oldest, sansOldest <> fmap filterUnknown r)
                                -- Otherwise, there could be an older block
                                | otherwise = filterTakeOldest' newOldest newSansOldest (withOldest Seq.|> l') bs
                                    where
                                        l' = filterUnknown l
                                        m = minimumBy (compare `on` bpArriveTime) l'
                                        (newOldest, newSansOldest) = if null l' || bpArriveTime oldest <= bpArriveTime m
                                            then (oldest, sansOldest Seq.|> l')
                                            else (m, withOldest Seq.|> List.delete m l')
                            (outBlocks1, branches1) = case unknownFinTrunk of
                                (b Seq.:<| bs) -> (Seq.singleton b, fmap (:[]) bs <> fmap filterUnknown myBranches)
                                Seq.Empty -> filterTakeOldest myBranches
                            trim = Seq.dropWhileR null
                            takeBranches :: Seq.Seq (BlockPointerType m) -> Seq.Seq [BlockPointerType m] -> m (Seq.Seq (BlockPointerType m))
                            takeBranches out (trim -> brs) = bestBlockOf brs >>= \case
                                Nothing -> return out
                                Just bb -> (out <>) <$> innerLoop Seq.empty brs Seq.empty bb
                            innerLoop out Seq.Empty brs _ = takeBranches out brs
                            innerLoop out brsL0@(brsL Seq.:|> bs) brsR bb
                                | bb `elem` bs = innerLoop (bb Seq.<| out) brsL (List.delete bb bs Seq.<| brsR) =<< bpParent bb
                                | otherwise = takeBranches out (brsL0 <> brsR)
                        -- if finalized blocks alone are enough to fill in the request take that, otherwise also include branches
                        -- Since branches are always kept in memory at this point processing them is cheap, hence we don't
                        -- bound them.
                        outBlocks2 <- if Seq.length unknownFinTrunk <= limit then takeBranches outBlocks1 branches1 else return unknownFinTrunk
                        lvs <- leavesBranches $ toList myBranches
                        myCUS <- makeCatchUpStatus False True lfb (fst lvs) []
                        return (Just (merge (Proxy @ m) frs outBlocks2, myCUS), catchUpResult)
                    else
                        -- No response required
                        return (Nothing, catchUpResult)
                _ -> do
                    -- If the peer's last finalized block is not known to be finalized (to us)
                    -- then we must effectively be on different finalized chains, so we should
                    -- reject this peer.
                    logEvent Skov LLWarning $ "Invalid catch up status: last finalized block not finalized."
                    return (Nothing, ResultInvalid)
