#!/bin/bash

git clone https://github.com/tpoechtrager/osxcross
cd osxcross
mv ../MacOSX14.0.sdk.tar.bz2 tarballs/
UNATTENDED=yes ./build.sh
