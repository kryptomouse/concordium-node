module Main where

import System.Environment
import Data.Semigroup
import Data.List (stripPrefix)
import Test.Hspec
import qualified GlobalStateTests.Instances(tests)
import qualified GlobalStateTests.FinalizationSerializationSpec(tests)
import qualified GlobalStateTests.Trie(tests)
import qualified GlobalStateTests.LFMBTree(tests)
import qualified GlobalStateTests.PersistentTreeState(tests)
import qualified GlobalStateTests.Accounts(tests)
import qualified GlobalStateTests.BlockHash(tests)
import qualified GlobalStateTests.AccountReleaseScheduleTest(tests)
import qualified GlobalStateTests.Updates(tests)

atLevel :: (Word -> IO ()) -> IO ()
atLevel a = do
        args0 <- getArgs
        let (args1, mlevel) = mconcat $ map lvlArg args0
        withArgs args1 $ a $! (maybe 1 getLast mlevel)
    where
        lvlArg s = case stripPrefix "--level=" s of
            Nothing -> ([s], Nothing)
            Just r -> ([], Just $! Last $! (read r :: Word))

main :: IO ()
main = atLevel $ \lvl -> hspec $ do
  GlobalStateTests.BlockHash.tests
  GlobalStateTests.LFMBTree.tests
  GlobalStateTests.Accounts.tests lvl
  GlobalStateTests.Trie.tests
  GlobalStateTests.PersistentTreeState.tests
  GlobalStateTests.FinalizationSerializationSpec.tests
  GlobalStateTests.Instances.tests lvl
  GlobalStateTests.AccountReleaseScheduleTest.tests
  GlobalStateTests.Updates.tests
