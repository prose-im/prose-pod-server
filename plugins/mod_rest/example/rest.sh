#!/bin/bash -eu

# Copyright (c) Kim Alvefur
# This file is MIT/X11 licensed.

# Dependencies:
# - https://httpie.io/
# - https://hg.sr.ht/~zash/httpie-oauth2

# shellcheck disable=SC1091

# Settings
HOST=""
DOMAIN=""

SESSION="session-read-only"

if [ -f "${XDG_CONFIG_HOME:-$HOME/.config}/restrc" ]; then
	# Config file can contain the above settings
	source "${XDG_CONFIG_HOME:-$HOME/.config}/restrc"

	if [ -z "${SCOPE:-}" ]; then
		SCOPE="openid xmpp"
	fi
fi

if [[ $# == 0 ]]; then
	echo "${0##*/} [-h HOST] [-rw] [/path] kind=(message|presence|iq) ...."
	# Last arguments are handed to HTTPie, so refer to its docs for further details
	exit 0
fi

if [[ "$1" == "-h" ]]; then
	HOST="$2"
	shift 2
elif [ -z "${HOST:-}" ]; then
	HOST="$(hostname)"
fi

if [[ "$1" == "-rw" ]]; then
	# To e.g. save Accept headers to the session
	SESSION="session"
	shift 1
fi

if [[ "$HOST" != *.* ]]; then
	# Assumes subdomain of your DOMAIN
	if [ -z "${DOMAIN:-}" ]; then
		DOMAIN="$(hostname -d)"
	fi
	if [[ "$HOST" == *:* ]]; then
		HOST="${HOST%:*}.$DOMAIN:${HOST#*:}"
	else
		HOST="$HOST.$DOMAIN"
	fi
fi


# For e.g /disco/example.com and such GET queries
GET_PATH=""
if [[ "$1" == /* ]]; then
	GET_PATH="$1"
	shift 1
fi

https --check-status -p b --"$SESSION" rest -A oauth2 -a "$HOST" --oauth2-scope "$SCOPE" "$HOST/rest$GET_PATH" "$@"
