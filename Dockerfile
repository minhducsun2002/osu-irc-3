FROM rust:1.76.0-alpine3.19 as build


WORKDIR /app
COPY . .
RUN cargo build --release

FROM alpine:3.19.1 as run
WORKDIR /app
COPY --from=build /app/target/release/osu-irc-3 .
ENTRYPOINT /app/osu-irc-3