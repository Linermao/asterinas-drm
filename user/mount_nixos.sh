#!/bin/bash
set -e

echo "remove target/nixos/asterinas.img ...."
rm -r ./target/nixos/asterinas.img 

echo "copy asterinas.img ...."
cp -f ./asterinas.img ./target/nixos/

echo "make nixos"
make nixos

echo "make run_nixos"
make run_nixos