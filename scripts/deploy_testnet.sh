#!/usr/bin/env bash
set -euo pipefail

# Testnet deployment script for TipJar contract
# Usage: DEPLOYER_SECRET=<secret> [WEBHOOK_URL=<url>] ./scripts/deploy_testnet.sh

NETWORK="testnet"
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEPLOYMENT_DIR="$ROOT_DIR/deployment"
ADDRESSES_FILE="$DEPLOYMENT_DIR/addresses.json"
VERIFY_SCRIPT="$ROOT_DIR/scripts/verify_deployment.sh"

echo "========================================="
echo "TipJar Testnet Deployment Pipeline"
echo "========================================="
echo ""

# Validate environment variables
if [ -z "${DEPLOYER_SECRET:-}" ]; then
  echo "❌ Error: DEPLOYER_SECRET environment variable is required"
  exit 1
fi

# Step 1: Build contract
echo "📦 Building contract..."
cd "$ROOT_DIR"
cargo build -p tipjar --target wasm32v1-none --release

WASM_PATH="$ROOT_DIR/target/wasm32v1-none/release/tipjar.wasm"
if [ ! -f "$WASM_PATH" ]; then
  echo "❌ Error: WASM file not found at $WASM_PATH"
  exit 1
fi
echo "✅ Build complete: $WASM_PATH"
echo ""

# Step 2: Optimize WASM
echo "⚡ Optimizing WASM..."
stellar contract optimize --wasm "$WASM_PATH"

OPTIMIZED_WASM="${WASM_PATH%.wasm}_optimized.wasm"
if [ ! -f "$OPTIMIZED_WASM" ]; then
  echo "❌ Error: Optimized WASM not found at $OPTIMIZED_WASM"
  exit 1
fi
echo "✅ Optimization complete: $OPTIMIZED_WASM"
echo ""

# Step 3: Deploy to testnet
echo "🚀 Deploying to testnet..."
CONTRACT_ID=$(stellar contract deploy \
  --wasm "$OPTIMIZED_WASM" \
  --source "$DEPLOYER_SECRET" \
  --network "$NETWORK" 2>&1 | tee /dev/tty | tail -n1)

if [ -z "$CONTRACT_ID" ]; then
  echo "❌ Error: Deployment failed - no contract ID returned"
  exit 1
fi

echo "✅ Contract deployed: $CONTRACT_ID"
echo ""

# Step 4: Verify deployment
echo "🔍 Verifying deployment..."
if [ -x "$VERIFY_SCRIPT" ]; then
  "$VERIFY_SCRIPT" "$CONTRACT_ID" "$NETWORK"
else
  echo "⚠️  Warning: verify_deployment.sh not found or not executable, skipping verification"
fi
echo ""

# Step 5: Update addresses.json
echo "📝 Updating deployment registry..."
TIMESTAMP=$(date -u +"%Y-%m-%dT%H:%M:%SZ")

if command -v jq &> /dev/null; then
  # Create deployment directory if it doesn't exist
  mkdir -p "$DEPLOYMENT_DIR"
  
  # Initialize addresses.json if it doesn't exist
  if [ ! -f "$ADDRESSES_FILE" ]; then
    echo '{"testnet":{"current":null,"history":[]},"mainnet":{"current":null,"history":[]}}' > "$ADDRESSES_FILE"
  fi
  
  # Update with new deployment
  jq --arg id "$CONTRACT_ID" --arg ts "$TIMESTAMP" \
    '.testnet.history += [{"contract_id": $id, "timestamp": $ts}] | .testnet.current = $id' \
    "$ADDRESSES_FILE" > "${ADDRESSES_FILE}.tmp" && mv "${ADDRESSES_FILE}.tmp" "$ADDRESSES_FILE"
  
  echo "✅ Registry updated: $ADDRESSES_FILE"
else
  echo "⚠️  Warning: jq not installed, skipping registry update"
fi
echo ""

# Step 6: Run smoke tests
echo "🧪 Running smoke tests..."
if cargo test -p tipjar -- --test-threads=1 2>&1 | grep -E "(test result|running)"; then
  echo "✅ Smoke tests passed"
else
  echo "⚠️  Warning: Some tests may have failed, check output above"
fi
echo ""

# Step 7: Send notification
if [ -n "${WEBHOOK_URL:-}" ]; then
  echo "📢 Sending deployment notification..."
  
  PAYLOAD=$(cat <<EOF
{
  "content": "🚀 **TipJar Testnet Deployment Complete**",
  "embeds": [{
    "title": "Deployment Details",
    "color": 3066993,
    "fields": [
      {"name": "Network", "value": "Testnet", "inline": true},
      {"name": "Contract ID", "value": "\`$CONTRACT_ID\`", "inline": false},
      {"name": "Timestamp", "value": "$TIMESTAMP", "inline": true},
      {"name": "Explorer", "value": "[View on Stellar Expert](https://stellar.expert/explorer/testnet/contract/$CONTRACT_ID)", "inline": false}
    ]
  }]
}
EOF
)
  
  if curl -H "Content-Type: application/json" \
       -X POST \
       -d "$PAYLOAD" \
       "$WEBHOOK_URL" \
       --silent --show-error --fail > /dev/null; then
    echo "✅ Notification sent"
  else
    echo "⚠️  Warning: Failed to send notification"
  fi
else
  echo "ℹ️  Skipping notification (WEBHOOK_URL not set)"
fi
echo ""

# Summary
echo "========================================="
echo "✅ Testnet Deployment Complete"
echo "========================================="
echo "Contract ID: $CONTRACT_ID"
echo "Network: testnet"
echo "Timestamp: $TIMESTAMP"
echo "Explorer: https://stellar.expert/explorer/testnet/contract/$CONTRACT_ID"
echo ""
echo "Next steps:"
echo "  1. Update your frontend/SDK to use this contract ID"
echo "  2. Run integration tests against the deployed contract"
echo "  3. Monitor the contract on Stellar Expert"
echo ""
