#!/usr/bin/env bash

# Keep-alive: until the script has finished
while true; do sudo -n true; sleep 60; kill -0 "$$" || exit; done 2>/dev/null &
