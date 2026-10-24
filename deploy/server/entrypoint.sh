#!/bin/sh
set -eu

exec xvfb-run -a --server-args="-screen 0 1280x900x24 -nolisten tcp" /usr/local/bin/chek "$@"
