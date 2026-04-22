#!/usr/bin/env bash
set -euo pipefail

# Rollback script for TipJar contract deployments
# Usage: ./scripts/rollback.sh <NETWORK> [--apply]
#
# Soroban contracts are immutable, so "rollback" means switching
# the active contract address reference to a previous deployment.

if [ $# -lt 1 ]; then
  echo "Usage: $0 <NETWORK> [--apply]"
  echo "Example: $0 testnet"
  echo "Example: $0 mainnet --apply"
  echo ""
  echo "Options:"
  echo "  --apply    Actually update addresses.json with the rollback"
  exit 1
fi

NETWORK="$1"
APPLY_ROLLBACK=false

if [ $# -eq 2 ] && [ "$2" == "--apply" ]; then
  APPLY_ROLLBACK=true
fi

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ADDRESSES_FILE="$ROOT_DIR/deployment/addresses.json"

echo "========================================="
echo "TipJar Contract Rollback"
echo "========================================="
echo "Network: $NETWORK"
echo ""

# Validate network
if [ "$NETWORK" != "testnet" ] && [ "$NETWORK" != "mainnet" ]; then
  echo "❌ Error: Network must be 'testnet' or 'mainnet'"
  exit 1
fi

# Check if addresses.json exists
if [ ! -f "$ADDRESSES_FILE" ]; then
  echo "❌ Error: Deployment registry not found at $ADDRESSES_FILE"
  exit 1
fi

# Check if jq is installed
if ! command -v jq &> /dev/null; then
  echo "❌ Error: jq is required for rollback operations"
  echo "Install with: sudo apt-get install jq (Ubuntu/Debian) or brew install jq (macOS)"
  exit 1
fi

# Get current contract ID
CURRENT_ID=$(jq -r ".${NETWORK}.current // empty" "$ADDRESSES_FILE")

if [ -z "$CURRENT_ID" ] || [ "$CURRENT_ID" == "null" ]; then
  echo "❌ Error: No current deployment found for $NETWORK"
  exit 1
fi

echo "Current contract ID: $CURRENT_ID"
echo ""

# Get deployment history
HISTORY_LENGTH=$(jq ".${NETWORK}.history | length" "$ADDRESSES_FILE")

if [ "$HISTORY_LENGTH" -lt 2 ]; then
  echo "❌ Error: No previous deployment found for rollback"
  echo "   History contains only $HISTORY_LENGTH deployment(s)"
  exit 1
fi

# Get the previous deployment (second to last in history)
PREVIOUS_INDEX=$((HISTORY_LENGTH - 2))
PREVIOUS_ID=$(jq -r ".${NETWORK}.history[$PREVIOUS_INDEX].contract_id" "$ADDRESSES_FILE")
PREVIOUS_TIMESTAMP=$(jq -r ".${NETWORK}.history[$PREVIOUS_INDEX].timestamp" "$ADDRESSES_FILE")

echo "📋 Rollback Target:"
echo "   Contract ID: $PREVIOUS_ID"
echo "   Deployed at: $PREVIOUS_TIMESTAMP"
echo ""

# Display all available versions
echo "📜 Deployment History:"
jq -r ".${NETWORK}.history[] | \"   [\(.timestamp)] \(.contract_id)\"" "$ADDRESSES_FILE"
echo ""

# Apply rollback if requested
if [ "$APPLY_ROLLBACK" = true ]; then
  echo "⚠️  Applying rollback..."
  
  if [ "$NETWORK" == "mainnet" ]; then
    echo "⚠️  WARNING: You are rolling back MAINNET"
    read -p "Type 'yes' to confirm: " CONFIRMATION
    if [ "$CONFIRMATION" != "yes" ]; then
      echo "❌ Rollback cancelled"
      exit 1
    fi
  fi
  
  # Update current to previous
  jq --arg id "$PREVIOUS_ID" \
    ".${NETWORK}.current = \$id" \
    "$ADDRESSES_FILE" > "${ADDRESSES_FILE}.tmp" && mv "${ADDRESSES_FILE}.tmp" "$ADDRESSES_FILE"
  
  echo "✅ Rollback applied: $ADDRESSES_FILE updated"
  echo ""
else
  echo "ℹ️  Dry-run mode (use --apply to actually rollback)"
  echo ""
fi

# Instructions
echo "========================================="
echo "📝 Rollback Instructions"
echo "========================================="
echo ""
echo "To complete the rollback, update your application to use:"
echo "   Contract ID: $PREVIOUS_ID"
echo ""
echo "Steps:"
echo "  1. Update frontend environment variables:"
echo "     VITE_CONTRACT_ID=$PREVIOUS_ID"
echo ""
echo "  2. Update SDK initialization:"
echo "     const contract = new TipJarContract('$PREVIOUS_ID', '$NETWORK');"
echo ""
echo "  3. Redeploy your frontend application"
echo ""
echo "  4. Verify the rollback:"
echo "     ./scripts/verify_deployment.sh $PREVIOUS_ID $NETWORK"
echo ""
echo "  5. Monitor the contract on Stellar Expert:"

if [ "$NETWORK" == "mainnet" ]; then
  echo "     https://stellar.expert/explorer/public/contract/$PREVIOUS_ID"
else
  echo "     https://stellar.expert/explorer/testnet/contract/$PREVIOUS_ID"
fi
echo ""

if [ "$APPLY_ROLLBACK" = false ]; then
  echo "To apply this rollback, run:"
  echo "   $0 $NETWORK --apply"
  echo ""
fi
