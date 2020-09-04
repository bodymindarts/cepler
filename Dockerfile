FROM clux/muslrust:stable AS build
  COPY . /src
  WORKDIR /src
  RUN cargo build --release

FROM alpine:latest
  COPY --from=build /src/target/x86_64-unknown-linux-musl/release/cepler /bin/
  RUN apk update && apk upgrade && apk add bash
  CMD ["cepler --help"]
