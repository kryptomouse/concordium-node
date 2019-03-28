{-# LANGUAGE DeriveGeneric #-}
{-# LANGUAGE LambdaCase #-}
{-# Language OverloadedStrings #-}
{-# LANGUAGE GeneralizedNewtypeDeriving #-}
module Concordium.Payload.Transaction where

import GHC.Generics
import Data.Word
import Data.ByteString.Char8(ByteString)
import Data.ByteString.Builder
import qualified Data.ByteString.Lazy.Char8 as LBS
import Concordium.Crypto.SHA256
import Data.Serialize
import Data.Hashable
import Data.Bits

import Data.Foldable(toList)
import qualified Data.HashMap.Strict as Map


import Concordium.GlobalState.Types
import Acorn.Types(Message(..))
import qualified Acorn.Types as Types
import qualified Acorn.EnvironmentImplementation as Types
import qualified Acorn.Utils.Init.Example as Init
import qualified Acorn.Scheduler as Sch

newtype TransactionNonce = TransactionNonce Hash
    deriving (Eq, Ord, Hashable, Generic)

instance Show TransactionNonce where
    show (TransactionNonce s) = show s

instance Serialize TransactionNonce

data Transaction = Transaction {
    transactionNonce :: TransactionNonce,
    transactionHeader :: !Header,
    transactionPayload :: !SerializedPayload
} deriving (Generic)

type GlobalState = Types.GlobalState

instance Message Transaction where
  getHeader = transactionHeader
  getPayload = decode . _spayload . transactionPayload

instance Serialize Transaction

instance Show Transaction where
    showsPrec l (Transaction txnonce meta payload) = showsPrec l txnonce . (':':) . showsPrec l meta . (':':) . showsPrec l payload

toTransactions :: ByteString -> Maybe [Transaction]
toTransactions bs = case decode bs of
        Left _ -> Nothing
        Right r -> Just r

fromTransactions :: [Transaction] -> ByteString
fromTransactions = encode . toList

executeBlockForState :: [Transaction] -> ChainMetadata -> Types.GlobalState -> Either Types.FailureKind Types.GlobalState
executeBlockForState txs cm gs = let (mres, gs') = Types.runSI (Sch.execBlock txs) cm gs
                                 in case mres of
                                      Nothing -> Right gs'
                                      Just fk -> Left fk
                              
makeBlock :: [Transaction] -> ChainMetadata -> Types.GlobalState -> ([(Transaction, Types.ValidResult)], [(Transaction, Types.FailureKind)], Types.GlobalState)
makeBlock msg cm gs = let ((suc, failure), gs') = Types.runSI (Sch.makeValidBlock msg) cm gs
                      in (suc, failure, gs')

initState :: Int -> Types.GlobalState
initState n = (Init.initialState n) { Types.accounts = (Map.fromList [("Mateusz", Types.Account "Mateusz" 1 (2 ^ (62 :: Int)))
                                                                     ,("Ales", Types.Account "Ales" 1 100000)
                                                                     ,("Thomas", Types.Account "Thomas" 1 100000)]) }
