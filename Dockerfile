# NexaOS Build Container
# Provides a reproducible, isolated build environment

FROM ubuntu:24.04

LABEL maintainer="Nexa Development Team"
LABEL description="NexaOS Enterprise Build Environment"

# Install build dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential \
    gcc \
    g++ \
    make \
    pkg-config \
    git \
    curl \
    wget \
    ca-certificates \
    # Kernel build dependencies
    linux-headers-generic \
    # Bootloader/ISO tools
    qemu-system-x86 \
    qemu-utils \
    xorriso \
    isolinux \
    syslinux \
    # Filesystems
    e2fsprogs \
    mkisofs \
    # Development tools
    strace \
    gdb \
    valgrind \
    # Build acceleration
    ccache \
    ninja-build \
    # Documentation
    doxygen \
    graphviz \
    && rm -rf /var/lib/apt/lists/*

# Install modern build tools
RUN apt-get update && apt-get install -y --no-install-recommends \
    cmake \
    meson \
    # Optional: Buildroot support
    bc \
    bison \
    flex \
    texinfo \
    && rm -rf /var/lib/apt/lists/*

# Set up build user (non-root)
RUN useradd -m -s /bin/bash builder && \
    mkdir -p /workspace && \
    chown -R builder:builder /workspace

# Set environment for ccache
ENV CCACHE_DIR=/workspace/.ccache
ENV PATH="/usr/lib/ccache:$PATH"

# Set working directory
WORKDIR /workspace

# Create build directories
RUN mkdir -p build dist mnt rootfs .build_cache && \
    chown -R builder:builder /workspace

# Copy source (optional - can be mounted as volume)
# COPY . /workspace/

# Use builder user for builds
USER builder

# Default command
CMD ["/bin/bash"]

# Build instructions:
# docker build -t nexaos/build:latest -f Dockerfile .
# docker run --rm -v $(pwd):/workspace nexaos/build:latest make all
