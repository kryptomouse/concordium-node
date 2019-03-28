{-# LANGUAGE DeriveGeneric #-}
{-# LANGUAGE OverloadedStrings #-}
{-# LANGUAGE RecordWildCards #-}
{-# LANGUAGE LambdaCase #-}
module Concordium.Getters where

import GHC.Generics(Generic)

import Lens.Micro.Platform hiding ((.=))
import Control.Monad.Trans.State

import Concordium.Payload.Transaction
import Concordium.Types as T
import Concordium.MonadImplementation
import Concordium.Kontrol.BestBlock
import Concordium.Skov.Monad
import Concordium.Logger

import Data.IORef
import Text.Read hiding (get)
import Data.Maybe
import qualified Data.Map as Map

-- import Data.String

-- import qualified Data.Map as Map

-- import Data.ByteString.Builder
-- import Data.Monoid((<>))

import Data.Text(Text)
-- import Data.Text.Encoding

import Data.Aeson

import Data.Word

-- import qualified Data.ByteString as BS
-- import qualified Data.ByteString.Lazy as BSL
-- import qualified Data.ByteString.Base16.Lazy as BSL16
-- import qualified Data.ByteString.Base16 as BS16

-- import Debug.Trace

-- import Data.Serialize.Get(getWord64le, runGet)

data TxOut = TxOut {txNonce :: Text -- base16 encoded nonce
                   ,txSender :: Text  -- base16 encoded address
                   ,txAddr :: Text -- base16 encoded address
                   ,txMessage :: Message
                   }
     deriving(Generic, Show)

data Message = Inc | Dec
    deriving(Show, Generic)

instance ToJSON Message
instance FromJSON Message

data BlockInfo = BlockInfo {blockHash :: Text -- base16 encoding 
                           ,blockParent :: Text  -- base 16 encoding
                           ,blockBaker :: Word64          -- id of the baker of the block
                           ,transactions :: [TxOut]       -- list of transactions in this block
                           ,globalState :: [(Text, Text)] -- global state at the end of this block
                           }
                 deriving(Generic)

instance ToJSON TxOut

instance ToJSON BlockInfo

instance FromJSON TxOut

-- transactionToTxOut :: Transaction -> TxOut
-- transactionToTxOut = undefined

-- -- transactionToTxOut (Transaction (TransactionNonce a b c d) meta (Update to msg)) = TxOut {..}
-- --   where txNonce = decodeUtf8 . BSL.toStrict . BSL16.encode . toLazyByteString $ word64LE a <> word64LE b <> word64LE c <> word64LE d
-- --         txSender = decodeUtf8 . BS16.encode . sender $ meta
-- --         txAddr = decodeUtf8 . BS16.encode $ to
-- --         txMessage = msg

-- globalStateToPairs :: GlobalState -> [(Text, Text)]
-- globalStateToPairs = undefined
-- -- Prelude.map (\(addr, State s) -> (decodeUtf8 . BS16.encode $ addr, s)) . Map.toList . instances

-- blockDataToTxOut :: Block -> Maybe [TxOut]
-- blockDataToTxOut b =  Prelude.map transactionToTxOut <$> toTransactions (blockData b)

mkBlockInfo :: BlockPointer -> BlockInfo
mkBlockInfo block = BlockInfo "" "" 0 [] []
--   | bpParent block == block = BlockInfo (fromString $ show $ bpHash block) (fromString $ show $ bpHash block) 0 [] (globalStateToPairs (bpState block)) -- Genesis block
--   | otherwise =
--       let bh = fromString . show $ bpHash block
--           bp = fromString . show $ bpHash (bpParent block)
--           bb = T.blockBaker (bpBlock block)
--       in case blockDataToTxOut (bpBlock block) of
--           Nothing -> BlockInfo bh bp bb [] (globalStateToPairs (bpState block))
--           Just txouts -> BlockInfo bh bp bb txouts (globalStateToPairs (bpState block))

-- bsToNonce :: BS.ByteString -> Maybe TransactionNonce
-- bsToNonce bs = case runGet parseNonce bs of
--                   Left _ -> Nothing
--                   Right x -> Just x
--   where parseNonce = do
--           b1 <- getWord64le
--           b2 <- getWord64le
--           b3 <- getWord64le
--           b4 <- getWord64le
--           return $ TransactionNonce b1 b2 b3 b4

txOutToTransaction :: TxOut -> Maybe Transaction
txOutToTransaction (TxOut nonce sndr addr msg) = undefined
  -- non <- bsToNonce . dec $ nonce
  -- return $ Transaction non (Metadata decSender) (Update decAddr msg)
  -- where dec = fst . BS16.decode . encodeUtf8
  --       decSender = dec sndr
  --       decAddr = dec addr

hsh :: BlockPointer -> String
hsh = show . bpHash

getConsensusStatus :: IORef SkovFinalizationState -> IO Value
getConsensusStatus sfsRef = do
    sfs <- readIORef sfsRef
    runSilentLogger $ flip evalStateT (sfs ^. sfsSkov) $ do
        bb <- bestBlock
        lfb <- lastFinalizedBlock
        return $ object [
                "bestBlock" .= hsh bb,
                "genesisBlock" .= hsh (sfs ^. genesisBlockPointer),
                "lastFinalizedBlock" .= hsh lfb,
                "bestBlockHeight" .= theBlockHeight (bpHeight bb),
                "lastFinalizedBlockHeight" .= theBlockHeight (bpHeight lfb),
                "blocksReceivedCount" .= (sfs ^. statistics . blocksReceivedCount),
                "blockLastReceivedTime" .= (sfs ^. statistics . blockLastReceived),
                "blockReceiveLatencyEMA" .= (sfs ^. statistics . blockReceiveLatencyEMA),
                "blockReceiveLatencyEMSD" .= sqrt (sfs ^. statistics . blockReceiveLatencyEMVar),
                "blockReceivePeriodEMA" .= (sfs ^. statistics . blockReceivePeriodEMA),
                "blockReceivePeriodEMSD" .= (sqrt <$> (sfs ^. statistics . blockReceivePeriodEMVar)),
                "blocksVerifiedCount" .= (sfs ^. statistics . blocksVerifiedCount),
                "blockLastArrivedTime" .= (sfs ^. statistics . blockLastArrive),
                "blockArriveLatencyEMA" .= (sfs ^. statistics . blockArriveLatencyEMA),
                "blockArriveLatencyEMSD" .= sqrt (sfs ^. statistics . blockArriveLatencyEMVar),
                "blockArrivePeriodEMA" .= (sfs ^. statistics . blockArrivePeriodEMA),
                "blockArrivePeriodEMSD" .= (sqrt <$> (sfs ^. statistics . blockArrivePeriodEMVar)),
                "transactionsPerBlockEMA" .= (sfs ^. statistics . transactionsPerBlockEMA),
                "transactionsPerBlockEMSD" .= sqrt (sfs ^. statistics . transactionsPerBlockEMVar)
            ]

getBlockInfo :: IORef SkovFinalizationState -> String -> IO Value
getBlockInfo sfsRef blockHash = case readMaybe blockHash of
        Nothing -> return Null
        Just bh -> do
            sfs <- readIORef sfsRef
            runSilentLogger $ flip evalStateT (sfs ^. sfsSkov) $
                resolveBlock bh >>= \case
                    Nothing -> return Null
                    Just bp -> do
                        let slot = blockSlot (bpBlock bp)
                        slotTime <- getSlotTime slot
                        bfin <- isFinalized bh
                        return $ object [
                            "blockHash" .= hsh bp,
                            "blockParent" .= hsh (bpParent bp),
                            "blockLastFinalized" .= hsh (bpLastFinalized bp),
                            "blockHeight" .= theBlockHeight (bpHeight bp),
                            "blockReceiveTime" .= bpReceiveTime bp,
                            "blockArriveTime" .= bpArriveTime bp,
                            "blockSlot" .= (fromIntegral slot :: Word64),
                            "blockSlotTime" .= slotTime,
                            "blockBaker" .= if slot == 0 then Null else toJSON (T.blockBaker (bpBlock bp)),
                            "finalized" .= bfin,
                            "transactionCount" .= bpTransactionCount bp
                            ]

getAncestors :: IORef SkovFinalizationState -> String -> BlockHeight -> IO Value
getAncestors sfsRef blockHash count = case readMaybe blockHash of
        Nothing -> return Null
        Just bh -> do
            sfs <- readIORef sfsRef
            runSilentLogger $ flip evalStateT (sfs ^. sfsSkov) $
                resolveBlock bh >>= \case
                    Nothing -> return Null
                    Just bp -> do
                        let heightLim = if count > bpHeight bp then 0 else bpHeight bp - count + 1
                        return $ toJSONList $ map hsh $ takeWhile (\b -> bpHeight b >= heightLim) $ iterate bpParent bp
 
getBranches :: IORef SkovFinalizationState -> IO Value
getBranches sfsRef = do
        sfs <- readIORef sfsRef
        runSilentLogger $ flip evalStateT (sfs ^. sfsSkov) $ do
            brs <- branchesFromTop
            let brt = foldl up Map.empty brs
            lastFin <- lastFinalizedBlock
            return $ object ["blockHash" .= hsh lastFin, "children" .= Map.findWithDefault [] lastFin brt]
    where
        up :: Map.Map BlockPointer [Value] -> [BlockPointer] -> Map.Map BlockPointer [Value]
        up childrenMap = foldr (\b -> at (bpParent b) . non [] %~ (object ["blockHash" .= hsh b, "children" .= Map.findWithDefault [] b childrenMap] :)) Map.empty
 