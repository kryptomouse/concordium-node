{-# LANGUAGE DeriveGeneric #-}
{-# LANGUAGE TypeFamilies #-}
{-# LANGUAGE BangPatterns #-}
{-# LANGUAGE TemplateHaskell #-}
{-# LANGUAGE OverloadedStrings #-}
{-# LANGUAGE ConstraintKinds #-}
{-# LANGUAGE UndecidableInstances #-}
module Concordium.GlobalState.Persistent.BlockState (
    PersistentBlockState,
    BlockStatePointers(..),
    HashedPersistentBlockState(..),
    hashBlockState,
    PersistentModule(..),
    persistentModuleToModule,
    Modules(..),
    emptyModules,
    makePersistentModules,
    PersistentBirkParameters(..),
    makePersistentBirkParameters,
    makePersistent,
    initialPersistentState,
    emptyBlockState,
    fromPersistentInstance,
    PersistentBlockStateContext(..),
    PersistentState,
    PersistentBlockStateMonad(..)
) where

import Data.Serialize
import qualified Data.HashMap.Strict as HM
import Data.IORef
import Control.Monad.Reader.Class
import Control.Monad.Trans
import Control.Monad
import Data.Foldable
import Data.Maybe
import Lens.Micro.Platform
import Concordium.Utils
import qualified Data.Set as Set
import qualified Data.Vector as Vec
import qualified Data.Map.Strict as Map

import GHC.Generics (Generic)

import qualified Concordium.Crypto.SHA256 as H
import Concordium.Types
import Concordium.Types.Execution ( TransactionSummary )
import qualified Concordium.Wasm as Wasm
import qualified Concordium.ID.Types as ID
import qualified Concordium.ID.Parameters as ID
import Concordium.Types.Updates

import Concordium.GlobalState.BakerInfo
import Concordium.GlobalState.Persistent.BlobStore
import qualified Concordium.GlobalState.Persistent.Trie as Trie
import Concordium.GlobalState.BlockState
import Concordium.GlobalState.Parameters
import Concordium.GlobalState.Types
import Concordium.GlobalState.Account hiding (addIncomingEncryptedAmount, addToSelfEncryptedAmount, _stakedAmount, _stakeEarnings, _accountBakerInfo, _bakerPendingChange, stakedAmount, stakeEarnings, accountBakerInfo, bakerPendingChange)
import qualified Concordium.GlobalState.IdentityProviders as IPS
import qualified Concordium.GlobalState.AnonymityRevokers as ARS
import qualified Concordium.GlobalState.Rewards as Rewards
import qualified Concordium.GlobalState.Persistent.Accounts as Accounts
import Concordium.GlobalState.Persistent.Bakers
import qualified Concordium.GlobalState.Persistent.Instances as Instances
import qualified Concordium.Types.Transactions as Transactions
import qualified Concordium.Types.Execution as Transactions
import Concordium.GlobalState.Persistent.Instances(PersistentInstance(..), PersistentInstanceParameters(..), CacheableInstanceParameters(..))
import Concordium.GlobalState.Instance (Instance(..),InstanceParameters(..),makeInstanceHash')
import Concordium.GlobalState.Persistent.Account
import Concordium.GlobalState.Persistent.BlockState.Updates
import qualified Concordium.GlobalState.Basic.BlockState.Account as TransientAccount
import qualified Concordium.GlobalState.Basic.BlockState as Basic
import qualified Concordium.GlobalState.Basic.BlockState.Updates as Basic
import qualified Concordium.GlobalState.Modules as TransientMods
import Concordium.GlobalState.SeedState
import Concordium.Logger (MonadLogger)
import Concordium.Types.HashableTo

import qualified Concordium.GlobalState.Basic.BlockState.AccountReleaseSchedule as TransientReleaseSchedule

type PersistentBlockState = IORef (BufferedRef BlockStatePointers)

data BlockStatePointers = BlockStatePointers {
    bspAccounts :: !Accounts.Accounts,
    bspInstances :: !Instances.Instances,
    bspModules :: !(HashedBufferedRef Modules),
    bspBank :: !(Hashed Rewards.BankStatus),
    bspIdentityProviders :: !(HashedBufferedRef IPS.IdentityProviders),
    bspAnonymityRevokers :: !(HashedBufferedRef ARS.AnonymityRevokers),
    bspBirkParameters :: !PersistentBirkParameters,
    bspCryptographicParameters :: !(HashedBufferedRef CryptographicParameters),
    bspUpdates :: !(BufferedRef Updates),
    bspReleaseSchedule :: !(BufferedRef (Map.Map AccountAddress Timestamp)),
    -- FIXME: Store transaction outcomes in a way that allows for individual indexing.
    bspTransactionOutcomes :: !Transactions.TransactionOutcomes
}

data HashedPersistentBlockState = HashedPersistentBlockState {
    hpbsPointers :: !PersistentBlockState,
    hpbsHash :: !StateHash
}

hashBlockState :: MonadBlobStore m => PersistentBlockState -> m HashedPersistentBlockState
hashBlockState hpbsPointers = do
        rbsp <- liftIO $ readIORef hpbsPointers
        bsp <- refLoad rbsp
        hpbsHash <- getHashM bsp
        return HashedPersistentBlockState{..}

instance MonadBlobStore m => MHashableTo m StateHash BlockStatePointers where
    getHashM BlockStatePointers{..} = do
        bshBirkParameters <- getHashM bspBirkParameters
        bshCryptographicParameters <- getHashM bspCryptographicParameters
        bshIdentityProviders <- getHashM bspIdentityProviders
        bshAnonymityRevokers <- getHashM bspAnonymityRevokers
        bshModules <- getHashM bspModules
        let bshBankStatus = getHash bspBank
        bshAccounts <- getHashM bspAccounts
        bshInstances <- getHashM bspInstances
        bshUpdates <- getHashM bspUpdates
        return $ makeBlockStateHash BlockStateHashInputs{..}

instance (MonadBlobStore m, BlobStorable m (Nullable (BlobRef Accounts.RegIdHistory))) => BlobStorable m BlockStatePointers where
    storeUpdate bsp0@BlockStatePointers{..} = do
        (paccts, bspAccounts') <- storeUpdate bspAccounts
        (pinsts, bspInstances') <- storeUpdate bspInstances
        (pmods, bspModules') <- storeUpdate bspModules
        (pips, bspIdentityProviders') <- storeUpdate bspIdentityProviders
        (pars, bspAnonymityRevokers') <- storeUpdate bspAnonymityRevokers
        (pbps, bspBirkParameters') <- storeUpdate bspBirkParameters
        (pcryptps, bspCryptographicParameters') <- storeUpdate bspCryptographicParameters
        (pupdates, bspUpdates') <- storeUpdate bspUpdates
        (preleases, bspReleaseSchedule') <- storeUpdate bspReleaseSchedule
        let putBSP = do
                paccts
                pinsts
                pmods
                put $ _unhashed bspBank
                pips
                pars
                pbps
                pcryptps
                put bspTransactionOutcomes
                pupdates
                preleases
        return (putBSP, bsp0 {
                    bspAccounts = bspAccounts',
                    bspInstances = bspInstances',
                    bspModules = bspModules',
                    bspIdentityProviders = bspIdentityProviders',
                    bspAnonymityRevokers = bspAnonymityRevokers',
                    bspBirkParameters = bspBirkParameters',
                    bspCryptographicParameters = bspCryptographicParameters',
                    bspUpdates = bspUpdates',
                    bspReleaseSchedule = bspReleaseSchedule'
                })
    store bsp = fst <$> storeUpdate bsp
    load = do
        maccts <- label "Accounts" load
        minsts <- label "Instances" load
        mmods <- label "Modules" load
        bspBank <- makeHashed <$> label "Bank" get
        mpips <- label "Identity providers" load
        mars <- label "Anonymity revokers" load
        mbps <- label "Birk parameters" load
        mcryptps <- label "Cryptographic parameters" load
        bspTransactionOutcomes <- label "Transaction outcomes" get
        mUpdates <- label "Updates" load
        mReleases <- label "Release schedule" load
        return $! do
            bspAccounts <- maccts
            bspInstances <- minsts
            bspModules <- mmods
            bspIdentityProviders <- mpips
            bspAnonymityRevokers <- mars
            bspBirkParameters <- mbps
            bspCryptographicParameters <- mcryptps
            bspUpdates <- mUpdates
            bspReleaseSchedule <- mReleases
            return $! BlockStatePointers{..}

instance MonadBlobStore m => Cacheable m BlockStatePointers where
    cache BlockStatePointers{..} = do
        accts <- cache bspAccounts
        insts <- cache bspInstances
        mods <- cache bspModules
        ips <- cache bspIdentityProviders
        ars <- cache bspAnonymityRevokers
        birkParams <- cache bspBirkParameters
        cryptoParams <- cache bspCryptographicParameters
        upds <- cache bspUpdates
        rels <- cache bspReleaseSchedule
        return BlockStatePointers{
            bspAccounts = accts,
            bspInstances = insts,
            bspModules = mods,
            bspBank = bspBank,
            bspIdentityProviders = ips,
            bspAnonymityRevokers = ars,
            bspBirkParameters = birkParams,
            bspCryptographicParameters = cryptoParams,
            bspUpdates = upds,
            bspReleaseSchedule = rels,
            bspTransactionOutcomes = bspTransactionOutcomes
        }

data PersistentModule = PersistentModule {
    pmInterface :: !Wasm.ModuleInterface,
    pmIndex :: !ModuleIndex
}

persistentModuleToModule :: PersistentModule -> Module
persistentModuleToModule PersistentModule{..} = Module {
    moduleInterface = pmInterface,
    moduleIndex = pmIndex
}

instance Serialize PersistentModule where
    put PersistentModule{..} = put pmInterface <> put pmIndex
    get = PersistentModule <$> get <*> get

instance MonadBlobStore m => BlobStorable m PersistentModule
instance Applicative m => Cacheable m PersistentModule

data Modules = Modules {
    modules :: Trie.TrieN (BufferedBlobbed BlobRef) ModuleRef PersistentModule,
    nextModuleIndex :: !ModuleIndex,
    runningHash :: !H.Hash
}

emptyModules :: Modules
emptyModules = Modules {
        modules = Trie.empty,
        nextModuleIndex = 0,
        runningHash = H.hash ""
    }

instance (MonadBlobStore m) => BlobStorable m Modules where
    storeUpdate ms@Modules{..} = do
        (pm, modules') <- storeUpdate modules
        return (pm >> put nextModuleIndex >> put runningHash, ms {modules = modules'})
    store m = fst <$> storeUpdate m
    load = do
        mmodules <- load
        nextModuleIndex <- get
        runningHash <- get
        return $ do
            modules <- mmodules
            return Modules{..}

instance HashableTo H.Hash Modules where
  getHash Modules {..} = runningHash

instance Monad m => MHashableTo m H.Hash Modules

instance (MonadBlobStore m) => Cacheable m Modules where
    cache m = do
        modules' <- cache (modules m)
        return m{modules = modules'}

makePersistentModules :: MonadIO m => TransientMods.Modules -> m Modules
makePersistentModules TransientMods.Modules{..} = do
    m' <- Trie.fromList $ upd <$> HM.toList _modules
    return $ Modules m' _nextModuleIndex _runningHash
    where
        upd (mref, Module{..}) = (mref, PersistentModule{
                pmInterface = moduleInterface,
                pmIndex = moduleIndex
            })

data PersistentBirkParameters = PersistentBirkParameters {
    -- |The currently-registered bakers.
    _birkActiveBakers :: !(BufferedRef PersistentActiveBakers),
    -- |The bakers that will be used for the next epoch.
    _birkNextEpochBakers :: !(HashedBufferedRef PersistentEpochBakers),
    -- |The bakers for the current epoch.
    _birkCurrentEpochBakers :: !(HashedBufferedRef PersistentEpochBakers),
    -- |The seed state used to derive the leadership election nonce.
    _birkSeedState :: !SeedState
} deriving (Generic, Show)

makeLenses ''PersistentBirkParameters

instance MonadBlobStore m => MHashableTo m H.Hash PersistentBirkParameters where
  getHashM PersistentBirkParameters {..} = do
    nextHash <- getHashM _birkNextEpochBakers
    currentHash <- getHashM _birkCurrentEpochBakers
    let bpH0 = H.hash $ "SeedState" <> encode _birkSeedState
        bpH1 = H.hashOfHashes nextHash currentHash
    return $ H.hashOfHashes bpH0 bpH1

instance MonadBlobStore m => BlobStorable m PersistentBirkParameters where
    storeUpdate bps@PersistentBirkParameters{..} = do
        (pabs, actBakers) <- storeUpdate _birkActiveBakers
        (pnebs, nextBakers) <- storeUpdate _birkNextEpochBakers
        (pcebs, currentBakers) <- storeUpdate _birkCurrentEpochBakers
        let putBSP = do
                pabs
                pnebs
                pcebs
                put _birkSeedState
        return (putBSP, bps {
                    _birkActiveBakers = actBakers,
                    _birkNextEpochBakers = nextBakers,
                    _birkCurrentEpochBakers = currentBakers
                })
    store bps = fst <$> storeUpdate bps
    load = do
        mabs <- label "Active bakers" $ load
        mnebs <- label "Next epoch bakers" $ load
        mcebs <- label "Current epoch bakers" $ load
        _birkSeedState <- label "Seed state" get
        return $! do
            _birkActiveBakers <- mabs
            _birkNextEpochBakers <- mnebs
            _birkCurrentEpochBakers <- mcebs
            return PersistentBirkParameters{..}

instance MonadBlobStore m => Cacheable m PersistentBirkParameters where
    cache PersistentBirkParameters{..} = do
        active <- cache _birkActiveBakers
        next <- cache _birkNextEpochBakers
        cur <- cache _birkCurrentEpochBakers
        return PersistentBirkParameters{
            _birkActiveBakers = active,
            _birkNextEpochBakers = next,
            _birkCurrentEpochBakers = cur,
            ..
        }

makePersistentBirkParameters :: MonadBlobStore m => Basic.BasicBirkParameters -> m PersistentBirkParameters
makePersistentBirkParameters bbps = do
    _birkActiveBakers <- refMake =<< makePersistentActiveBakers (Basic._birkActiveBakers bbps)
    _birkNextEpochBakers <- refMake =<< makePersistentEpochBakers (_unhashed (Basic._birkNextEpochBakers bbps))
    _birkCurrentEpochBakers <- refMake =<< makePersistentEpochBakers (_unhashed (Basic._birkCurrentEpochBakers bbps))
    let _birkSeedState = Basic._birkSeedState bbps
    return $ PersistentBirkParameters{..}

makePersistent :: MonadBlobStore m  => Basic.BlockState -> m HashedPersistentBlockState
makePersistent Basic.BlockState{..} = do
  persistentBlockInstances <- Instances.makePersistent _blockInstances
  persistentBirkParameters <- makePersistentBirkParameters _blockBirkParameters
  persistentMods <- makePersistentModules _blockModules
  modules <- refMake persistentMods
  identityProviders <- bufferHashed _blockIdentityProviders
  anonymityRevokers <- bufferHashed _blockAnonymityRevokers
  cryptographicParameters <- bufferHashed _blockCryptographicParameters
  blockAccounts <- Accounts.makePersistent _blockAccounts
  updates <- makeBufferedRef =<< makePersistentUpdates _blockUpdates
  rels <- makeBufferedRef _blockReleaseSchedule
  bsp <-
    makeBufferedRef $
      BlockStatePointers
        { bspAccounts = blockAccounts,
          bspInstances = persistentBlockInstances,
          bspModules = modules,
          bspBank = _blockBank,
          bspIdentityProviders = identityProviders,
          bspAnonymityRevokers = anonymityRevokers,
          bspBirkParameters = persistentBirkParameters,
          bspCryptographicParameters = cryptographicParameters,
          bspTransactionOutcomes = _blockTransactionOutcomes,
          bspUpdates = updates,
          bspReleaseSchedule = rels
        }
  bps <- liftIO $ newIORef $! bsp
  hashBlockState bps

initialPersistentState :: MonadBlobStore m => SeedState
             -> CryptographicParameters
             -> [TransientAccount.Account]
             -> IPS.IdentityProviders
             -> ARS.AnonymityRevokers
             -> Amount
             -> Authorizations
             -> ChainParameters
             -> m HashedPersistentBlockState
initialPersistentState ss cps accts ips ars amt auths chainParams = makePersistent $ Basic.initialState ss cps accts ips ars amt auths chainParams

-- |Mostly empty block state, apart from using 'Rewards.genesisBankStatus' which
-- has hard-coded initial values for amount of gtu in existence.
emptyBlockState :: MonadBlobStore m => PersistentBirkParameters -> CryptographicParameters -> Authorizations -> ChainParameters -> m PersistentBlockState
emptyBlockState bspBirkParameters cryptParams auths chainParams = do
  modules <- refMake emptyModules
  identityProviders <- refMake IPS.emptyIdentityProviders
  anonymityRevokers <- refMake ARS.emptyAnonymityRevokers
  cryptographicParameters <- refMake cryptParams
  bspUpdates <- refMake =<< initialUpdates auths chainParams
  bspReleaseSchedule <- refMake $ Map.empty
  bsp <- makeBufferedRef $ BlockStatePointers
          { bspAccounts = Accounts.emptyAccounts,
            bspInstances = Instances.emptyInstances,
            bspModules = modules,
            bspBank = makeHashed Rewards.emptyBankStatus,
            bspIdentityProviders = identityProviders,
            bspAnonymityRevokers = anonymityRevokers,
            bspCryptographicParameters = cryptographicParameters,
            bspTransactionOutcomes = Transactions.emptyTransactionOutcomes,
            ..
          }
  liftIO $ newIORef $! bsp

fromPersistentInstance ::  MonadBlobStore m =>
    PersistentBlockState -> Instances.PersistentInstance -> m Instance
fromPersistentInstance _ Instances.PersistentInstance{pinstanceCachedParameters = (Some CacheableInstanceParameters{..}), ..} = do
    PersistentInstanceParameters{..} <- loadBufferedRef pinstanceParameters
    let instanceParameters = InstanceParameters {
            instanceAddress = pinstanceAddress,
            instanceOwner = pinstanceOwner,
            instanceContractModule = pinstanceContractModule,
            instanceInitName = pinstanceInitName,
            instanceReceiveFuns = pinstanceReceiveFuns,
            instanceModuleInterface = pinstanceModuleInterface,
            instanceParameterHash = pinstanceParameterHash
        }
    return Instance{ instanceModel = pinstanceModel,
            instanceAmount = pinstanceAmount,
            instanceHash = pinstanceHash,
            ..
         }
fromPersistentInstance pbs Instances.PersistentInstance{..} = do
    PersistentInstanceParameters{..} <- loadBufferedRef pinstanceParameters
    doGetModule pbs pinstanceContractModule >>= \case
        Nothing -> error "fromPersistentInstance: unresolvable module" -- TODO: Possibly don't error here
        Just m -> do
            let instanceParameters = InstanceParameters {
                    instanceAddress = pinstanceAddress,
                    instanceOwner = pinstanceOwner,
                    instanceContractModule = pinstanceContractModule,
                    instanceInitName = pinstanceInitName,
                    instanceReceiveFuns = pinstanceReceiveFuns,
                    instanceModuleInterface = moduleInterface m,
                    instanceParameterHash = pinstanceParameterHash
                }
            return Instance{
                    instanceModel = pinstanceModel,
                    instanceAmount = pinstanceAmount,
                    instanceHash = pinstanceHash,
                    ..
                }

loadPBS :: MonadBlobStore m => PersistentBlockState -> m BlockStatePointers
loadPBS = loadBufferedRef <=< liftIO . readIORef
{-# INLINE loadPBS #-}

storePBS :: MonadBlobStore m => PersistentBlockState -> BlockStatePointers -> m PersistentBlockState
storePBS pbs bsp = liftIO $ do
    pbsp <- makeBufferedRef bsp
    writeIORef pbs pbsp
    return pbs
{-# INLINE storePBS #-}

doGetModule :: MonadBlobStore m => PersistentBlockState -> ModuleRef -> m (Maybe Module)
doGetModule s modRef = do
    bsp <- loadPBS s
    mods <- refLoad (bspModules bsp)
    fmap persistentModuleToModule <$> Trie.lookup modRef (modules mods)

doGetModuleList :: MonadBlobStore m => PersistentBlockState -> m [ModuleRef]
doGetModuleList s = do
    bsp <- loadPBS s
    mods <- refLoad (bspModules bsp)
    Trie.keys (modules mods)

doPutNewModule :: MonadBlobStore m =>PersistentBlockState
    -> Wasm.ModuleInterface
    -> m (Bool, PersistentBlockState)
doPutNewModule pbs pmInterface = do
        let mref = Wasm.miModuleRef pmInterface
        bsp <- loadPBS pbs
        mods <- refLoad (bspModules bsp)
        let
            newMod = PersistentModule{pmIndex = nextModuleIndex mods, ..}
            tryIns Nothing = return (True, Trie.Insert newMod)
            tryIns (Just _) = return (False, Trie.NoChange)
        (b, modules') <- Trie.adjust tryIns mref (modules mods)
        if b then do
            let
                newMods = mods {modules = modules', nextModuleIndex = nextModuleIndex mods + 1}
            modules <- refMake newMods
            (True,) <$> storePBS pbs (bsp {bspModules = modules})
        else
            return (False, pbs)

doGetSeedState :: MonadBlobStore m => PersistentBlockState -> m SeedState
doGetSeedState pbs = _birkSeedState . bspBirkParameters <$> loadPBS pbs

doSetSeedState :: MonadBlobStore m => PersistentBlockState -> SeedState -> m PersistentBlockState
doSetSeedState pbs ss = do
        bsp <- loadPBS pbs
        storePBS pbs bsp{bspBirkParameters = (bspBirkParameters bsp){_birkSeedState = ss}}

doGetCurrentEpochBakers :: MonadBlobStore m => PersistentBlockState -> m FullBakers
doGetCurrentEpochBakers pbs = epochToFullBakers =<< refLoad . _birkCurrentEpochBakers . bspBirkParameters =<< loadPBS pbs

doGetSlotBakers :: MonadBlobStore m => PersistentBlockState -> Slot -> m FullBakers
doGetSlotBakers pbs slot = do
        bs <- loadPBS pbs
        let
            bps = bspBirkParameters bs
            SeedState{..} = bps ^. birkSeedState
            slotEpoch = fromIntegral $ slot `quot` epochLength
        case compare slotEpoch (epoch + 1) of
            LT -> epochToFullBakers =<< refLoad (bps ^. birkCurrentEpochBakers)
            EQ -> epochToFullBakers =<< refLoad (bps ^. birkNextEpochBakers)
            GT -> do
                activeBids <- Trie.keysAsc . _activeBakers =<< refLoad (bps ^. birkActiveBakers)
                let resolveBaker (BakerId aid) = Accounts.indexedAccount aid (bspAccounts bs) >>= \case
                        Just acct -> case acct ^. accountBaker of
                            Some bkr -> do
                                pab <- refLoad bkr
                                abi <- refLoad (pab ^. accountBakerInfo)
                                return $ case _bakerPendingChange pab of
                                    RemoveBaker remEpoch
                                        | remEpoch < slotEpoch -> Nothing
                                    ReduceStake newAmt redEpoch
                                        | redEpoch < slotEpoch -> Just (FullBakerInfo abi newAmt)
                                    _ -> Just (FullBakerInfo abi (pab ^. stakedAmount))
                            Null -> error "Persistent.getSlotBakers invariant violation: active baker account not a baker"
                        Nothing -> error "Persistent.getSlotBakers invariant violation: active baker account not valid"
                futureBakers <- Vec.fromList . catMaybes <$> mapM resolveBaker activeBids
                return FullBakers {
                    fullBakerInfos = futureBakers,
                    bakerTotalStake = sum (_bakerStake <$> futureBakers)
                }

doGetBakerAccount :: MonadBlobStore m => PersistentBlockState -> BakerId -> m (Maybe PersistentAccount)
doGetBakerAccount pbs (BakerId ai) = do
        bsp <- loadPBS pbs
        Accounts.indexedAccount ai (bspAccounts bsp)

doTransitionEpochBakers :: MonadBlobStore m => PersistentBlockState -> Epoch -> m PersistentBlockState
doTransitionEpochBakers pbs newEpoch = do
        bsp <- loadPBS pbs
        let oldBPs = bspBirkParameters bsp
        curActiveBIDs <- Trie.keysAsc . _activeBakers =<< refLoad (_birkActiveBakers oldBPs)
        -- Retrieve/update the baker info
        let accumBakers (bs0, bkrs0) bkr@(BakerId aid) = Accounts.indexedAccount aid (bspAccounts bsp) >>= \case
                Just PersistentAccount{_accountBaker = Some acctBkrRef} -> do
                    acctBkr <- refLoad acctBkrRef
                    case _bakerPendingChange acctBkr of
                        RemoveBaker remEpoch
                            -- Removal takes effect next epoch, so exclude it from the list of bakers
                            | remEpoch == newEpoch + 1 -> return (bs0, bkrs0)
                            -- Removal complete, so update the active bakers and account as well
                            | remEpoch <= newEpoch -> do
                                -- Remove the baker from the active bakers
                                curABs <- refLoad (_birkActiveBakers (bspBirkParameters bs0))
                                newAB <- Trie.delete bkr (_activeBakers curABs)
                                abi <- refLoad (_accountBakerInfo acctBkr)
                                newAK <- Trie.delete (_bakerAggregationVerifyKey abi) (_aggregationKeys curABs)
                                newABs <- refMake $ PersistentActiveBakers {
                                        _activeBakers = newAB,
                                        _aggregationKeys = newAK
                                    }
                                -- Remove the baker from the account
                                let updAcc acc = ((),) <$> setPersistentAccountBaker acc Null
                                (_, newAccounts) <- Accounts.updateAccountsAtIndex updAcc aid (bspAccounts bs0)
                                -- The baker is not included for this epoch
                                return (bs0 {
                                        bspBirkParameters = (bspBirkParameters bs0) {_birkActiveBakers = newABs},
                                        bspAccounts = newAccounts
                                    }, bkrs0)
                        ReduceStake newAmt redEpoch
                            -- Reduction takes effect next epoch, so apply it in the generated list
                            | redEpoch == newEpoch + 1 -> return (bs0, (_accountBakerInfo acctBkr, newAmt) : bkrs0)
                            -- Reduction complete, so update the account as well
                            | redEpoch <= newEpoch -> do
                                -- Reduce the baker's stake on the account
                                let newBaker = acctBkr{_stakedAmount = newAmt, _bakerPendingChange = NoChange}
                                let updAcc acc = ((),) <$> setPersistentAccountBaker acc (Some newBaker)
                                (_, newAccounts) <- Accounts.updateAccountsAtIndex updAcc aid (bspAccounts bs0)
                                -- The baker is included with the revised stake
                                return (bs0 {bspAccounts = newAccounts}, (_accountBakerInfo acctBkr, newAmt) : bkrs0)
                        _ -> return (bs0, (_accountBakerInfo acctBkr, _stakedAmount acctBkr) : bkrs0)
                _ -> error "Persistent.bsoTransitionEpochBakers invariant violation: active baker account not a valid baker"
        -- Get the baker info. The list of baker ids is reversed in the input so the accumulated list
        -- is in ascending order.
        (bsp', bkrs) <- foldM accumBakers (bsp, []) (reverse curActiveBIDs)
        newBakerInfos <- refMake . BakerInfos . Vec.fromList $ fst <$> bkrs
        let stakesVec = Vec.fromList $ snd <$> bkrs
        newBakerStakes <- refMake (BakerStakes stakesVec)
        neb <- refLoad (_birkNextEpochBakers oldBPs)
        -- If the baker infos structure has the same hash as the previous one,
        -- use that to avoid duplicate storage.
        _bakerInfos <- secondIfEqual newBakerInfos (_bakerInfos neb)
        -- Also for stakes. This is less likely to be useful, but it's pretty cheap to check,
        -- so why not?
        _bakerStakes <- secondIfEqual newBakerStakes (_bakerStakes neb)
        let _bakerTotalStake = sum stakesVec
        newNextBakers <- refMake PersistentEpochBakers{..}
        let newBirkParams = oldBPs {
            _birkCurrentEpochBakers = _birkNextEpochBakers oldBPs,
            _birkNextEpochBakers = newNextBakers
        }
        storePBS pbs bsp'{bspBirkParameters = newBirkParams}
    where
        secondIfEqual a b = do
            h1 <- getHashM a
            h2 <- getHashM b
            return $ if (h1 :: H.Hash) == h2 then b else a

doAddBaker :: MonadBlobStore m => PersistentBlockState -> AccountAddress -> BakerAdd -> m (BakerAddResult, PersistentBlockState)
doAddBaker pbs aaddr BakerAdd{..} = do
        bsp <- loadPBS pbs
        Accounts.getAccountWithIndex aaddr (bspAccounts bsp) >>= \case
            -- Cannot resolve the account
            Nothing -> return (BAInvalidAccount, pbs)
            -- Account is already a baker
            Just (ai, PersistentAccount{_accountBaker = Some _}) -> return (BAAlreadyBaker (BakerId ai), pbs)
            Just (ai, PersistentAccount{..}) -> do
                    let bid = BakerId ai
                    pab <- refLoad (_birkActiveBakers (bspBirkParameters bsp))
                    let updAgg Nothing = return (True, Trie.Insert ())
                        updAgg (Just ()) = return (False, Trie.NoChange)
                    Trie.adjust updAgg (bkuAggregationKey baKeys) (_aggregationKeys pab) >>= \case
                        -- Aggregation key is a duplicate
                        (False, _) -> return (BADuplicateAggregationKey, pbs)
                        (True, newAggregationKeys) -> do
                            newActiveBakers <- Trie.insert bid () (_activeBakers pab)
                            newpabref <- refMake PersistentActiveBakers{
                                    _aggregationKeys = newAggregationKeys,
                                    _activeBakers = newActiveBakers
                                }
                            let newBirkParams = bspBirkParameters bsp & birkActiveBakers .~ newpabref
                            let updAcc acc = do
                                    newBakerInfo <- refMake (bakerKeyUpdateToInfo bid baKeys)
                                    acc' <- setPersistentAccountBaker acc (Some PersistentAccountBaker{
                                        _stakedAmount = baStake,
                                        _stakeEarnings = baStakeEarnings,
                                        _accountBakerInfo = newBakerInfo,
                                        _bakerPendingChange = NoChange
                                    })
                                    return ((), acc')
                            -- This cannot fail to update the account, since we already looked up the account.
                            (_, newAccounts) <- Accounts.updateAccountsAtIndex updAcc ai (bspAccounts bsp)
                            (BASuccess bid,) <$> storePBS pbs bsp{
                                bspBirkParameters = newBirkParams,
                                bspAccounts = newAccounts
                            }

doUpdateBakerKeys :: MonadBlobStore m => PersistentBlockState -> AccountAddress -> BakerKeyUpdate -> m (BakerKeyUpdateResult, PersistentBlockState)
doUpdateBakerKeys pbs aaddr bku@BakerKeyUpdate{..} = do
        bsp <- loadPBS pbs
        Accounts.getAccountWithIndex aaddr (bspAccounts bsp) >>= \case
            -- The account is valid and has a baker
            Just (ai, PersistentAccount{_accountBaker = Some pAcctBkr}) -> do
                acctBkr <- refLoad pAcctBkr
                pab <- refLoad (_birkActiveBakers (bspBirkParameters bsp))
                bkrInfo <- refLoad (_accountBakerInfo acctBkr)
                -- Try updating the aggregation keys
                (keyOK, newAggregationKeys) <-
                        -- If the aggregation key has not changed, we have nothing to do.
                        if bkuAggregationKey == _bakerAggregationVerifyKey bkrInfo then
                            return (True, _aggregationKeys pab)
                        else do
                            -- Remove the old key
                            ak1 <- Trie.delete (_bakerAggregationVerifyKey bkrInfo) (_aggregationKeys pab)
                            -- Add the new key and check that it is not already present
                            let updAgg Nothing = return (True, Trie.Insert ())
                                updAgg (Just ()) = return (False, Trie.NoChange)
                            Trie.adjust updAgg bkuAggregationKey ak1
                if keyOK then do
                    -- The new aggregation key is known to be unique
                    newActiveBakers <- refMake pab{_aggregationKeys = newAggregationKeys}
                    let newBirkParams = bspBirkParameters bsp & birkActiveBakers .~ newActiveBakers
                    -- Update the account with the new keys
                    let updAcc acc = do
                            newBakerInfo <- refMake (bakerKeyUpdateToInfo (BakerId ai) bku)
                            acc' <- setPersistentAccountBaker acc (Some acctBkr {_accountBakerInfo = newBakerInfo})
                            return ((), acc')
                    (_, newAccounts) <- Accounts.updateAccountsAtIndex updAcc ai (bspAccounts bsp)
                    (BKUSuccess (BakerId ai),) <$> storePBS pbs bsp{
                        bspBirkParameters = newBirkParams,
                        bspAccounts = newAccounts
                    }
                else
                    return (BKUDuplicateAggregationKey, pbs)
            -- Cannot reesolve the account, or it is not a baker
            _ -> return (BKUInvalidBaker, pbs)

doUpdateBakerStake :: MonadBlobStore m => PersistentBlockState -> AccountAddress -> Amount -> m (BakerStakeUpdateResult, PersistentBlockState)
doUpdateBakerStake pbs aaddr newStake = do
        bsp <- loadPBS pbs
        Accounts.getAccountWithIndex aaddr (bspAccounts bsp) >>= \case
            -- The account is valid and has a baker
            Just (ai, PersistentAccount{_accountBaker = Some pAcctBkr}) -> do
                acctBkr <- refLoad pAcctBkr
                if _bakerPendingChange acctBkr /= NoChange
                -- A change is already pending
                then return (BSUChangePending (BakerId ai), pbs)
                -- We can make the change
                else do
                    let curEpoch = epoch $ _birkSeedState (bspBirkParameters bsp)
                    upds <- refLoad (bspUpdates bsp)
                    cooldown <- (2+) . _cpBakerExtraCooldownEpochs . unStoreSerialized <$> refLoad (currentParameters upds)
                    let (res, updateStake) = case compare newStake (_stakedAmount acctBkr) of
                                LT -> (BSUStakeReduced (BakerId ai) (curEpoch + cooldown), bakerPendingChange .~ ReduceStake newStake (curEpoch + cooldown))
                                EQ -> (BSUStakeUnchanged (BakerId ai), id)
                                GT -> (BSUStakeIncreased (BakerId ai), stakedAmount .~ newStake)
                        updAcc acc = do
                                acc' <- setPersistentAccountBaker acc (Some (updateStake acctBkr))
                                return ((), acc')
                    (_, newAccounts) <- Accounts.updateAccountsAtIndex updAcc ai (bspAccounts bsp)
                    (res,) <$> storePBS pbs bsp{bspAccounts = newAccounts}
            _ -> return (BSUInvalidBaker, pbs)

doUpdateBakerRestakeEarnings :: MonadBlobStore m => PersistentBlockState -> AccountAddress -> Bool -> m (BakerRestakeEarningsUpdateResult, PersistentBlockState)
doUpdateBakerRestakeEarnings pbs aaddr newRestakeEarnings = do
        bsp <- loadPBS pbs
        Accounts.getAccountWithIndex aaddr (bspAccounts bsp) >>= \case
            -- The account is valid and has a baker
            Just (ai, PersistentAccount{_accountBaker = Some pAcctBkr}) -> do
                acctBkr <- refLoad pAcctBkr
                if newRestakeEarnings == acctBkr ^. stakeEarnings
                then return (BREUUpdated (BakerId ai), pbs)
                else do
                    let updAcc acc = ((), ) <$> setPersistentAccountBaker acc (Some (acctBkr & stakeEarnings .~ newRestakeEarnings))
                    (_, newAccounts) <- Accounts.updateAccountsAtIndex updAcc ai (bspAccounts bsp)
                    (BREUUpdated (BakerId ai),) <$> storePBS pbs bsp{bspAccounts = newAccounts}
            _ -> return (BREUInvalidBaker, pbs)


doRemoveBaker :: MonadBlobStore m => PersistentBlockState -> AccountAddress -> m (BakerRemoveResult, PersistentBlockState)
doRemoveBaker pbs aaddr = do
        bsp <- loadPBS pbs
        Accounts.getAccountWithIndex aaddr (bspAccounts bsp) >>= \case
            -- The account is valid and has a baker
            Just (ai, PersistentAccount{_accountBaker = Some pab}) -> do
                ab <- refLoad pab
                if _bakerPendingChange ab /= NoChange then
                    -- A change is already pending
                    return (BRChangePending (BakerId ai), pbs)
                else do
                    -- We can make the change
                    -- Note: this just sets the account to be removed at a future epoch
                    -- transition.
                    let curEpoch = epoch $ _birkSeedState (bspBirkParameters bsp)
                    upds <- refLoad (bspUpdates bsp)
                    cooldown <- (2+) . _cpBakerExtraCooldownEpochs . unStoreSerialized <$> refLoad (currentParameters upds)
                    let updAcc acc = do
                            acc' <- setPersistentAccountBaker acc (Some ab{_bakerPendingChange = RemoveBaker (curEpoch + cooldown)})
                            return ((), acc')
                    (_, newAccounts) <- Accounts.updateAccountsAtIndex updAcc ai (bspAccounts bsp)
                    (BRRemoved (BakerId ai) (curEpoch + cooldown),) <$> storePBS pbs bsp{bspAccounts = newAccounts}
            -- The account is not valid or has no baker
            _ -> return (BRInvalidBaker, pbs)


doRewardBaker :: MonadBlobStore m => PersistentBlockState -> BakerId -> Amount -> m (Maybe AccountAddress, PersistentBlockState)
doRewardBaker pbs (BakerId ai) reward = do
        bsp <- loadPBS pbs
        (maddr, newAccounts) <- Accounts.updateAccountsAtIndex updAcc ai (bspAccounts bsp)
        (maddr,) <$> storePBS pbs bsp{bspAccounts = newAccounts}
    where
        updAcc acc = do
            addr <- acc ^^. accountAddress
            newAccountBaker <- forM (acc ^. accountBaker) $ \pbkr -> do
                bkr <- refLoad pbkr
                if bkr ^. stakeEarnings then
                    refMake $ bkr & stakedAmount +~ reward
                else
                    return pbkr
            acc' <- rehashAccount $ acc & accountBaker .~ newAccountBaker & accountAmount +~ reward
            return (addr, acc')

doGetRewardStatus :: MonadBlobStore m => PersistentBlockState -> m Rewards.BankStatus
doGetRewardStatus pbs = _unhashed . bspBank <$> loadPBS pbs

doSetInflation :: MonadBlobStore m => PersistentBlockState -> Amount -> m PersistentBlockState
doSetInflation pbs amount = do
        bsp <- loadPBS pbs
        storePBS pbs (bsp {bspBank = bspBank bsp & unhashed . Rewards.mintedGTUPerSlot .~ amount})

doMint :: MonadBlobStore m => PersistentBlockState -> Amount -> m (Amount, PersistentBlockState)
doMint pbs amount = do
        bsp <- loadPBS pbs
        let newBank = bspBank bsp & (unhashed . Rewards.totalGTU +~ amount) . (unhashed . Rewards.centralBankGTU +~ amount)
        (newBank ^. unhashed . Rewards.centralBankGTU,) <$> storePBS pbs (bsp {bspBank = newBank})

doDecrementCentralBankGTU :: MonadBlobStore m => PersistentBlockState -> Amount -> m (Amount, PersistentBlockState)
doDecrementCentralBankGTU pbs amount = do
        bsp <- loadPBS pbs
        let newBank = bspBank bsp & unhashed . Rewards.centralBankGTU -~ amount
        (newBank ^. unhashed . Rewards.centralBankGTU,) <$> storePBS pbs (bsp {bspBank = newBank})

doGetAccount :: MonadBlobStore m => PersistentBlockState -> AccountAddress -> m (Maybe PersistentAccount)
doGetAccount pbs addr = do
        bsp <- loadPBS pbs
        Accounts.getAccount addr (bspAccounts bsp)

doAccountList :: MonadBlobStore m => PersistentBlockState -> m [AccountAddress]
doAccountList pbs = do
        bsp <- loadPBS pbs
        Accounts.accountAddresses (bspAccounts bsp)

doRegIdExists :: MonadBlobStore m => PersistentBlockState -> ID.CredentialRegistrationID -> m Bool
doRegIdExists pbs regid = do
        bsp <- loadPBS pbs
        fst <$> Accounts.regIdExists regid (bspAccounts bsp)

doCreateAccount :: MonadBlobStore m => PersistentBlockState -> ID.GlobalContext -> ID.AccountKeys -> AccountAddress -> ID.AccountCredential ->  m (Maybe PersistentAccount, PersistentBlockState)
doCreateAccount pbs cryptoParams verifKeys acctAddr credential = do
        acct <- newAccount cryptoParams verifKeys acctAddr credential
        bsp <- loadPBS pbs
        -- Add the account
        (res, accts1) <- Accounts.putNewAccount acct (bspAccounts bsp)
        if res then do
            -- Record the RegId
            accts2 <- Accounts.recordRegId (ID.regId credential) accts1
            (Just acct,) <$> storePBS pbs (bsp {bspAccounts = accts2})
        else
            return (Nothing, pbs)

doModifyAccount :: MonadBlobStore m => PersistentBlockState -> AccountUpdate -> m PersistentBlockState
doModifyAccount pbs aUpd@AccountUpdate{..} = do
        bsp <- loadPBS pbs
        -- Do the update to the account
        (_, accts1) <- Accounts.updateAccounts upd _auAddress (bspAccounts bsp)
        -- If we deploy a credential, record it
        accts2 <- case _auCredential of
            Just cdi -> Accounts.recordRegId (ID.regId cdi) accts1
            Nothing -> return accts1
        storePBS pbs (bsp {bspAccounts = accts2})
    where
        upd oldAccount = ((), ) <$> Accounts.updateAccount aUpd oldAccount

doGetInstance :: MonadBlobStore m => PersistentBlockState -> ContractAddress -> m (Maybe Instance)
doGetInstance pbs caddr = do
        bsp <- loadPBS pbs
        minst <- Instances.lookupContractInstance caddr (bspInstances bsp)
        forM minst $ fromPersistentInstance pbs

doContractInstanceList :: MonadBlobStore m => PersistentBlockState -> m [Instance]
doContractInstanceList pbs = do
        bsp <- loadPBS pbs
        insts <- Instances.allInstances (bspInstances bsp)
        mapM (fromPersistentInstance pbs) insts

doPutNewInstance :: MonadBlobStore m => PersistentBlockState -> (ContractAddress -> Instance) -> m (ContractAddress, PersistentBlockState)
doPutNewInstance pbs fnew = do
        bsp <- loadPBS pbs
        -- Create the instance
        (inst, insts) <- Instances.newContractInstance fnew' (bspInstances bsp)
        let ca = instanceAddress (instanceParameters inst)
        -- Update the owner account's set of instances
        let updAcct oldAccount = ((), ) <$> (oldAccount & accountInstances %~~ Set.insert ca)
        (mdelegate, accts) <- Accounts.updateAccounts updAcct (instanceOwner (instanceParameters inst)) (bspAccounts bsp)
        -- Update the stake delegate
        case mdelegate of
            Nothing -> error "Invalid contract owner"
            Just _ -> do
                (ca,) <$> storePBS pbs bsp{
                                    bspInstances = insts,
                                    bspAccounts = accts
                                }
    where
        fnew' ca = let inst@Instance{instanceParameters = InstanceParameters{..}, ..} = fnew ca in do
            params <- makeBufferedRef $ PersistentInstanceParameters {
                                            pinstanceAddress = instanceAddress,
                                            pinstanceOwner = instanceOwner,
                                            pinstanceContractModule = instanceContractModule,
                                            pinstanceReceiveFuns = instanceReceiveFuns,
                                            pinstanceInitName = instanceInitName,
                                            pinstanceParameterHash = instanceParameterHash
                                        }
            return (inst, PersistentInstance{
                pinstanceParameters = params,
                pinstanceCachedParameters = Some (CacheableInstanceParameters{
                        pinstanceModuleInterface = instanceModuleInterface
                    }),
                pinstanceModel = instanceModel,
                pinstanceAmount = instanceAmount,
                pinstanceHash = instanceHash
            })

doModifyInstance :: MonadBlobStore m => PersistentBlockState -> ContractAddress -> AmountDelta -> Wasm.ContractState -> m PersistentBlockState
doModifyInstance pbs caddr deltaAmnt val = do
        bsp <- loadPBS pbs
        -- Update the instance
        Instances.updateContractInstance upd caddr (bspInstances bsp) >>= \case
            Nothing -> error "Invalid contract address"
            Just (_, insts) ->
                storePBS pbs bsp{bspInstances = insts}
    where
        upd oldInst = do
            (piParams, newParamsRef) <- cacheBufferedRef (pinstanceParameters oldInst)
            if deltaAmnt == 0 then
                return ((), rehash (pinstanceParameterHash piParams) $ oldInst {pinstanceParameters = newParamsRef, pinstanceModel = val})
            else
                return ((), rehash (pinstanceParameterHash piParams) $ oldInst {pinstanceParameters = newParamsRef, pinstanceAmount = applyAmountDelta deltaAmnt (pinstanceAmount oldInst), pinstanceModel = val})
        rehash iph inst@PersistentInstance {..} = inst {pinstanceHash = makeInstanceHash' iph pinstanceModel pinstanceAmount}

doGetIdentityProvider :: MonadBlobStore m => PersistentBlockState -> ID.IdentityProviderIdentity -> m (Maybe IPS.IpInfo)
doGetIdentityProvider pbs ipId = do
        bsp <- loadPBS pbs
        ips <- refLoad (bspIdentityProviders bsp)
        return $! IPS.idProviders ips ^? ix ipId

doGetAllIdentityProvider :: MonadBlobStore m => PersistentBlockState -> m [IPS.IpInfo]
doGetAllIdentityProvider pbs = do
        bsp <- loadPBS pbs
        ips <- refLoad (bspIdentityProviders bsp)
        return $! Map.elems $ IPS.idProviders ips

doGetAnonymityRevokers :: MonadBlobStore m => PersistentBlockState -> [ID.ArIdentity] -> m (Maybe [ARS.ArInfo])
doGetAnonymityRevokers pbs arIds = do
        bsp <- loadPBS pbs
        ars <- refLoad (bspAnonymityRevokers bsp)
        return
          $! let arsMap = ARS.arRevokers ars
              in forM arIds (`Map.lookup` arsMap)

doGetAllAnonymityRevokers :: MonadBlobStore m => PersistentBlockState -> m [ARS.ArInfo]
doGetAllAnonymityRevokers pbs = do
        bsp <- loadPBS pbs
        ars <- refLoad (bspAnonymityRevokers bsp)
        return $! Map.elems $ ARS.arRevokers ars

doGetCryptoParams :: MonadBlobStore m => PersistentBlockState -> m CryptographicParameters
doGetCryptoParams pbs = do
        bsp <- loadPBS pbs
        refLoad (bspCryptographicParameters bsp)

doGetTransactionOutcome :: MonadBlobStore m => PersistentBlockState -> Transactions.TransactionIndex -> m (Maybe TransactionSummary)
doGetTransactionOutcome pbs transHash = do
        bsp <- loadPBS pbs
        return $! bspTransactionOutcomes bsp ^? ix transHash

doGetTransactionOutcomesHash :: MonadBlobStore m => PersistentBlockState -> m TransactionOutcomesHash
doGetTransactionOutcomesHash pbs =  do
    bsp <- loadPBS pbs
    return $! getHash (bspTransactionOutcomes bsp)

doSetTransactionOutcomes :: MonadBlobStore m => PersistentBlockState -> [TransactionSummary] -> m PersistentBlockState
doSetTransactionOutcomes pbs transList = do
        bsp <- loadPBS pbs
        storePBS pbs bsp {bspTransactionOutcomes = Transactions.transactionOutcomesFromList transList}

doNotifyExecutionCost :: MonadBlobStore m => PersistentBlockState -> Amount -> m PersistentBlockState
doNotifyExecutionCost pbs amnt = do
        bsp <- loadPBS pbs
        storePBS pbs bsp {bspBank = bspBank bsp & unhashed . Rewards.executionCost +~ amnt}

doNotifyEncryptedBalanceChange :: MonadBlobStore m => PersistentBlockState -> AmountDelta -> m PersistentBlockState
doNotifyEncryptedBalanceChange pbs amntDiff = do
        bsp <- loadPBS pbs
        storePBS pbs bsp{bspBank = bspBank bsp & unhashed . Rewards.totalEncryptedGTU %~ applyAmountDelta amntDiff}

doNotifyIdentityIssuerCredential :: MonadBlobStore m => PersistentBlockState -> ID.IdentityProviderIdentity -> m PersistentBlockState
doNotifyIdentityIssuerCredential pbs idk = do
        bsp <- loadPBS pbs
        storePBS pbs bsp {bspBank = bspBank bsp & (unhashed . Rewards.identityIssuersRewards . at' idk . non 0) +~ 1}

doGetExecutionCost :: MonadBlobStore m => PersistentBlockState -> m Amount
doGetExecutionCost pbs = (^. unhashed . Rewards.executionCost) . bspBank <$> loadPBS pbs

doGetSpecialOutcomes :: MonadBlobStore m => PersistentBlockState -> m [Transactions.SpecialTransactionOutcome]
doGetSpecialOutcomes pbs = (^. to bspTransactionOutcomes . Transactions.outcomeSpecial) <$> loadPBS pbs

doGetOutcomes :: MonadBlobStore m => PersistentBlockState -> m (Vec.Vector TransactionSummary)
doGetOutcomes pbs = (^. to bspTransactionOutcomes . to Transactions.outcomeValues) <$> loadPBS pbs

doAddSpecialTransactionOutcome :: MonadBlobStore m => PersistentBlockState -> Transactions.SpecialTransactionOutcome -> m PersistentBlockState
doAddSpecialTransactionOutcome pbs !o = do
        bsp <- loadPBS pbs
        storePBS pbs $! bsp {bspTransactionOutcomes = bspTransactionOutcomes bsp & Transactions.outcomeSpecial %~ (o :)}

doGetElectionDifficulty :: MonadBlobStore m => PersistentBlockState -> Timestamp -> m ElectionDifficulty
doGetElectionDifficulty pbs ts = do
        bsp <- loadPBS pbs
        futureElectionDifficulty (bspUpdates bsp) ts

doGetNextUpdateSequenceNumber :: MonadBlobStore m => PersistentBlockState -> UpdateType -> m UpdateSequenceNumber
doGetNextUpdateSequenceNumber pbs uty = do
        bsp <- loadPBS pbs
        lookupNextUpdateSequenceNumber (bspUpdates bsp) uty

doGetCurrentElectionDifficulty :: MonadBlobStore m => PersistentBlockState -> m ElectionDifficulty
doGetCurrentElectionDifficulty pbs = do
        bsp <- loadPBS pbs
        upds <- refLoad (bspUpdates bsp)
        _cpElectionDifficulty . unStoreSerialized <$> refLoad (currentParameters upds)

doGetUpdates :: MonadBlobStore m => PersistentBlockState -> m Basic.Updates
doGetUpdates = makeBasicUpdates <=< refLoad . bspUpdates <=< loadPBS

doProcessUpdateQueues :: MonadBlobStore m => PersistentBlockState -> Timestamp -> m PersistentBlockState
doProcessUpdateQueues pbs ts = do
        bsp <- loadPBS pbs
        u' <- processUpdateQueues ts (bspUpdates bsp)
        storePBS pbs bsp{bspUpdates = u'}

doProcessReleaseSchedule :: MonadBlobStore m => PersistentBlockState -> Timestamp -> m PersistentBlockState
doProcessReleaseSchedule pbs ts = do
        bsp <- loadPBS pbs
        releaseSchedule <- loadBufferedRef (bspReleaseSchedule bsp)
        if Map.null releaseSchedule
          then return pbs
          else do
          let (accountsToRemove, blockReleaseSchedule') = Map.partition (<= ts) releaseSchedule
              f (ba, readded) addr = do
                let upd acc = do
                      rData <- loadBufferedRef (acc ^. accountReleaseSchedule)
                      let (_, rData') = TransientReleaseSchedule.unlockAmountsUntil ts rData
                      rDataRef <- makeBufferedRef rData'
                      acc' <- rehashAccount $ acc & accountReleaseSchedule .~ rDataRef
                      return (Map.lookupMin . TransientReleaseSchedule._pendingReleases $ rData',
                                acc')
                (toRead, ba') <- Accounts.updateAccounts upd addr ba
                return (ba', case join toRead of
                               Just (t, _) -> (addr, t) : readded
                               Nothing -> readded)
          (bspAccounts', accsToReadd) <- foldlM f (bspAccounts bsp, []) (Map.keys accountsToRemove)
          bspReleaseSchedule' <- makeBufferedRef $ foldl' (\b (a, t) -> Map.insert a t b) blockReleaseSchedule' accsToReadd
          storePBS pbs (bsp {bspAccounts = bspAccounts', bspReleaseSchedule = bspReleaseSchedule'})

doGetCurrentAuthorizations :: MonadBlobStore m => PersistentBlockState -> m Authorizations
doGetCurrentAuthorizations pbs = do
        bsp <- loadPBS pbs
        u <- refLoad (bspUpdates bsp)
        unStoreSerialized <$> refLoad (currentAuthorizations u)

doEnqueueUpdate :: MonadBlobStore m => PersistentBlockState -> TransactionTime -> UpdatePayload -> m PersistentBlockState
doEnqueueUpdate pbs effectiveTime payload = do
        bsp <- loadPBS pbs
        u' <- enqueueUpdate effectiveTime payload (bspUpdates bsp)
        storePBS pbs bsp{bspUpdates = u'}

doAddReleaseSchedule :: MonadBlobStore m => PersistentBlockState -> [(AccountAddress, Timestamp)] -> m PersistentBlockState
doAddReleaseSchedule pbs rel = do
        bsp <- loadPBS pbs
        releaseSchedule <- loadBufferedRef (bspReleaseSchedule bsp)
        let f relSchedule (addr, t) = Map.alter (\case
                                                    Nothing -> Just t
                                                    Just t' -> Just $ min t' t) addr relSchedule
        bspReleaseSchedule' <- makeBufferedRef $ foldl' f releaseSchedule rel
        storePBS pbs bsp {bspReleaseSchedule = bspReleaseSchedule'}

doGetEnergyRate :: MonadBlobStore m => PersistentBlockState -> m EnergyRate
doGetEnergyRate pbs = do
    bsp <- loadPBS pbs
    lookupEnergyRate (bspUpdates bsp)

newtype PersistentBlockStateContext = PersistentBlockStateContext {
    pbscBlobStore :: BlobStore
}

instance HasBlobStore PersistentBlockStateContext where
    blobStore = pbscBlobStore

newtype PersistentBlockStateMonad r m a = PersistentBlockStateMonad {runPersistentBlockStateMonad :: m a}
    deriving (Functor, Applicative, Monad, MonadIO, MonadReader r, MonadLogger)

type PersistentState r m = (MonadIO m, MonadReader r m, HasBlobStore r)

instance PersistentState r m => MonadBlobStore (PersistentBlockStateMonad r m)

type instance BlockStatePointer PersistentBlockState = BlobRef BlockStatePointers
type instance BlockStatePointer HashedPersistentBlockState = BlobRef BlockStatePointers

instance BlockStateTypes (PersistentBlockStateMonad r m) where
    type BlockState (PersistentBlockStateMonad r m) = HashedPersistentBlockState
    type UpdatableBlockState (PersistentBlockStateMonad r m) = PersistentBlockState
    type Account (PersistentBlockStateMonad r m) = PersistentAccount

instance PersistentState r m => BlockStateQuery (PersistentBlockStateMonad r m) where
    getModule = doGetModule . hpbsPointers
    getAccount = doGetAccount . hpbsPointers
    getContractInstance = doGetInstance . hpbsPointers
    getModuleList = doGetModuleList . hpbsPointers
    getAccountList = doAccountList . hpbsPointers
    getContractInstanceList = doContractInstanceList . hpbsPointers
    getSeedState = doGetSeedState . hpbsPointers
    getCurrentEpochBakers = doGetCurrentEpochBakers . hpbsPointers
    getSlotBakers = doGetSlotBakers . hpbsPointers
    getBakerAccount = doGetBakerAccount . hpbsPointers
    getRewardStatus = doGetRewardStatus . hpbsPointers
    getTransactionOutcome = doGetTransactionOutcome . hpbsPointers
    getTransactionOutcomesHash = doGetTransactionOutcomesHash . hpbsPointers
    getStateHash = return . hpbsHash
    getSpecialOutcomes = doGetSpecialOutcomes . hpbsPointers
    getOutcomes = doGetOutcomes . hpbsPointers
    getAllIdentityProviders = doGetAllIdentityProvider . hpbsPointers
    getAllAnonymityRevokers = doGetAllAnonymityRevokers . hpbsPointers
    getElectionDifficulty = doGetElectionDifficulty . hpbsPointers
    getNextUpdateSequenceNumber = doGetNextUpdateSequenceNumber . hpbsPointers
    getCurrentElectionDifficulty = doGetCurrentElectionDifficulty . hpbsPointers
    getUpdates = doGetUpdates . hpbsPointers

instance PersistentState r m => AccountOperations (PersistentBlockStateMonad r m) where

  getAccountAddress acc = acc ^^. accountAddress

  getAccountAmount acc = return $ acc ^. accountAmount

  getAccountNonce acc = return $ acc ^. accountNonce

  getAccountCredentials acc = acc ^^. accountCredentials

  getAccountMaxCredentialValidTo acc = acc ^^. accountMaxCredentialValidTo

  getAccountVerificationKeys acc = acc ^^. accountVerificationKeys

  getAccountEncryptedAmount acc = loadPersistentAccountEncryptedAmount =<< loadBufferedRef (acc ^. accountEncryptedAmount)

  getAccountEncryptionKey acc = acc ^^. accountEncryptionKey

  getAccountReleaseSchedule acc = loadBufferedRef (acc ^. accountReleaseSchedule)

  getAccountInstances acc = acc ^^. accountInstances

  getAccountBaker acc = case acc ^. accountBaker of
        Null -> return Nothing
        Some bref -> do
            PersistentAccountBaker{..} <- refLoad bref
            abi <- refLoad _accountBakerInfo
            return $ Just AccountBaker{_accountBakerInfo = abi, ..}

instance PersistentState r m => BlockStateOperations (PersistentBlockStateMonad r m) where
    bsoGetModule pbs mref = fmap moduleInterface <$> doGetModule pbs mref
    bsoGetAccount = doGetAccount
    bsoGetInstance = doGetInstance
    bsoRegIdExists = doRegIdExists
    bsoCreateAccount = doCreateAccount
    bsoPutNewInstance = doPutNewInstance
    bsoPutNewModule = doPutNewModule
    bsoModifyAccount = doModifyAccount
    bsoModifyInstance = doModifyInstance
    bsoNotifyExecutionCost = doNotifyExecutionCost
    bsoNotifyEncryptedBalanceChange = doNotifyEncryptedBalanceChange
    bsoNotifyIdentityIssuerCredential = doNotifyIdentityIssuerCredential
    bsoGetExecutionCost = doGetExecutionCost
    bsoGetSeedState = doGetSeedState
    bsoSetSeedState = doSetSeedState
    bsoTransitionEpochBakers = doTransitionEpochBakers
    bsoAddBaker = doAddBaker
    bsoUpdateBakerKeys = doUpdateBakerKeys
    bsoUpdateBakerStake = doUpdateBakerStake
    bsoUpdateBakerRestakeEarnings = doUpdateBakerRestakeEarnings
    bsoRemoveBaker = doRemoveBaker
    bsoRewardBaker = doRewardBaker
    bsoSetInflation = doSetInflation
    bsoMint = doMint
    bsoDecrementCentralBankGTU = doDecrementCentralBankGTU
    bsoGetIdentityProvider = doGetIdentityProvider
    bsoGetAnonymityRevokers = doGetAnonymityRevokers
    bsoGetCryptoParams = doGetCryptoParams
    bsoSetTransactionOutcomes = doSetTransactionOutcomes
    bsoAddSpecialTransactionOutcome = doAddSpecialTransactionOutcome
    bsoProcessUpdateQueues = doProcessUpdateQueues
    bsoProcessReleaseSchedule = doProcessReleaseSchedule
    bsoGetCurrentAuthorizations = doGetCurrentAuthorizations
    bsoGetNextUpdateSequenceNumber = doGetNextUpdateSequenceNumber
    bsoEnqueueUpdate = doEnqueueUpdate
    bsoAddReleaseSchedule = doAddReleaseSchedule
    bsoGetEnergyRate = doGetEnergyRate

instance PersistentState r m => BlockStateStorage (PersistentBlockStateMonad r m) where
    thawBlockState HashedPersistentBlockState{..} = do
            bsp <- loadPBS hpbsPointers
            pbsp <- makeBufferedRef bsp {
                        bspBank = bspBank bsp & unhashed . Rewards.executionCost .~ 0 & unhashed . Rewards.identityIssuersRewards .~ HM.empty
                    }
            liftIO $ newIORef $! pbsp

    freezeBlockState pbs = hashBlockState pbs

    dropUpdatableBlockState pbs = liftIO $ writeIORef pbs (error "Block state dropped")

    purgeBlockState pbs = liftIO $ writeIORef (hpbsPointers pbs) (error "Block state purged")

    archiveBlockState HashedPersistentBlockState{..} = do
        inner <- liftIO $ readIORef hpbsPointers
        inner' <- uncacheBuffered inner
        liftIO $ writeIORef hpbsPointers inner'

    saveBlockState HashedPersistentBlockState{..} = do
        inner <- liftIO $ readIORef hpbsPointers
        (inner', ref) <- flushBufferedRef inner
        liftIO $ writeIORef hpbsPointers inner'
        flushStore
        return ref

    loadBlockState hpbsHash ref = do
        hpbsPointers <- liftIO $ newIORef $ BRBlobbed ref
        return HashedPersistentBlockState{..}

    cacheBlockState pbs@HashedPersistentBlockState{..} = do
        bsp <- liftIO $ readIORef hpbsPointers
        bsp' <- cache bsp
        liftIO $ writeIORef hpbsPointers bsp'
        return pbs
