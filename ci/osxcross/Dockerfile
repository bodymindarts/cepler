FROM rust:latest

RUN apt-get update && apt-get install -y \
    cmake \
    clang \
    gcc \
    g++ \
    bison \
    flex \
    git \
    zlib1g-dev \
    libmpc-dev \
    libmpfr-dev \
    libgmp-dev \
    protobuf-compiler

RUN rustup target add x86_64-apple-darwin x86_64-unknown-linux-gnu

COPY . /workspace
WORKDIR /workspace

ENV CARGO_HOME=/workspace/.cargo

RUN ./osxcross_setup.sh

# Add osxcross toolchain to path
ENV PATH=/workspace/osxcross/target/bin:$PATH

RUN git clone --branch=cross_platform https://github.com/markmentovai/bootstrap_cmds \
    && cd bootstrap_cmds \
    && autoreconf --install \
    && sh configure \
    && make && make install
