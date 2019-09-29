#!/bin/bash
set -e

if [ -z "$JENKINS_HOME" ]; then
  CURRENT_BRANCH=$(git branch | grep \* | cut -d ' ' -f2)
  git checkout master
  git pull
fi

if [ "$#" -ne 1 ]
then
  echo "Usage: ./build-testnet-production-release.sh [debug|release]"
  exit 1
fi

PATH="$PATH:/usr/local/bin" git lfs install
PATH="$PATH:/usr/local/bin" git lfs pull

VERSION=`cat Cargo.toml | grep "version = \"" | head -n1 | sed 's/version = \"//' | sed 's/\"//'`

./scripts/build-all-docker.sh $VERSION $1

if [ -z "$JENKINS_HOME" ]; then
  git checkout $CURRENT_BRANCH
fi

echo "Finished building production release with tag $VERSION"
