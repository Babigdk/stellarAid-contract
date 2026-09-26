# StellarAid Admin Multi-Sig Readiness & Verification Guide

## 1. Executive Summary

In StellarAid contracts (including the Campaign contract), administrative access is governed by an opaque `admin: Address` and enforced via `admin.require_auth()`.

Because Soroban delegates signature and threshold verification directly to the underlying Stellar host environment and Stellar Core, **the contract is natively multi-sig ready with zero smart contract modifications or redesign required**.

When deploying to Stellar Mainnet or Testnet, an admin address can be either:
1. **A Stellar Classic Multi-Sig Account (`G...`)**: An account configured on the Stellar ledger with $M$-of-$N$ threshold signers (e.g., 2-of-3 or 3-of-5).
2. **A Smart Contract Account (`C...`)**: A dedicated multi-sig contract (such as `contracts/multi_sig` or a smart contract wallet).

---

## 2. Architecture & Native Multi-Sig Mechanism

### How `Address::require_auth()` Interacts with Stellar Multi-Sig

1. **Transaction Envelope Level Validation**:
   When a transaction invoking an admin-guarded method (`freeze`, `unfreeze`, `pause`, `unpause`, `set_admin`, `upgrade`, `archive_campaign`, etc.) is submitted to the Stellar network:
   - The transaction envelope carries one or more cryptographic signatures.
   - For an account address (`G...`), Stellar Core evaluates whether the sum of the signer weights whose signatures are present in the transaction envelope meets or exceeds the account's **Medium threshold** (the threshold required for Soroban contract invocations).
   - If the combined weight satisfies the Medium threshold, the invocation is authorized.
   - If the combined weight is below the threshold, Stellar Core rejects the transaction before or during host authentication with an auth failure.

2. **Zero Contract Redesign**:
   The contract code simply executes:
   ```rust
   let admin: Address = env.storage().instance().get(&DataKey::Admin).unwrap();
   admin.require_auth();
   ```
   No custom signer tracking, proposal loops, or off-chain signature cryptography are needed inside the Campaign contract itself.

---

## 3. Mainnet / Testnet Multi-Sig Setup Runbook

The following steps demonstrate how to configure an $M$-of-$N$ multi-sig account on Stellar (e.g., a 2-of-3 configuration) and use it as the contract admin.

### Step 1: Generate Keypairs for Admin Signers
Generate keypairs for the signers and the multi-sig coordination account:

```bash
# Generate 3 individual signer keys
stellar keys generate signer1 --network testnet
stellar keys generate signer2 --network testnet
stellar keys generate signer3 --network testnet

# Generate the main admin account key
stellar keys generate multisig-admin --network testnet
```

### Step 2: Fund the Multi-Sig Admin Account
Fund the account (via Friendbot on testnet, or via XLM transfer on mainnet):

```bash
stellar keys fund multisig-admin --network testnet
```

### Step 3: Configure Multi-Sig Signers and Thresholds
Configure the account with a **2-of-3 threshold**:
- Master key weight: 0 (disables the single private key, preventing single-key bypass)
- Signer 1 weight: 1
- Signer 2 weight: 1
- Signer 3 weight: 1
- Low threshold: 1
- **Medium threshold: 2** (governs Soroban contract invocations)
- High threshold: 2 (governs signer/threshold modifications)

Run using the Stellar CLI / Horizon transaction:
```bash
# Obtain public keys
SIGNER1_PK=$(stellar keys address signer1)
SIGNER2_PK=$(stellar keys address signer2)
SIGNER3_PK=$(stellar keys address signer3)
ADMIN_PK=$(stellar keys address multisig-admin)

# Add Signer 1 with weight 1
stellar tx new set-options \
  --source-account multisig-admin \
  --signer "$SIGNER1_PK:1" \
  --network testnet > set_signer1.json
stellar tx sign --tx-file set_signer1.json --sign-with-key multisig-admin > signed1.json
stellar tx send --tx-file signed1.json --network testnet

# Add Signer 2 with weight 1
stellar tx new set-options \
  --source-account multisig-admin \
  --signer "$SIGNER2_PK:1" \
  --network testnet > set_signer2.json
stellar tx sign --tx-file set_signer2.json --sign-with-key multisig-admin > signed2.json
stellar tx send --tx-file signed2.json --network testnet

# Add Signer 3 with weight 1, configure thresholds, and remove master key weight (weight: 0)
stellar tx new set-options \
  --source-account multisig-admin \
  --signer "$SIGNER3_PK:1" \
  --master-weight 0 \
  --low-threshold 1 \
  --med-threshold 2 \
  --high-threshold 2 \
  --network testnet > finalize_multisig.json
stellar tx sign --tx-file finalize_multisig.json --sign-with-key multisig-admin > signed_final.json
stellar tx send --tx-file signed_final.json --network testnet
```

The account `$ADMIN_PK` is now a 2-of-3 multi-signature account.

---

## 4. Manual Verification Runbook

### A. Deploy and Initialize Contract with Multi-Sig Admin
Deploy the Campaign contract and initialize it with the multi-sig account address:

```bash
# Deploy WASM
WASM_HASH=$(stellar contract install --wasm target/wasm32-unknown-unknown/release/campaign.wasm --source-account deployer --network testnet)
CONTRACT_ID=$(stellar contract deploy --wasm-hash $WASM_HASH --source-account deployer --network testnet)

# Initialize with multi-sig admin
stellar contract invoke \
  --id $CONTRACT_ID \
  --source-account deployer \
  --network testnet \
  -- \
  initialize \
  --admin "$ADMIN_PK"
```

Verify that `get_admin` returns `$ADMIN_PK`:
```bash
stellar contract invoke \
  --id $CONTRACT_ID \
  --network testnet \
  -- \
  get_admin
# Output: "G..." (matches $ADMIN_PK)
```

### B. Verification Test 1: Insufficient Signatures (Failure Case)
Construct an emergency freeze transaction and sign it with only **1 signer** (weight = 1, below medium threshold 2):

```bash
# Build unsigned invocation
stellar contract invoke \
  --id $CONTRACT_ID \
  --source-account signer1 \
  --network testnet \
  --build-only \
  -- \
  freeze > freeze_unsigned.json

# Sign with only signer1
stellar tx sign \
  --tx-file freeze_unsigned.json \
  --sign-with-key signer1 \
  --network testnet > freeze_1_sig.json

# Attempt to submit: MUST FAIL with txBAD_AUTH or HostError
stellar tx send --tx-file freeze_1_sig.json --network testnet
# Expected result: FAILED (threshold 2 not met, transaction rejected)
```

### C. Verification Test 2: Threshold Signatures Met (Success Case)
Now add the signature from **Signer 2** (total weight = 2, meeting medium threshold 2):

```bash
# Add second signature from signer2
stellar tx sign \
  --tx-file freeze_1_sig.json \
  --sign-with-key signer2 \
  --network testnet > freeze_2_sigs.json

# Submit multi-signed transaction: MUST SUCCEED
stellar tx send --tx-file freeze_2_sigs.json --network testnet
# Expected result: SUCCESS
```

Query freeze status:
```bash
stellar contract invoke \
  --id $CONTRACT_ID \
  --network testnet \
  -- \
  is_frozen
# Output: true
```

### D. Verification Test 3: Unfreeze with Alternate Threshold Pair (Signer 2 + Signer 3)
Signers 2 and 3 unfreeze the contract:

```bash
stellar contract invoke \
  --id $CONTRACT_ID \
  --source-account signer2 \
  --network testnet \
  --build-only \
  -- \
  unfreeze > unfreeze_unsigned.json

stellar tx sign --tx-file unfreeze_unsigned.json --sign-with-key signer2 --network testnet > unfreeze_sig2.json
stellar tx sign --tx-file unfreeze_sig2.json --sign-with-key signer3 --network testnet > unfreeze_sig2_3.json

stellar tx send --tx-file unfreeze_sig2_3.json --network testnet
# Expected result: SUCCESS
```

---

## 5. Smart Contract Multi-Sig Wallet Alternative

If the organization prefers an on-chain proposal and voting flow rather than off-chain transaction co-signing, the `admin` can be set to a smart contract address (such as `contracts/multi_sig`):
- The `multi_sig` contract collects votes on-chain.
- Once a proposal reaches its configured threshold, the multi-sig contract calls the target admin method (`freeze`, `pause`, `set_admin`).
- Soroban's cross-contract authorization (`contract.require_auth()`) validates that the calling contract is the registered admin.

---

## 6. Automated Test Coverage

The test suite in [contracts/campaign/src/lib.rs](file:///c:/Users/hp/OneDrive/Desktop/GrantFox/stellarAid-contract/contracts/campaign/src/lib.rs) includes automated tests verifying multi-sig readiness:

1. `test_admin_multisig_readiness_with_contract_wallet`:
   Verifies that a multi-sig smart contract address can be initialized as admin and successfully execute `freeze` and `unfreeze` invocations.
2. `test_admin_multisig_simulated_threshold_authorization`:
   Verifies threshold logic where an invocation with unsatisfied signer weights fails, while an invocation with threshold-satisfied aggregate signatures succeeds.
3. `test_admin_multisig_rotation_between_multisig_accounts`:
   Verifies that `set_admin` seamlessly rotates ownership from one multi-sig address to another multi-sig address with dual authorization.
