# NexaOS Modern Build System

## Overview

The NexaOS build system has been completely modernized to follow enterprise best practices:

- **Unified Makefile**: Single source of truth for all build targets
- **Centralized Configuration**: `build.conf` replaces scattered parameters
- **Reproducible Builds**: Docker containerization ensures consistency
- **Automated CI/CD**: GitHub Actions pipeline for testing and releases
- **Modern Tooling**: Support for Buildroot, CMake, Meson
- **Parallel Compilation**: Optimized for multi-core systems
- **Comprehensive Logging**: Full build audit trail

## Quick Start

### Prerequisites
```bash
# Ubuntu/Debian
sudo apt-get install build-essential gcc g++ make qemu-system-x86 e2fsprogs

# macOS (with Homebrew)
brew install gcc qemu e2fsprogs
```

### Basic Build
```bash
# Initialize build environment
./build.sh setup

# Full build (kernel, rootfs, disk, ISO)
./build.sh build

# Boot in QEMU
./build.sh boot                    # Serial mode (recommended)
./build.sh boot-gui                # GUI mode with display window
```

## File Structure

```
NexaOS/
├── Makefile              # Enterprise build system (replaces all .sh scripts)
├── build.conf            # Configuration (versions, flags, features)
├── build.sh              # User-friendly build launcher
├── Dockerfile            # Reproducible build environment
├── buildroot.config      # Buildroot configuration
├── .github/
│   └── workflows/
│       └── ci.yml        # GitHub Actions CI/CD pipeline
├── src/                  # Source code
├── build/                # Build artifacts (auto-created)
├── dist/                 # Distributions (ISO, images)
└── docs/                 # Documentation
```

## Configuration

### build.conf
Central configuration file that controls:
- Versions of all components
- Compiler flags and optimizations
- Feature enablement (SSH, GUI, Python, etc.)
- QEMU settings
- Package management
- Security hardening

**Example customizations:**
```bash
# Change kernel version
KERNEL_VERSION=6.18.33

# Enable/disable features
ENABLE_GUI=yes
ENABLE_SSH=yes
ENABLE_PYTHON=yes

# Compiler optimization
CFLAGS_COMMON=-Wall -Wextra -O3

# Parallel compilation jobs
PARALLEL_JOBS=8
```

## Build Targets

### Using make directly:
```bash
make setup              # Prepare environment
make kernel             # Build Linux kernel only
make kernel-module      # Build nexa_proc kernel module
make rootfs             # Build root filesystem
make initramfs          # Build initramfs
make disk               # Create disk image
make apps               # Build applications
make gui                # Build GUI components
make iso                # Create bootable ISO
make boot               # Boot in QEMU (serial)
make boot-gui           # Boot in QEMU (GUI)
make ci                 # Run full CI pipeline
make docker             # Build in Docker
make clean              # Remove build artifacts
make distclean          # Complete clean
make check-env          # Verify dependencies
```

### Using build.sh launcher:
```bash
./build.sh help         # Show all commands
./build.sh setup        # Initialize
./build.sh build        # Full build
./build.sh boot         # Boot system
./build.sh docker       # Docker build
./build.sh clean        # Cleanup
```

## Advanced Usage

### Custom Configuration
```bash
# Use custom build configuration
make -f Makefile all KERNEL_VERSION=6.20

# Override parallel jobs
make all PARALLEL_JOBS=16

# Build with verbose output
make all VERBOSE=1
```

### Docker Build (Reproducible)
```bash
# Build Docker image
docker build -t nexaos/build:latest .

# Run build in container
docker run --rm -v $(pwd):/workspace nexaos/build:latest make all

# One-liner
./build.sh docker
```

### Continuous Integration
```bash
# Run local CI pipeline (mimics GitHub Actions)
make ci

# This runs:
# 1. Setup
# 2. Build all
# 3. Run tests
# 4. Create ISO
# 5. Validate artifacts
```

## Build Process Flow

```
┌─────────────────────────────────────────────────────────────┐
│ 1. Setup: Create directories and verify environment         │
└────────────────┬────────────────────────────────────────────┘
                 │
┌────────────────▼────────────────────────────────────────────┐
│ 2. Compile Core Binaries (init, httpd, monitor, etc.)       │
└────────────────┬────────────────────────────────────────────┘
                 │
┌────────────────▼────────────────────────────────────────────┐
│ 3. Build Kernel (6.18.33)                                   │
└────────────────┬────────────────────────────────────────────┘
                 │
┌────────────────▼────────────────────────────────────────────┐
│ 4. Build Kernel Module (nexa_proc)                          │
└────────────────┬────────────────────────────────────────────┘
                 │
┌────────────────▼────────────────────────────────────────────┐
│ 5. Create Root Filesystem (etc, bin, lib, etc.)             │
└────────────────┬────────────────────────────────────────────┘
                 │
┌────────────────▼────────────────────────────────────────────┐
│ 6. Build Initramfs (compressed root + init script)          │
└────────────────┬────────────────────────────────────────────┘
                 │
┌────────────────▼────────────────────────────────────────────┐
│ 7. Create Disk Image (ext4, 1GB)                            │
└────────────────┬────────────────────────────────────────────┘
                 │
┌────────────────▼────────────────────────────────────────────┐
│ 8. Build Applications and GUI                               │
└────────────────┬────────────────────────────────────────────┘
                 │
┌────────────────▼────────────────────────────────────────────┐
│ 9. Create ISO Image (bootable)                              │
└────────────────┬────────────────────────────────────────────┘
                 │
┌────────────────▼────────────────────────────────────────────┐
│ 10. Ready to Boot (dist/nexa.img)                           │
└─────────────────────────────────────────────────────────────┘
```

## GitHub Actions CI/CD

The `.github/workflows/ci.yml` provides:

### Automatic Builds
- Triggered on push, PR, or tag
- Matrix builds (default + minimal configs)
- Parallel compilation
- Artifact storage

### Testing
- Environment verification
- Binary validation
- Boot testing in QEMU

### Security
- Trivy vulnerability scanning
- CodeQL analysis
- SARIF report upload

### Releases
- Automatic release creation on tags
- Asset upload
- Release notes generation

### Container Registry
- Docker image builds
- Registry push (ghcr.io)
- Build cache optimization

**Triggering builds:**
```bash
# Push to main
git push origin main          # Triggers build

# Tag release
git tag v3.1.0
git push --tags              # Triggers release build
```

## Buildroot Integration

For modern Linux distro building, use Buildroot:

```bash
# Initialize Buildroot
make buildroot-init

# Configure
make buildroot-menuconfig

# Build
make buildroot-build

# Output in output/
```

The `buildroot.config` file pre-configures:
- Kernel (6.18.33)
- Toolchain (GCC 13, musl/glibc)
- Packages (Busybox, SSH, Python, etc.)
- Filesystems (ext4, ISO9660)
- Security hardening

## Troubleshooting

### Build fails
```bash
# Check environment
make check-env

# View full build log
cat build/build.log

# Clean and retry
make distclean
make all
```

### QEMU boot fails
```bash
# Verify kernel and images exist
ls -lh dist/{bzImage,initramfs.cpio.gz,nexa.img}

# Check QEMU installation
qemu-system-x86_64 --version

# Boot with verbose output
make boot VERBOSE=1
```

### Docker build issues
```bash
# Rebuild Docker image
docker build -t nexaos/build:latest --no-cache .

# Run with debug
docker run -it nexaos/build:latest bash
```

## Performance Optimization

### Parallel Compilation
```bash
# Auto-detect cores
make all

# Force specific job count
make all PARALLEL_JOBS=8
```

### Build Caching
```bash
# Use ccache
export PATH="/usr/lib/ccache:$PATH"
make all
```

### Incremental Builds
```bash
# Rebuild only changed components
make rootfs              # Only rootfs
make apps                # Only applications
```

## Modernization Benefits

| Old System | New System |
|-----------|-----------|
| Multiple shell scripts | Single Makefile |
| Scattered parameters | build.conf |
| Manual compilation | Automated tooling |
| No CI/CD | GitHub Actions |
| Non-reproducible | Docker containers |
| Manual version tracking | Semantic versioning |
| Limited error handling | Comprehensive logging |

## Next Steps

1. **Local Development**
   ```bash
   ./build.sh setup
   ./build.sh build
   ./build.sh boot
   ```

2. **GitHub Integration**
   - Push to GitHub
   - Enable Actions
   - Automated builds trigger

3. **Docker Hub**
   - Push build image
   - Enable auto-builds
   - Distribute containers

4. **Release Management**
   - Tag versions: `git tag v3.1.0`
   - Push tags: `git push --tags`
   - Automatic release creation

## Support & Documentation

- Build system: `make help` or `./build.sh help`
- Configuration: See `build.conf`
- Logs: Check `build/build.log`
- CI/CD: See `.github/workflows/ci.yml`

## License

NexaOS build system © 2024 Nexa Development Team
