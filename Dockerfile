# Stage 1: Build the Rust application using a newer Rust image
FROM rust:1.84.1 AS builder
WORKDIR /app

# Copy Cargo files and fetch dependencies (using a dummy main to cache deps)
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo "fn main() {}" > src/main.rs
RUN cargo fetch

# Copy the full source code and build in release mode.
COPY . .
RUN cargo build --release

# Stage 2: Create the final runtime image
FROM ubuntu:22.04

# Set noninteractive mode for apt-get to prevent interactive prompts
ENV DEBIAN_FRONTEND=noninteractive

# Install runtime dependencies and tools to build NSJail.
RUN apt-get update && apt-get install -y \
    libssl-dev \
    ca-certificates \
    build-essential \
    git \
    libseccomp-dev \
    pkg-config \
    flex \
    bison \
    libprotobuf-dev \
    protobuf-compiler \
    libnl-3-dev \
    libnl-route-3-dev \
    libcap-dev \
    libnetfilter-queue-dev \
    libmnl-dev \
    libaio-dev \
    libcmocka-dev \
    libsystemd-dev \
    libev-dev \
    liblz4-dev \
    liblzma-dev \
    libunwind-dev \
    libaudit-dev \
    libgtk-3-dev \
    libtool \
    libtool-bin \
    && rm -rf /var/lib/apt/lists/*

# Clone and build NSJail from source.
RUN git clone https://github.com/google/nsjail.git /nsjail && \
    cd /nsjail && make

# Symlink NSJail into /usr/local/bin so it is available in PATH.
RUN ln -s /nsjail/nsjail /usr/local/bin/nsjail

# Set the working directory to /app
WORKDIR /app

# Copy static assets and test cases file.
COPY static /app/static
COPY test_cases.json /app/test_cases.json
COPY tempfiles /app/tempfiles
RUN chmod -R +x /app/tempfiles
# Copy the compiled Rust binary from the builder stage.
COPY --from=builder /app/target/release/autograder /app/autograder

# Expose the port used by your Rocket app.
EXPOSE 8000

# Run your application.
CMD ["/app/autograder"]
