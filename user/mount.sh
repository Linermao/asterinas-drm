#!/bin/bash
set -e

mkdir -p /mnt
mount -o loop -t ext2 ./test/initramfs/build/ext2.img /mnt
cp -rf ./user/build/ /mnt
sync
umount /mnt

make BOOT_PROTOCOL=linux-efi-handover64 run_kernel 