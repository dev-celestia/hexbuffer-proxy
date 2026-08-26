# Scripts Documentation & Release Notes

This directory contains automation, benchmarking, and release management scripts for `hexbuffer-proxy`.

---

## 1. `publish.sh` — Cargo / Crates.io Publishing Automation

The `publish.sh` script automates and safeguards the release workflow to [crates.io](https://crates.io/).

### Prerequisites & Token Configuration
You can supply your Crates.io API token in any of the following ways:
1. **`.env` file (Recommended)**:
   Add your token to `/Users/arham/Desktop/project/hexbuffer-proxy/.env`:
   ```dotenv
   CRATE_TOKEN=cioz...your_crates_io_token...
   ```
   > **Note:** `.env` is listed in `.gitignore` to prevent leaking tokens.

2. **Command Line argument**:
   ```bash
   ./scripts/publish.sh --token <YOUR_TOKEN>
   ```

3. **Global Cargo Credentials**:
   Login once with standard cargo:
   ```bash
   cargo login <YOUR_TOKEN>
   ```

4. **Environment Variable**:
   ```bash
   export CARGO_REGISTRY_TOKEN=<YOUR_TOKEN>
   ```

---

### Usage & Commands

```bash
# Interactive publish workflow (Recommended)
./scripts/publish.sh
# or via Makefile
make publish

# Dry-run validation (checks compilation, packaging, and tests without uploading)
./scripts/publish.sh --dry-run
# or via Makefile
make publish-dry

# Automatic version bump + publish
./scripts/publish.sh --bump patch      # e.g., 0.0.2 -> 0.0.3
./scripts/publish.sh --bump minor      # e.g., 0.0.2 -> 0.1.0
./scripts/publish.sh --bump major      # e.g., 0.0.2 -> 1.0.0
./scripts/publish.sh --bump 0.1.5      # Specific custom version

# Skip test suite (if already run)
./scripts/publish.sh --skip-tests

# Allow uncommitted git changes
./scripts/publish.sh --allow-dirty

# Non-interactive / CI mode
./scripts/publish.sh --yes --dry-run
```

---

### Step-by-Step Workflow Executed by `publish.sh`

1. **Step 1: Crates.io Authentication**:
   - Detects `CRATE_TOKEN` in `.env` or existing cargo credentials.
   - Automatically executes `cargo login` if a token is supplied.
2. **Step 2: Git Status Check**:
   - Ensures working tree is clean to prevent untracked state from being packaged.
3. **Step 3: Version Bump**:
   - Displays current version from `Cargo.toml`.
   - Offers interactive SemVer bump (patch / minor / major / custom).
4. **Step 4: Quality & Test Suite**:
   - Runs `cargo test` across all unit and doc tests.
5. **Step 5: Packaging Dry Run**:
   - Runs `cargo publish --dry-run` to compile and verify packaged crate contents.
6. **Step 6: Release Confirmation**:
   - Prompts for explicit confirmation before uploading package to crates.io.
7. **Step 7: Git Tag & Push**:
   - Optionally commits updated `Cargo.toml`, creates a git tag (`vX.Y.Z`), and pushes tag to remote.

---

## 2. `load_test.sh` — Performance & Load Testing

A benchmarking script to test proxy throughput and concurrency under load.

### Usage
```bash
./scripts/load_test.sh
```

### Features
- Builds the optimized release binary (`cargo build --release`).
- Automatically launches the proxy on port `8080`.
- Benchmarks using:
  - `hey` (if installed)
  - `wrk` (if installed)
  - Multi-threaded `curl` loop (fallback)
- Auto-terminates background proxy process on exit.

---

## 3. Makefile Quick Reference

| Command | Action |
|---|---|
| `make publish` | Runs `./scripts/publish.sh` (Interactive publish) |
| `make publish-dry` | Runs `./scripts/publish.sh --dry-run` (Safe dry-run check) |
| `make run` | Kills port 8080 and runs the example proxy |
| `make build` | Builds debug target |
| `make release` | Builds optimized release target |
| `make test` | Runs the test suite |
| `make check` | Runs `cargo check` |
| `make clean` | Cleans cargo build target artifacts |
