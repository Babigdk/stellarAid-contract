# Error Codes Documentation

This document lists all error codes used across stellarAid contracts.

## Common Error Codes

| Code | Name | Description |
|------|------|-------------|
| E001 | NotInitialized | Contract has not been initialized |
| E002 | AlreadyInitialized | Contract is already initialized |
| E003 | Unauthorized | Caller is not authorized to perform this action |
| E004 | InvalidAmount | The provided amount is invalid (zero or negative) |
| E005 | InsufficientBalance | Insufficient balance for the operation |
| E006 | InvalidAddress | The provided address is invalid or empty |
| E007 | OperationFailed | Generic operation failure |
| E008 | Overflow | Arithmetic overflow detected |
| E009 | DeadlineExpired | Transaction deadline has expired |
| E010 | NotFound | Requested resource was not found |

## Usage

Error codes are returned as part of contract error responses. Each contract
maps these codes to specific ContractError enum variants.

## Insufficient input and payouts

Not every failure is a contract error. Two conditions are reported as panic
strings whose text is the **name** of the corresponding typed variant, so an
indexer sees the same identifier whether the contract has a `#[contracterror]`
enum yet or not:

| Panic string | Typed equivalent | Raised by |
|---|---|---|
| `InsufficientContractBalance` | `Error::InsufficientContractBalance` (308) | `campaign::balance::require_contract_balance`, `withdrawal::approve_withdrawal`, `donation::refund` |
| `InvalidAssets` | `Error::InvalidAssets` (311) | `campaign::sanitize::require_asset_code`, `require_reason_hash` |

Both are raised **before** the state change they protect, so a rejected call
leaves nothing behind. See
[the balance module](../contracts/campaign/src/balance.rs) and
[the sanitization module](../contracts/campaign/src/sanitize.rs).

## New codes

| Contract | Code | Name | Description |
|---|---|---|---|
| dispute arbiter | 10 | `NoteTooLong` | The resolution note exceeds `MAX_MEMO_LEN` (256 bytes). Appended — existing codes 1–9 are unchanged, per the permanence rule above. |
