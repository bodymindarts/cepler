#!/bin/bash

git clone https://github.com/tpoechtrager/osxcross
cd osxcross
mv ../MacOSX10.15.sdk.tar.xz tarballs/
UNATTENDED=yes ./build.sh
