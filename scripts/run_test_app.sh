#!/usr/bin/env bash
# scripts/run_test_app.sh — Build and run the Hexbuffer Proxy interactive test web app

set -euo pipefail

PROXY_PORT=8080
DASHBOARD_PORT=8081
UPSTREAM_PORT=8082

BUILD_MODE="debug"
CARGO_FLAGS=("--example" "test_app" "--features" "decoder")
AUTO_OPEN=true
LAUNCH_CHROME=false
BUILD_ONLY=false

# ── Argument Parsing ─────────────────────────────────────────────────────────
show_help() {
    cat <<EOF
Usage: ./scripts/run_test_app.sh [OPTIONS]

Builds and runs the Hexbuffer Proxy interactive web test dashboard.

Options:
  --release         Build and run in release mode (optimized)
  --no-open         Do not automatically open browser on startup
  --chrome          Open an isolated Chrome window configured to use the proxy
  --build-only      Only compile the example binary without starting the servers
  -h, --help        Show this help message

Ports Used:
  - $PROXY_PORT: MITM Proxy listener
  - $DASHBOARD_PORT: Web Inspector & Dashboard UI
  - $UPSTREAM_PORT: Local mock echo upstream server
EOF
    exit 0
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --release)
            BUILD_MODE="release"
            CARGO_FLAGS+=("--release")
            shift
            ;;
        --no-open)
            AUTO_OPEN=false
            shift
            ;;
        --chrome)
            LAUNCH_CHROME=true
            shift
            ;;
        --build-only)
            BUILD_ONLY=true
            shift
            ;;
        -h|--help)
            show_help
            ;;
        *)
            echo "Unknown option: $1"
            show_help
            ;;
    esac
done

# Navigate to workspace root
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

echo "================================================================"
echo "  HEXBUFFER PROXY — Interactive Web Test Application Launcher   "
echo "================================================================"

# ── Step 1: Free Stale Ports ──────────────────────────────────────────────────
echo "[1/3] Checking and freeing ports ($PROXY_PORT, $DASHBOARD_PORT, $UPSTREAM_PORT)..."
if command -v lsof &>/dev/null; then
    PIDS=$(lsof -ti :$PROXY_PORT -ti :$DASHBOARD_PORT -ti :$UPSTREAM_PORT 2>/dev/null || true)
    if [[ -n "$PIDS" ]]; then
        echo "Killing stale processes on ports: $PIDS"
        echo "$PIDS" | xargs kill -9 2>/dev/null || true
    fi
fi

# ── Step 2: Build Example Binary ─────────────────────────────────────────────
echo "[2/3] Compiling test_app example ($BUILD_MODE mode with decoder feature)..."
cargo build "${CARGO_FLAGS[@]}"

if [[ "$BUILD_ONLY" == true ]]; then
    echo "Build complete! (Run without --build-only to launch the server)"
    exit 0
fi

# ── Step 3: Run Test Application ─────────────────────────────────────────────
echo "[3/3] Starting servers..."

BINARY_PATH="./target/${BUILD_MODE}/examples/test_app"

# Helper to open URL based on OS
open_browser() {
    local url="$1"
    if [[ "$OSTYPE" == "darwin"* ]]; then
        open "$url"
    elif [[ "$OSTYPE" == "linux-gnu"* ]] && command -v xdg-open &>/dev/null; then
        xdg-open "$url" &>/dev/null || true
    fi
}

# Background worker to wait for server readiness and optionally open browser
(
    # Wait for dashboard port to be listening
    for i in {1..30}; do
        if curl -s -f "http://127.0.0.1:${DASHBOARD_PORT}/api/status" &>/dev/null; then
            break
        fi
        sleep 0.5
    done

    echo ""
    echo "🚀 All services ready!"
    echo "   - Dashboard UI:  http://127.0.0.1:${DASHBOARD_PORT}"
    echo "   - MITM Proxy:    http://127.0.0.1:${PROXY_PORT}"
    echo "   - Mock Upstream: http://127.0.0.1:${UPSTREAM_PORT}"
    echo "   - Root CA Cert:  http://127.0.0.1:${DASHBOARD_PORT}/ca.pem"
    echo ""

    if [[ "$AUTO_OPEN" == true && "$LAUNCH_CHROME" == false ]]; then
        echo "Opening dashboard in your default browser..."
        open_browser "http://127.0.0.1:${DASHBOARD_PORT}"
    fi

    if [[ "$LAUNCH_CHROME" == true ]]; then
        if [[ -d "/Applications/Google Chrome.app" ]]; then
            echo "Launching isolated Chrome instance with proxy 127.0.0.1:${PROXY_PORT}..."
            /Applications/Google\ Chrome.app/Contents/MacOS/Google\ Chrome \
                --proxy-server="http://127.0.0.1:${PROXY_PORT}" \
                --user-data-dir="/tmp/chrome-hexbuffer-test-profile" \
                "http://127.0.0.1:${DASHBOARD_PORT}" &>/dev/null &
        else
            echo "Google Chrome not found at /Applications/Google Chrome.app, opening default browser..."
            open_browser "http://127.0.0.1:${DASHBOARD_PORT}"
        fi
    fi
) &

# Execute the application in the foreground so Ctrl+C gracefully stops it
exec "$BINARY_PATH"
