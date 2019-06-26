{-# LANGUAGE TupleSections, LambdaCase #-}
{-# LANGUAGE LambdaCase #-}
module Main where

import Control.Concurrent
import Control.Monad
import System.Random
import Data.Time.Clock.POSIX
import System.IO
import Data.IORef
import Lens.Micro.Platform
import Data.List(intercalate)

import Concordium.Types.HashableTo
import Concordium.GlobalState.Parameters
import Concordium.GlobalState.Transactions
import Concordium.GlobalState.Block
import Concordium.GlobalState.Finalization
import Concordium.GlobalState.Instances
import Concordium.GlobalState.BlockState(BlockPointerData(..))
import Concordium.GlobalState.Basic.BlockState
import Concordium.GlobalState.Basic.TreeState
import Concordium.Scheduler.Utils.Init.Example as Example

import Concordium.Birk.Bake
import Concordium.Types
import Concordium.Runner
import Concordium.Logger
import Concordium.Skov
import Concordium.Afgjort.Finalize(FinalizationPoint)
import qualified Concordium.Getters as Get

import Concordium.Startup


type Peer = MVar SkovBufferedFinalizationState

data InEvent
    = IEMessage (InMessage Peer)
    | IECatchupFinalization FinalizationPoint Bool (Chan InEvent)

nContracts :: Int
nContracts = 2

transactions :: StdGen -> [Transaction]
transactions gen = trs (0 :: Nonce) (randoms gen :: [Int])
    where
        contr i = ContractAddress (fromIntegral $ i `mod` nContracts) 0
        trs n (a : b : rs) = Example.makeTransaction (a `mod` 9 /= 0) (contr b) n : trs (n+1) rs
        trs _ _ = error "Ran out of transaction data"

sendTransactions :: Chan (InEvent) -> [Transaction] -> IO ()
sendTransactions chan (t : ts) = do
        writeChan chan (IEMessage $ MsgTransactionReceived t)
        -- r <- randomRIO (5000, 15000)
        threadDelay 50000
        sendTransactions chan ts
sendTransactions _ _ = return ()

relayIn :: Chan InEvent -> Chan (InMessage Peer) -> MVar SkovBufferedFinalizationState -> IORef Bool -> IO ()
relayIn msgChan bakerChan sfsRef connectedRef = loop
    where
        loop = do
            msg <- readChan msgChan
            connected <- readIORef connectedRef
            when connected $ case msg of
                IEMessage imsg -> writeChan bakerChan imsg
                IECatchupFinalization fp reciprocate chan -> do
                    finMsgs <- Get.getFinalizationMessages sfsRef fp
                    forM_ finMsgs $ writeChan chan . IEMessage . MsgFinalizationReceived sfsRef
                    when reciprocate $ do
                        myFp <- Get.getFinalizationPoint sfsRef
                        writeChan chan $ IECatchupFinalization myFp False msgChan
            loop


relay :: Chan (OutMessage Peer) -> MVar SkovBufferedFinalizationState -> IORef Bool -> Chan (Either (BlockHash, BakedBlock, Maybe BlockState) FinalizationRecord) -> Chan InEvent -> [Chan InEvent] -> IO ()
relay inp sfsRef connectedRef monitor loopback outps = loop
    where
        chooseDelay = do
            factor <- (/2) . (+1) . sin . (*(pi/240)) . fromRational . toRational <$> getPOSIXTime
            truncate . (*(factor :: Double)) . fromInteger . (`div` 10) . (^(2::Int)) <$> randomRIO (0, 7800)
        delayed a = void $ forkIO $ do
            threadDelay =<< chooseDelay
            a
        usually a = do
            -- Do the action most of the time, but randomly don't do it.
            r <- randomRIO (0,9::Int)
            unless (r == 0) a
        loop = do
            msg <- readChan inp
            connected <- readIORef connectedRef
            when connected $ case msg of
                MsgNewBlock block -> do
                    let bh = getHash block :: BlockHash
                    sfs <- readMVar sfsRef
                    bp <- runSilentLogger $ flip evalSSM (sfs ^. skov) (resolveBlock bh)
                    -- when (isNothing bp) $ error "Block is missing!"
                    writeChan monitor (Left (bh, block, bpState <$> bp))
                    forM_ outps $ \outp -> usually $ delayed $
                        writeChan outp (IEMessage $ MsgBlockReceived sfsRef block)
                MsgFinalization bs ->
                    forM_ outps $ \outp -> delayed $
                        writeChan outp (IEMessage $ MsgFinalizationReceived sfsRef bs)
                MsgFinalizationRecord fr -> do
                    writeChan monitor (Right fr)
                    forM_ outps $ \outp -> usually $ delayed $
                        writeChan outp (IEMessage $ MsgFinalizationRecordReceived sfsRef fr)
                MsgMissingBlock src bh 0 -> do
                    mb <- Get.getBlockData src bh
                    case mb of
                        Just (NormalBlock bb) -> writeChan loopback (IEMessage $ MsgBlockReceived src bb)
                        _ -> return ()
                MsgMissingBlock src bh delta -> do
                    mb <- Get.getBlockDescendant src bh delta
                    case mb of
                        Just (NormalBlock bb) -> writeChan loopback (IEMessage $ MsgBlockReceived src bb)
                        _ -> return ()
                MsgMissingFinalization src fin -> do
                    mf <- case fin of
                        Left bh -> Get.getBlockFinalization src bh
                        Right fi -> Get.getIndexedFinalization src fi
                    forM_ mf $ \fr -> writeChan loopback (IEMessage $ MsgFinalizationRecordReceived src fr)
            loop

toggleConnection :: LogMethod IO -> MVar SkovBufferedFinalizationState -> IORef Bool -> Chan InEvent -> [Chan InEvent] -> IO ()
toggleConnection logM sfsRef connectedRef loopback outps = readIORef connectedRef >>= loop
    where
        loop connected = do
            delay <- (^(2::Int)) <$> randomRIO (if connected then (3200,7800) else (0,4500))
            threadDelay delay
            tid <- myThreadId
            if connected then do
                putStrLn $ "// " ++ show tid ++ ": toggle off"
                logM External LLInfo $ "Disconnected"
                writeIORef connectedRef False
                loop False
            else do
                -- Reconnect
                putStrLn $ "// " ++ show tid ++ ": toggle on"
                logM External LLInfo $ "Reconnected"
                writeIORef connectedRef True
                fp <- Get.getFinalizationPoint sfsRef
                forM_ outps $ \outp -> writeChan outp (IECatchupFinalization fp True loopback)
                loop True


removeEach :: [a] -> [(a,[a])]
removeEach = re []
    where
        re l (x:xs) = (x,l++xs) : re (x:l) xs
        re _ [] = []

gsToString :: BlockState -> String
gsToString gs = intercalate "\\l" . map show $ keys
    where
        ca n = ContractAddress (fromIntegral n) 0
        keys = map (\n -> (n, instanceModel <$> getInstance (ca n) (gs ^. blockInstances))) $ enumFromTo 0 (nContracts-1)

main :: IO ()
main = do
    let n = 20
    now <- truncate <$> getPOSIXTime
    let (gen, bis) = makeGenesisData now n 1 0.5
    let iState = Example.initialState (genesisBirkParameters gen) (genesisBakerAccounts gen) nContracts
    trans <- transactions <$> newStdGen
    chans <- mapM (\(bid, _) -> do
        let logFile = "consensus-" ++ show now ++ "-" ++ show (bakerId bid) ++ ".log"
        logChan <- newChan
        let logLoop = do
                logMsg <- readChan logChan
                appendFile logFile logMsg
                logLoop
        _ <- forkIO logLoop
        let logM src lvl msg = do
                                    timestamp <- getCurrentTime
                                    writeChan logChan $ "[" ++ show timestamp ++ "] " ++ show lvl ++ " - " ++ show src ++ ": " ++ msg ++ "\n"
        (cin, cout, out) <- makeAsyncRunner logM bid gen iState
        cin' <- newChan
        connectedRef <- newIORef True
        _ <- forkIO $ relayIn cin' cin out connectedRef
        _ <- forkIO $ sendTransactions cin' trans
        return (cin', cout, out, connectedRef, logM)) bis
    monitorChan <- newChan
    forM_ (removeEach chans) $ \((cin, cout, stateRef, connectedRef, logM), cs) -> do
        let cs' = ((\(c, _, _, _, _) -> c) <$> cs)
        _ <- forkIO $ toggleConnection logM stateRef connectedRef cin cs'
        forkIO $ relay cout stateRef connectedRef monitorChan cin cs'
    let loop = do
            readChan monitorChan >>= \case
                Left (bh, block, gs') -> do
                    let ts = blockTransactions block
                    let stateStr = case gs' of
                                    Nothing -> ""
                                    Just gs -> gsToString gs
                    putStrLn $ " n" ++ show bh ++ " [label=\"" ++ show (blockBaker $ bbFields block) ++ ": " ++ show (blockSlot block) ++ " [" ++ show (length ts) ++ "]\\l" ++ stateStr ++ "\\l\"];"
                    putStrLn $ " n" ++ show bh ++ " -> n" ++ show (blockPointer $ bbFields block) ++ ";"
                    putStrLn $ " n" ++ show bh ++ " -> n" ++ show (blockLastFinalized $ bbFields block) ++ " [style=dotted];"
                    hFlush stdout
                    loop
                Right fr -> do
                    putStrLn $ " n" ++ show (finalizationBlockPointer fr) ++ " [color=green];"
                    loop
    loop

