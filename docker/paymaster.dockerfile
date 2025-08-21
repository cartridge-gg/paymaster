FROM rust:1.83 as builder

ARG BUILD_TOKEN

WORKDIR /paymaster

COPY ./ .

# Build for release.
RUN cargo build --release

FROM gcr.io/distroless/cc-debian12
COPY --from=builder /paymaster/target/release/paymaster-service .

CMD ["./paymaster-service"]
