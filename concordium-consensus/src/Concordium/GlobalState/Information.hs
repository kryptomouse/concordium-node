{-# LANGUAGE DeriveGeneric #-}
{-# LANGUAGE RecordWildCards #-}

module Concordium.GlobalState.Information where

import qualified Data.Serialize as S
import GHC.Generics

import qualified Concordium.Types.Acorn.Core as Core
import Concordium.Types.Acorn.Interfaces
import Concordium.Types

-- *Summary of global state to be sent over the network.

data InstanceInfo = InstanceInfo
    {
     messageType :: !(Core.Type Core.ModuleRef)
    ,localState :: !Value  -- must be storable
    ,instanceAmount :: !Amount
    } deriving(Show)

instance S.Serialize InstanceInfo where
  put (InstanceInfo{..}) = S.put messageType <> putStorable localState <> S.put instanceAmount
  get = InstanceInfo <$> S.get <*> getStorable <*> S.get

data AccountInfo = AccountInfo
    {accountNonce :: !Nonce
    ,accountAmount :: !Amount
    } 
    deriving(Show, Generic)

instance S.Serialize AccountInfo
