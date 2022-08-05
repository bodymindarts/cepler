FROM rust

RUN apt-get update && apt-get install -y \
    cmake \
    clang \
    gcc \
    g++ \
    zlib1g-dev \
    libmpc-dev \
    libmpfr-dev \
    libgmp-dev \
    protobuf-compiler

RUN rustup target add x86_64-apple-darwin

COPY . /workspace
WORKDIR /workspace

ENV CARGO_HOME=/workspace/.cargo

RUN ./osxcross_setup.sh

# Add osxcross toolchain to path
ENV PATH=/workspace/osxcross/target/bin:$PATH
