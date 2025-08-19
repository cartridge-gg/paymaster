FROM rust:1.83 as builder

WORKDIR /paymaster

COPY . .

# Build for release.
RUN cargo build --release --bin paymaster-cli

FROM gcr.io/distroless/cc-debian12
COPY --from=builder /paymaster/target/release/paymaster-cli .

CMD ["./paymaster-cli"]
