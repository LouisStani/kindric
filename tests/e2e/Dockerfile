# kindric's end-to-end tests (decisions 250, 251, 310): the desktop app in a container, on a screen
# that does not exist (Xvfb), driven through WebDriver (tauri-driver + WebKitWebDriver), with a fake
# claude (tests/e2e/bin/claude). Nothing is installed on the machine that runs them.
# Build and run: tests/e2e/run.sh

FROM debian:trixie

ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates curl build-essential pkg-config git \
        libwebkit2gtk-4.1-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev libxdo-dev \
        webkit2gtk-driver xvfb xauth dbus-x11 fonts-dejavu-core python3 \
    && rm -rf /var/lib/apt/lists/*

# Rust for building kindric here: a binary built on another distribution may not run on this one.
ENV RUSTUP_HOME=/opt/rustup CARGO_HOME=/opt/cargo PATH=/opt/cargo/bin:$PATH
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain stable

# Tauri's own WebDriver server, which hands the app to WebKitWebDriver.
RUN cargo install --locked tauri-driver

# The tests run as a user, never as root: Claude Code and kindric live in a home.
RUN useradd --create-home tester
WORKDIR /src
COPY --chown=tester:tester . /src
USER tester
# The toolchain stays in /opt (root's); the crates downloaded for kindric go in the tester's home.
ENV CARGO_HOME=/home/tester/.cargo CARGO_TARGET_DIR=/home/tester/target
RUN cargo build --locked --release -p kindric-desktop -p kindric-accounts -p kindric

# The fake claude and kindric's programs come first in PATH: no real claude exists here.
ENV PATH=/src/tests/e2e/bin:/home/tester/target/release:$PATH
