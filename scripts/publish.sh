#!/usr/bin/env bash
set -e

# ==============================================================================
# Cargo Publish Automation Script for hexbuffer-proxy
# Reference: https://doc.rust-lang.org/cargo/commands/cargo-login.html
# ==============================================================================

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
BOLD='\033[1m'
NC='\033[0m' # No Color

# Defaults
DRY_RUN=false
ALLOW_DIRTY=false
SKIP_TESTS=false
NON_INTERACTIVE=false
BUMP_TYPE=""
CUSTOM_TOKEN=""
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

cd "$ROOT_DIR"

print_header() {
    echo -e "\n${BLUE}${BOLD}==> $1${NC}"
}

print_success() {
    echo -e "${GREEN}✓ $1${NC}"
}

print_warning() {
    echo -e "${YELLOW}⚠ $1${NC}"
}

print_error() {
    echo -e "${RED}✗ $1${NC}"
}

print_info() {
    echo -e "${CYAN}ℹ $1${NC}"
}

show_help() {
    cat << EOF
Usage: ./scripts/publish.sh [OPTIONS]

Easy Cargo / Crates.io publish automation script.

Options:
  --dry-run              Run pre-checks and packaging dry-run without uploading to crates.io
  --allow-dirty          Allow publishing with uncommitted git changes
  --skip-tests           Skip running tests before publishing
  --bump <type>          Bump version: patch | minor | major | <custom_version> (e.g. 0.0.3)
  --token <token>        Provide crates.io API token for cargo login
  -y, --yes              Non-interactive mode (auto-confirm prompts)
  -h, --help             Show this help message

Examples:
  ./scripts/publish.sh                      # Full interactive publish
  ./scripts/publish.sh --dry-run            # Test packaging locally
  ./scripts/publish.sh --bump patch         # Bump patch version & publish
  ./scripts/publish.sh --token <API_TOKEN>  # Login with token and publish
  make publish                              # Run publish via Makefile
  make publish-dry                          # Run dry-run via Makefile
EOF
}

# Parse CLI Arguments
while [[ $# -gt 0 ]]; do
    case "$1" in
        --dry-run)
            DRY_RUN=true
            shift
            ;;
        --allow-dirty)
            ALLOW_DIRTY=true
            shift
            ;;
        --skip-tests)
            SKIP_TESTS=true
            shift
            ;;
        --bump)
            BUMP_TYPE="$2"
            shift 2
            ;;
        --token)
            CUSTOM_TOKEN="$2"
            shift 2
            ;;
        -y|--yes)
            NON_INTERACTIVE=true
            shift
            ;;
        -h|--help)
            show_help
            exit 0
            ;;
        *)
            print_error "Unknown option: $1"
            show_help
            exit 1
            ;;
    esac
done

# If not running in a terminal, set NON_INTERACTIVE
if [ ! -t 0 ]; then
    NON_INTERACTIVE=true
fi

echo -e "${CYAN}${BOLD}"
echo "  _                _             __  __                                 "
echo " | |__   _____  __| |__  _   _ / _|/ _| ___ _ __       _ __  _ __ _____  _   _ "
echo " | '_ \ / _ \ \/ /| '_ \| | | | |_| |_ / _ \ '__|____ | '_ \| '__/ _ \ \/ / | | |"
echo " | | | |  __/>  < | |_) | |_| |  _|  _|  __/ | |_____|| |_) | | | (_) >  <| |_| |"
echo " |_| |_|\___/_/\_\|_.__/ \__,_|_| |_|  \___|_|        | .__/|_|  \___/_/\_\__, |"
echo "                                                      |_|                  |___/ "
echo -e "${NC}"
print_info "Starting Cargo publish workflow in ${ROOT_DIR}"

# ------------------------------------------------------------------------------
# 1. Load .env & Cargo Authentication Check (cargo login)
# Reference: https://doc.rust-lang.org/cargo/commands/cargo-login.html
# ------------------------------------------------------------------------------
print_header "Step 1: Checking Crates.io Authentication"

# Auto-load token from .env if present
if [[ -f "$ROOT_DIR/.env" ]]; then
    # Extract token variables from .env without executing arbitrary code
    ENV_TOKEN=$(grep -E '^(CRATE_TOKEN|CRATES_TOKEN|CARGO_TOKEN|CARGO_REGISTRY_TOKEN|CRATES_IO_TOKEN)=' "$ROOT_DIR/.env" | head -n1 | cut -d '=' -f2- | tr -d '"' | tr -d "'" | tr -d '\r' | tr -d ' ')
    if [[ -n "$ENV_TOKEN" ]]; then
        print_info "Found crate token in .env file."
        CUSTOM_TOKEN="$ENV_TOKEN"
    fi
fi

CARGO_CRED_TOML="$HOME/.cargo/credentials.toml"
CARGO_CRED_PLAIN="$HOME/.cargo/credentials"

has_token() {
    if [[ -n "$CARGO_REGISTRY_TOKEN" ]]; then
        return 0
    fi
    if [[ -f "$CARGO_CRED_TOML" ]] && grep -q 'secret-key\|token' "$CARGO_CRED_TOML" 2>/dev/null; then
        return 0
    fi
    if [[ -f "$CARGO_CRED_PLAIN" ]] && grep -q 'token' "$CARGO_CRED_PLAIN" 2>/dev/null; then
        return 0
    fi
    return 1
}

if [[ -n "$CUSTOM_TOKEN" ]]; then
    print_info "Logging in with crate token..."
    echo "$CUSTOM_TOKEN" | cargo login --quiet
    print_success "Logged in successfully to crates.io using token."
elif has_token; then
    print_success "Crates.io credentials found."
elif [ "$DRY_RUN" = true ]; then
    print_info "No token found, but running in --dry-run mode (authentication not required for local packaging)."
elif [ "$NON_INTERACTIVE" = true ]; then
    print_warning "No crates.io token found in non-interactive mode. Proceeding (publish will use cargo default / CARGO_REGISTRY_TOKEN)."
else
    print_warning "No crates.io authentication token found in cargo credentials or .env."
    print_info "You can add CRATE_TOKEN=<token> to .env or generate one at https://crates.io/settings/tokens"
    echo -n -e "${BOLD}Would you like to enter your crates.io token now? (y/N): ${NC}"
    read -r do_login
    if [[ "$do_login" =~ ^[Yy]$ ]]; then
        echo -n -e "${BOLD}Paste crates.io API token: ${NC}"
        read -r -s input_token
        echo ""
        if [[ -n "$input_token" ]]; then
            echo "$input_token" | cargo login --quiet
            print_success "Logged in successfully to crates.io."
        else
            print_error "Empty token provided."
            exit 1
        fi
    else
        print_warning "Proceeding without logging in now. (Publish may fail if not already authenticated)"
    fi
fi

# ------------------------------------------------------------------------------
# 2. Working Directory & Git Status
# ------------------------------------------------------------------------------
print_header "Step 2: Checking Git Status"

if ! git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    print_warning "Not inside a git repository."
else
    GIT_STATUS=$(git status --porcelain)
    if [[ -n "$GIT_STATUS" ]]; then
        if [ "$ALLOW_DIRTY" = true ]; then
            print_warning "Working directory is dirty, but --allow-dirty was specified. Continuing."
        elif [ "$NON_INTERACTIVE" = true ]; then
            print_warning "Working directory has uncommitted changes. Using --allow-dirty in non-interactive mode."
            ALLOW_DIRTY=true
        else
            print_warning "Working directory has uncommitted changes:"
            git status --short
            echo -n -e "${BOLD}Do you want to proceed with uncommitted changes (--allow-dirty)? (y/N): ${NC}"
            read -r allow_dirty_input
            if [[ "$allow_dirty_input" =~ ^[Yy]$ ]]; then
                ALLOW_DIRTY=true
                print_info "Proceeding with --allow-dirty enabled."
            else
                print_error "Aborted by user. Please commit or stash your changes first."
                exit 1
            fi
        fi
    else
        print_success "Git working tree is clean."
    fi
fi

# ------------------------------------------------------------------------------
# 3. Version Bump / Verification
# ------------------------------------------------------------------------------
print_header "Step 3: Checking Package Version"

CURRENT_VERSION=$(grep -m1 '^version' Cargo.toml | sed -E 's/version = "(.*)"/\1/')
print_info "Current version in Cargo.toml: ${BOLD}${CURRENT_VERSION}${NC}"

bump_semver() {
    local version="$1"
    local type="$2"
    local major minor patch
    IFS='.' read -r major minor patch <<< "$version"

    case "$type" in
        major)
            echo "$((major + 1)).0.0"
            ;;
        minor)
            echo "${major}.$((minor + 1)).0"
            ;;
        patch)
            echo "${major}.${minor}.$((patch + 1))"
            ;;
        *)
            echo "$type"
            ;;
    esac
}

NEW_VERSION="$CURRENT_VERSION"

if [[ -n "$BUMP_TYPE" ]]; then
    NEW_VERSION=$(bump_semver "$CURRENT_VERSION" "$BUMP_TYPE")
elif [ "$NON_INTERACTIVE" = true ]; then
    NEW_VERSION="$CURRENT_VERSION"
else
    echo -e "\nSelect version option:"
    echo "  1) Keep current (${CURRENT_VERSION})"
    echo "  2) Bump patch   ($(bump_semver "$CURRENT_VERSION" "patch"))"
    echo "  3) Bump minor   ($(bump_semver "$CURRENT_VERSION" "minor"))"
    echo "  4) Bump major   ($(bump_semver "$CURRENT_VERSION" "major"))"
    echo "  5) Enter custom version"
    echo -n -e "${BOLD}Choice [1-5] (default: 1): ${NC}"
    read -r choice

    case "$choice" in
        2) NEW_VERSION=$(bump_semver "$CURRENT_VERSION" "patch") ;;
        3) NEW_VERSION=$(bump_semver "$CURRENT_VERSION" "minor") ;;
        4) NEW_VERSION=$(bump_semver "$CURRENT_VERSION" "major") ;;
        5)
            echo -n -e "${BOLD}Enter new version: ${NC}"
            read -r custom_v
            if [[ -n "$custom_v" ]]; then
                NEW_VERSION="$custom_v"
            fi
            ;;
        *) NEW_VERSION="$CURRENT_VERSION" ;;
    esac
fi

if [[ "$NEW_VERSION" != "$CURRENT_VERSION" ]]; then
    print_info "Updating version in Cargo.toml from ${CURRENT_VERSION} to ${NEW_VERSION}..."
    if [[ "$OSTYPE" == "darwin"* ]]; then
        sed -i '' "s/^version = \"${CURRENT_VERSION}\"/version = \"${NEW_VERSION}\"/" Cargo.toml
    else
        sed -i "s/^version = \"${CURRENT_VERSION}\"/version = \"${NEW_VERSION}\"/" Cargo.toml
    fi
    cargo check --quiet >/dev/null 2>&1 || true
    print_success "Cargo.toml updated to v${NEW_VERSION}."
fi

# ------------------------------------------------------------------------------
# 4. Code Quality & Test Suite
# ------------------------------------------------------------------------------
print_header "Step 4: Running Quality Checks & Tests"

if [ "$SKIP_TESTS" = false ]; then
    print_info "Running 'cargo test'..."
    if cargo test --quiet; then
        print_success "All tests passed."
    else
        print_error "Tests failed! Aborting publish."
        exit 1
    fi
else
    print_warning "Skipping tests as requested."
fi

# ------------------------------------------------------------------------------
# 5. Packaging Dry-Run Check
# ------------------------------------------------------------------------------
print_header "Step 5: Validating Packaging (Dry Run)"

EXTRA_FLAGS=""
if [ "$ALLOW_DIRTY" = true ]; then
    EXTRA_FLAGS="--allow-dirty"
fi

print_info "Running 'cargo publish --dry-run ${EXTRA_FLAGS}'..."
if cargo publish --dry-run $EXTRA_FLAGS; then
    print_success "Packaging dry-run succeeded."
else
    print_error "Packaging dry-run failed. Please inspect the errors above."
    exit 1
fi

if [ "$DRY_RUN" = true ]; then
    print_header "Dry Run Completed"
    print_success "Dry run passed cleanly! No packages were uploaded to crates.io."
    exit 0
fi

# ------------------------------------------------------------------------------
# 6. Confirmation & Publishing
# ------------------------------------------------------------------------------
print_header "Step 6: Ready to Publish to Crates.io"

echo -e "${BOLD}Package:${NC} hexbuffer-proxy"
echo -e "${BOLD}Version:${NC} ${NEW_VERSION}"
echo -e "${BOLD}Target :${NC} crates.io"

if [ "$NON_INTERACTIVE" = false ]; then
    echo -n -e "\n${BOLD}${YELLOW}Are you sure you want to publish hexbuffer-proxy v${NEW_VERSION} to crates.io? (y/N): ${NC}"
    read -r confirm_publish

    if [[ ! "$confirm_publish" =~ ^[Yy]$ ]]; then
        print_warning "Publishing cancelled by user."
        exit 0
    fi
fi

print_info "Publishing to crates.io..."
if cargo publish $EXTRA_FLAGS; then
    print_success "🎉 hexbuffer-proxy v${NEW_VERSION} published successfully to crates.io!"
else
    print_error "Failed to publish to crates.io."
    exit 1
fi

# ------------------------------------------------------------------------------
# 7. Git Tagging & Commit (Optional)
# ------------------------------------------------------------------------------
if git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    print_header "Step 7: Git Tagging & Release"
    TAG_NAME="v${NEW_VERSION}"
    
    do_tag=false
    if [ "$NON_INTERACTIVE" = true ]; then
        do_tag=false
    else
        echo -n -e "${BOLD}Create and push git tag '${TAG_NAME}'? (y/N): ${NC}"
        read -r make_tag
        if [[ "$make_tag" =~ ^[Yy]$ ]]; then
            do_tag=true
        fi
    fi

    if [ "$do_tag" = true ]; then
        if [[ "$NEW_VERSION" != "$CURRENT_VERSION" ]]; then
            git add Cargo.toml Cargo.lock 2>/dev/null || git add Cargo.toml
            git commit -m "release: ${TAG_NAME}" || true
        fi
        git tag -a "$TAG_NAME" -m "Release ${TAG_NAME}"
        print_success "Created git tag ${TAG_NAME}"
        
        echo -n -e "${BOLD}Push commit and tag to origin? (y/N): ${NC}"
        read -r push_tag
        if [[ "$push_tag" =~ ^[Yy]$ ]]; then
            git push origin HEAD --tags
            print_success "Pushed changes and tag to origin."
        fi
    fi
fi

print_header "All Done!"
print_success "Release workflow for hexbuffer-proxy v${NEW_VERSION} completed."
