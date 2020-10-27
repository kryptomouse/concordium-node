{-# LANGUAGE TemplateHaskell,
             OverloadedStrings,
             BangPatterns #-}
{-|
Module      : Concordium.GlobalState.Basic.BlockState.AccountReleaseSchedule
Description : The data structure implementing account lock ups.

This module defines a data structure that stores the amounts that are locked
up for a given account.

Amounts are stored in a map sorted by timestamp of release.
-}
module Concordium.GlobalState.Basic.BlockState.AccountReleaseSchedule where

import Data.Map (Map)
import qualified Data.Map as Map
import Lens.Micro.Platform
import Concordium.Types
import Data.Foldable
import Data.Serialize
import Data.Aeson (ToJSON)
import qualified Data.Aeson as AE
import Concordium.Utils.Serialization
import Data.Maybe

-- | Contains the amounts that are locked for a given account as well as
-- their release dates.
data AccountReleaseSchedule = AccountReleaseSchedule {
  -- | The priority queue with the locked amounts
  _pendingReleases :: !(Map Timestamp Amount),
  -- | The total amount that is locked for this account
  _totalLockedUpBalance :: !Amount
  } deriving (Show, Eq)
makeLenses ''AccountReleaseSchedule

instance ToJSON AccountReleaseSchedule where
  toJSON AccountReleaseSchedule{..} =
    AE.object ["total" AE..= _totalLockedUpBalance,
               "schedule" AE..= Map.toAscList _pendingReleases]

instance Serialize AccountReleaseSchedule where
  get = do
    _pendingReleases <- getSafeMapOf get get
    let _totalLockedUpBalance = foldl' (+) 0 _pendingReleases
    return AccountReleaseSchedule{..}
  put AccountReleaseSchedule{..} =
    putSafeMapOf putWord64be put put _pendingReleases

-- | Create an empty account release schedule
emptyAccountReleaseSchedule :: AccountReleaseSchedule
emptyAccountReleaseSchedule = AccountReleaseSchedule Map.empty 0

-- | Add a list of amounts to this @AccountReleaseSchedule@.
addReleases :: [(Timestamp, Amount)] -> AccountReleaseSchedule -> AccountReleaseSchedule
addReleases l AccountReleaseSchedule{..} = go _pendingReleases _totalLockedUpBalance l
  where go _pendingReleases _totalLockedUpBalance [] = AccountReleaseSchedule {..}
        go p t ((i, v):xs) =
          let f (Just v') = Just (v' + v) -- if there is another release at this timestamp, sum them
              f Nothing = Just v
              !p' = Map.alter f i p
              !t' = t + v
          in
            go p' t' xs

-- | Remove the amounts up to the given timestamp.
unlockAmountsUntil :: Timestamp -> AccountReleaseSchedule -> (Amount, AccountReleaseSchedule)
unlockAmountsUntil up ars = let !pq = ars ^. pendingReleases
                                (!toRemove, x, !toKeep) = Map.splitLookup up pq -- remove all items in which @timestamp <= up@
                                minusAmount = foldl' (+) (fromMaybe 0 x) toRemove
                            in
                              (minusAmount, ars & pendingReleases .~ toKeep
                                                & totalLockedUpBalance -~ minusAmount)
