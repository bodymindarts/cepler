#!/bin/bash

cargo install --all-features cepler
if [[ $(cepler --version) != "cepler $(cat version/number)" ]]; then
  echo "Installed cepler does not have expected version number"
  exit 1
fi
cepler help
