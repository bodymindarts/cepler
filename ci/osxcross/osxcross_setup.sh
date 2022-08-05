#!/bin/bash

git clone https://github.com/tpoechtrager/osxcross
cd osxcross
mv ../MacOSX12.3.sdk.tar.bz2 tarballs/
UNATTENDED=yes ./build.sh
