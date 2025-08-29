# Union Bridge — Peg-in QA Test Plan

## Scope
This document covers peg-in and peg-out processes of the Union Bridge from the user's perspective.

---

## 1. Introduction

The Union Bridge connects **Bitcoin** and **Rootstock**.  
A **peg-in** means a user deposits BTC into a Taproot address controlled by Union Bridge committees, and then receives RBTC on Rootstock.

The peg-in flow has two user-visible steps:

1. **Step 1 — Request deposit address**  
   The user asks the Union Bridge for a deposit address **Y**.  
   The wallet provides:
    - Rootstock address (**A**) to receive RBTC
    - Amount in satoshis (**v**)
    - Bitcoin reimbursement address (**R**)

   The Bridge responds with a Taproot address (**Y**) where the user must deposit funds.

2. **Step 2 — Broadcast PegInRequest transaction**  
   The user creates a Bitcoin transaction with:
    - **Output 0**: `v` sats to **Y**
    - **Output 1**: OP_RETURN with metadata (`"RSK_PEGIN"`, **N**, **A**, **R**)

   After confirmations and an SPV proof, the Union Bridge contract registers the peg-in and mints RBTC to **A**.

A **peg-out** means a user burns RBTC on Rootstock and receives BTC on Bitcoin from the Union Bridge committee-controlled funds. The peg-out flow works in this way from the user's perspective:
- The user submits a request to burn RBTC and receive BTC. The user provides:
  - Rootstock address (**A**) holding RBTC
  - Bitcoin destination address (**B**) to receive BTC
  - Amount in wei (**w**) to burn (converted to satoshis **v**)
- The Bridge validates the request and burns the RBTC tokens.

---

## 2. Concepts

- **Peg-in request transaction**:  
  Bitcoin transaction with 2 main outputs:
    - **Output 0 (Y)**: Taproot deposit address. BTC goes here. Encodes spending paths:
        - key-path: Committee N
        - optional backup: Committee N+1
        - optional reimbursement: R with CSV L
    - **Output 1 (OP_RETURN)**: Metadata. Carries `"RSK_PEGIN"`, packet number **N**, Rootstock address **A**, reimbursement address **R**

- **A**: Rootstock address credited with RBTC
- **R**: Bitcoin reimbursement address (or all-zeros to omit)
- **v**: BTC deposit amount. Must equal stream denomination
- **L**: Relative timelock (CSV) for reimbursement
- **N**: Packet number (committee index)
- **S**: Current stream/packet state
- **PegInRequestID**: Txid of PegInRequest (Bitcoin tx)
- **SPV proof**: Proof that the PegInRequest tx was included in a Bitcoin block (txid + merkle branch + header)

- **Peg-out request transaction**:  
  Rootstock transaction that burns RBTC and registers withdrawal request:
    - Burns **w** wei of RBTC from user's address **A**
    - Records Bitcoin destination address **B**
    - Creates peg-out request with unique **PegOutRequestID**

- **A**: Rootstock address holding RBTC to burn
- **B**: Bitcoin destination address for withdrawal
- **w**: RBTC amount in wei to burn
- **v**: BTC amount in satoshis to receive (w / 10^10)
- **N**: Committee/packet number processing the request
- **PegOutRequestID**: Unique identifier for the peg-out request

---

## 3. General Acceptance Criteria

- Step 1 always produces valid, verifiable **Y** for valid inputs
- Step 2 only credits RBTC if BTC went to the correct **Y** matching OP_RETURN metadata
- Valid peg-out requests are only accepted from addresses with sufficient RBTC balance
- Invalid or malicious cases are rejected safely
- Users can recover funds even if mistakes happen (expired Y, missing OP_RETURN, etc.)
- Users receive exact BTC amount corresponding to burned RBTC (minus fees)
- Failed peg-outs can be retried or refunded appropriately

---

## 4. High-Level Test Cases

### 4.1 Step 1 — User Requests Deposit Address

#### Positive Cases
- **S01 Happy path**: Valid A, valid R, valid v → Response is a valid Taproot address
    - Mainnet: `bc1p...`
    - Testnet: `tb1p...`
    - Regtest: `bcrt1p...`
- **Supported denominations**: v ∈ {100000, 1000000, 10000000, 100000000, 1000000000} satoshis
- **Determinism**: Same inputs (A, R, v) under stable committee → identical Y returned
- **Different reimbursement keys**: Same A, v but different R → different Y returned
- **Different Rootstock addresses**: Same R, v but different A → different Y returned
- **Different value**: Same A, R but different v (different denomination) → different Y returned

#### Negative Cases
- Invalid or missing Rootstock address → error
- Invalid or missing reimbursement key → error
- Invalid or missing denomination (test boundaries) → error

#### Additional Cases
- **Stress test**: Many requests in short time window → service remains stable

---

### 4.2 Step 2 — User Broadcasts Deposit Transaction

#### Positive Cases
- **Happy path**: Valid Taproot output 0 with spending paths, valid OP_RETURN metadata in output 1, SPV proof valid, sufficient confirmations → Peg-in processed correctly
- **R omitted**: R=0 → Peg-in processed correctly
- **Change output present**: Third output allowed → Peg-in processed correctly
- **Recovery 1**: User spends funds via reimbursement path (after CSV L) → BTC recovered
- **Recovery 2**: User lost tweak backup, but OP_RETURN data available → BTC recovered
- **Recovery 3**: User lost tweak backup and OP_RETURN omitted → Brute-force reconstruction possible → BTC recovered
- **Address determinism**: Same inputs (A, R, v, committee) → always same Y generated

#### Negative Cases — Transaction Structure
- Missing or malformed output 0
- Missing or malformed output 1
- Wrong Taproot key (output 0 not matching recompute)
- Output 0 has invalid amount (≠ denomination, dust, etc.)
- Output 1 has wrong A
- Output 1 has wrong R
- Output 1 has wrong N

#### Negative Cases — SPV and Blockchain
- Invalid merkle proof (branch fails validation)
- Block not found (non-existent block hash)
- Insufficient confirmations
- Invalid txid (hash doesn’t match transaction data)
- Malformed transaction (fails Bitcoin rules)
- Double spending (duplicate txid already processed)
- Future block (timestamp too far ahead)

#### Negative Cases — Business Logic
- Committee mismatch (Y generated under different committee than current)
- Already registered (same txid processed before)

### 4.3 Step 3 — User Requests Peg-out

#### Positive Cases
- **Happy path**: Valid A with sufficient RBTC, valid B, valid w → Request accepted, RBTC burned, PegOutRequestID generated
- **Supported Bitcoin addresses**: B supports P2PKH, P2SH, P2WPKH, P2WSH, P2TR formats
    - Mainnet: `1...`, `3...`, `bc1q...`, `bc1p...`
    - Testnet: `m...`, `2...`, `tb1q...`, `tb1p...`
    - Regtest: `m...`, `2...`, `bcrt1q...`, `bcrt1p...`
- **Denomination boundaries**: Test minimum (e.g., 10,000 wei) and maximum amounts
- **Exact balance**: User has exactly w wei → Request succeeds, balance becomes 0
- **Multiple requests**: Same user submits multiple peg-out requests → All processed independently
- **Gas fee handling**: Request includes proper gas fee → Transaction confirmed on Rootstock

#### Negative Cases
- **Invalid Bitcoin address**: Malformed B → Request rejected
- **Invalid RSK address**: Malformed A → Request rejected
- **Invalid amount**: w = 0 → Request rejected
- **Invalid amount**: w < minimum denomination → Request rejected
- **Invalid amount**: w doesn't match any stream denomination → Request rejected
- **Unsupported address type**: B uses unsupported Bitcoin script type → Request rejected
- **Gas issues**: Insufficient gas → Transaction reverts, no state change


---

## 5. Next Steps

- Map each test to level: **unit, integration, e2e**
- Identify which cases already covered by existing tests (client repo, contracts repo)
- Determine required tools, mocks, and data injection methods
- Define testing priorities and automation scope
- Define in-scope vs out-of-scope
- Establish entry/exit criteria for QA cycles

---