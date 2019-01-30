#!/bin/bash

if [ "$#" -ne 1 ]
then
  echo "Usage: ./build-universal-docker.sh VERSION-TAG"
  exit 1
fi

docker build -f scripts/universal.Dockerfile -t concordium/universal:$1 .

docker tag concordium/universal:$1 192549843005.dkr.ecr.eu-west-1.amazonaws.com/concordium/universal:$1

docker push 192549843005.dkr.ecr.eu-west-1.amazonaws.com/concordium/universal:$1

echo "DONE BUILDING universal!"