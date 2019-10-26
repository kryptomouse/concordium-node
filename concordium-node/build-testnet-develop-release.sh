#!/usr/bin/env bash
set -e

if [ "$#" -lt 1 ]
then
  echo "Usage: ./build-testnet-develop-release.sh [debug|release] [default|no-rgs] [profiling=[true|false]]"
  exit 1
fi

CONSENSUS_TYPE=""
if [ ! -z "$2" ]; then 
  CONSENSUS_TYPE="$2"
else
  CONSENSUS_TYPE="default"
fi

CONSENSUS_PROFILING="false"
if [[ ! -z "$3" && "$3" == "true" ]]; then 
  CONSENSUS_PROFILING="true"
fi

if [ -z "$JENKINS_HOME" ]; then
  git pull
fi

PATH="$PATH:/usr/local/bin" git lfs install
PATH="$PATH:/usr/local/bin" git lfs pull

VERSION=`git rev-parse --verify HEAD`

./scripts/build-all-docker.sh $VERSION $2 $CONSENSUS_TYPE $CONSENSUS_PROFILING

echo "Finished building and pushing develop release with tag $VERSION with consensus $CONSENSUS_TYPE, and profiling $CONSENSUS_PROFILING"
