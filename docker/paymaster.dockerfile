FROM rust:1.83 as builder

ARG BUILD_TOKEN

WORKDIR /paymaster

COPY ./ .

# Build for release.
RUN cargo build --release

FROM debian:12-slim
COPY --from=builder /paymaster/target/release/paymaster-service .

CMD ["./paymaster-service"]
