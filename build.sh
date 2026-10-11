#!/bin/bash
# Modern NexaOS Build Launcher
# This replaces the old individual build scripts with a unified, modern approach

set -euo pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'  # No Color

# Configuration
PROJECT_VERSION="3.1.0"
BUILD_DIR="build"
DIST_DIR="dist"
LOG_FILE="$BUILD_DIR/build.log"

# Helper functions
print_header() {
    echo -e "${BLUE}========================================${NC}"
    echo -e "${BLUE}$1${NC}"
    echo -e "${BLUE}========================================${NC}"
}

print_success() {
    echo -e "${GREEN}✓ $1${NC}"
}

print_error() {
    echo -e "${RED}✗ Error: $1${NC}"
    exit 1
}

print_warning() {
    echo -e "${YELLOW}⚠ Warning: $1${NC}"
}

print_info() {
    echo -e "${BLUE}ℹ $1${NC}"
}

show_help() {
    cat << 'EOF'
NexaOS Modern Build System v3.1.0

USAGE:
  ./build.sh [COMMAND] [OPTIONS]

COMMANDS:
  setup            Prepare build environment
  build            Full build (kernel, rootfs, disk, iso)
  build-kernel     Build Linux kernel only
  build-modules    Build kernel modules
  build-apps       Build applications and GUI
  boot             Boot in QEMU (serial mode)
  boot-gui         Boot in QEMU (GUI mode)
  docker           Build in Docker container
  ci               Run CI pipeline (local testing)
  clean            Remove build artifacts
  distclean        Remove everything
  check             Verify dependencies
  help             Show this help message

OPTIONS:
  --parallel N     Set parallel jobs (default: nproc)
  --config FILE    Use custom build.conf (default: build.conf)
  --docker         Use Docker for build
  --verbose        Enable verbose output
  --version        Show version

EXAMPLES:
  ./build.sh setup                 # Initialize build
  ./build.sh build                 # Full build
  ./build.sh boot                  # Boot in QEMU
  ./build.sh docker build          # Build in Docker
  ./build.sh build --parallel 8    # Build with 8 jobs

CONFIGURATION:
  All build options are in build.conf
  Edit build.conf to customize versions, features, flags, etc.

ENVIRONMENT:
  BUILD_PARALLEL  Number of parallel jobs
  VERBOSE         Enable verbose output
  USE_DOCKER      Force Docker builds

EOF
}

# Check dependencies
check_dependencies() {
    print_header "Checking Build Dependencies"
    
    local missing=0
    local deps=("gcc" "make" "qemu-img" "qemu-system-x86_64")
    
    for cmd in "${deps[@]}"; do
        if command -v "$cmd" &> /dev/null; then
            print_success "$cmd found"
        else
            print_warning "$cmd not found (optional)"
            ((missing++))
        fi
    done
    
    if [ $missing -gt 0 ]; then
        print_warning "Some optional dependencies are missing"
    fi
    
    if ! command -v make &> /dev/null; then
        print_error "make is required but not found"
    fi
}

# Setup build directories
setup_build() {
    print_header "Setting Up Build Environment"
    
    mkdir -p "$BUILD_DIR" "$DIST_DIR" mnt rootfs
    mkdir -p "$BUILD_DIR"/{apps,modules}
    mkdir -p "$DIST_DIR"/{iso,packages}
    
    print_success "Build directories created"
    print_info "Log file: $LOG_FILE"
}

# Run Makefile targets
run_make() {
    local target="$1"
    shift
    local args=("$@")
    
    if make "$target" "${args[@]}" 2>&1 | tee -a "$LOG_FILE"; then
        print_success "Target '$target' completed"
    else
        print_error "Target '$target' failed (see $LOG_FILE)"
    fi
}

# Main build
build_all() {
    print_header "Building NexaOS ${PROJECT_VERSION}"
    
    print_info "Configuration: build.conf"
    
    if [ "${VERBOSE:-0}" = "1" ]; then
        print_info "Verbose output enabled"
    fi
    
    run_make "all" VERBOSE="${VERBOSE:-0}"
    
    echo ""
    print_header "Build Complete"
    echo ""
    echo "Artifacts in $(pwd)/$DIST_DIR:"
    ls -lh "$DIST_DIR"/{bzImage,initramfs.cpio.gz,nexa.img} 2>/dev/null || true
    echo ""
    print_info "Boot with:"
    echo "  Serial:  ./build.sh boot"
    echo "  GUI:     ./build.sh boot-gui"
    echo ""
}

# Docker build
build_docker() {
    print_header "Building in Docker"
    
    if ! command -v docker &> /dev/null; then
        print_error "Docker is not installed"
    fi
    
    print_info "Building Docker image..."
    docker build -t nexaos/build:latest -f Dockerfile . 2>&1 | tee -a "$LOG_FILE"
    
    print_info "Running build in container..."
    docker run --rm \
        -v "$(pwd):/workspace" \
        -e VERBOSE="${VERBOSE:-0}" \
        nexaos/build:latest \
        make -C /workspace all VERBOSE="${VERBOSE:-0}" 2>&1 | tee -a "$LOG_FILE"
    
    print_success "Docker build complete"
}

# QEMU boot
boot_system() {
    local mode="$1"
    
    if [ ! -f "$DIST_DIR/nexa.img" ]; then
        print_error "Disk image not found. Run './build.sh build' first."
    fi
    
    print_header "Booting NexaOS in QEMU (${mode} mode)"
    
    case "$mode" in
        serial|text)
            print_info "Press Ctrl+A then X to exit QEMU"
            run_make "boot"
            ;;
        gui|graphical)
            run_make "boot-gui"
            ;;
        *)
            print_error "Unknown boot mode: $mode"
            ;;
    esac
}

# Main entry point
main() {
    local command="${1:-help}"
    shift 2>/dev/null || true
    
    case "$command" in
        help|-h|--help)
            show_help
            ;;
        
        check)
            check_dependencies
            ;;
        
        setup)
            check_dependencies
            setup_build
            ;;
        
        build)
            setup_build
            build_all "$@"
            ;;
        
        build-kernel)
            setup_build
            run_make "kernel" "$@"
            ;;
        
        build-modules)
            setup_build
            run_make "kernel-module" "$@"
            ;;
        
        build-apps)
            setup_build
            run_make "apps" "$@"
            run_make "gui" "$@"
            ;;
        
        boot)
            boot_system "serial"
            ;;
        
        boot-gui)
            boot_system "gui"
            ;;
        
        docker)
            build_docker
            ;;
        
        ci)
            setup_build
            run_make "ci" "$@"
            ;;
        
        clean)
            print_header "Cleaning Build Artifacts"
            run_make "clean"
            ;;
        
        distclean)
            print_header "Deep Clean (removing all builds)"
            run_make "distclean"
            ;;
        
        version)
            echo "NexaOS Build System v${PROJECT_VERSION}"
            ;;
        
        *)
            print_error "Unknown command: $command"
            show_help
            exit 1
            ;;
    esac
}

# Run main function
main "$@"
