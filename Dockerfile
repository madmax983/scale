# Builds the browser (WASM) client with Trunk and serves dist/ with Caddy.
#
# The build stage runs on the builder's native platform: its output is
# platform-independent WASM, so there's no reason to emulate the deploy target.
FROM --platform=$BUILDPLATFORM rust:1-bookworm AS build

ARG TRUNK_VERSION=0.21.14
RUN rustup target add wasm32-unknown-unknown \
    && curl -fsSL "https://github.com/trunk-rs/trunk/releases/download/v${TRUNK_VERSION}/trunk-$(uname -m)-unknown-linux-gnu.tar.gz" \
    | tar -xz -C /usr/local/bin

WORKDIR /app
COPY . .
RUN trunk build --release

FROM caddy:2-alpine
COPY Caddyfile /etc/caddy/Caddyfile
COPY --from=build /app/dist /srv
