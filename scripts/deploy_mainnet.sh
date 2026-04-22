#!/usr/bin/env bash
set -euo pipefail

# Mainnet deployment script for TipJar contract
# Usage: MAINNET_DEPLOYER_SECRET=<secret> [WEBHOOK_URL=<url>] ./scripts/deploy_mainnet.sh

NETWORK="mainnet"
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEPLOYMENT_DIR="$ROOT_DIR/deployment"
ADDRESSES_FILE="$DEPLOYMENT_DIR/addresses.json"
VERIFY_SCRIPT="$ROOT_DIR/scripts/verify_deployment.sh"

echo "========================================="
echo "⚠️  TipJar MAINNET Deployment Pipeline"
echo "========================================="
echo ""
echo "⚠️  WARNING: You are about to deploy to MAINNET"
echo "⚠️  This will use real funds and cannot be undone"
echo ""

# Validate environment variables
if [ -z "${MAINNET_DEPLOYER_SECRET:-}" ]; then
  echo "❌ Error: MAINNET_DEPLOYER_SECRET environment variable is required"
  exit 1
fi

# Confirmation prompt
read -p "Deploy to MAINNET? Type 'yes' to confirm: " CONFIRMATION
if [ "$CONFIRMATION" != "yes" ]; then
  echo "❌ Deployment cancelled"
  exit 1
fi
echo ""

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

# Step 3: Deploy to mainnet
echo "🚀 Deploying to MAINNET..."
CONTRACT_ID=$(stellar contract deploy \
  --wasm "$OPTIMIZED_WASM" \
  --source "$MAINNET_DEPLOYER_SECRET" \
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
    '.mainnet.history += [{"contract_id": $id, "timestamp": $ts}] | .mainnet.current = $id' \
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
  "content": "🚀 **TipJar MAINNET Deployment Complete** @here",
  "embeds": [{
    "title": "Production Deployment Details",
    "color": 15158332,
    "fields": [
      {"name": "Network", "value": "⚠️ **MAINNET**", "inline": true},
      {"name": "Contract ID", "value": "\`$CONTRACT_ID\`", "inline": false},
      {"name": "Timestamp", "value": "$TIMESTAMP", "inline": true},
      {"name": "Explorer", "value": "[View on Stellar Expert](https://stellar.expert/explorer/public/contract/$CONTRACT_ID)", "inline": false}
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
echo "✅ MAINNET Deployment Complete"
echo "========================================="
echo "Contract ID: $CONTRACT_ID"
echo "Network: mainnet"
echo "Timestamp: $TIMESTAMP"
echo "Explorer: https://stellar.expert/explorer/public/contract/$CONTRACT_ID"
echo ""
echo "⚠️  CRITICAL NEXT STEPS:"
echo "  1. Verify the contract on Stellar Expert"
echo "  2. Run comprehensive integration tests"
echo "  3. Update production frontend/SDK configuration"
echo "  4. Monitor contract activity closely"
echo "  5. Keep this contract ID in a secure location"
echo ""
