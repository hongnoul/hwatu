#!/bin/sh
set -eu

exec dbus-run-session -- hwatu mcp
