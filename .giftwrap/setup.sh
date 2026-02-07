#!/bin/sh
set -eu

export DEBIAN_FRONTEND=noninteractive

# In the rootless user-namespace setup sandbox, apt's default _apt privilege
# drop can fail with setgroups/seteuid errors. Force apt sandbox user to root.
apt-get -o APT::Sandbox::User=root update
apt-get -o APT::Sandbox::User=root install -y --no-install-recommends neovim
