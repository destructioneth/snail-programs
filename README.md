# Snail Programs

This repository contains the source code for the Snail Game and Snail Launch Solana programs.

## Snail Game: How It Works

The Snail Game is a decentralized application on Solana where the goal is to prevent a "snail" (representing the token's market cap) from being "touched."

### Technical Overview

The core of the game is the `snail_game` program (`2PgtpKBFjWgdk7wLxZD7xC8sc6qpsXmDw1dPKQnmdJPT`), which you can view on [Solscan](https://solscan.io/account/2PgtpKBFjWgdk7wLxZD7xC8sc6qpsXmDw1dPKQnmdJPT).

1.  **Game State:** A central `GameState` account holds all the game's parameters, such as start/end times, the target market cap, and the liquidity pool addresses. This account is initialized once by the owner.
2.  **Required Market Cap:** The program calculates a "required market cap" that increases over time along a configurable curve. This value represents the minimum market cap the token must maintain at any given moment. You can see this logic in the `check_required_market_cap` function.
3.  **"Touching the Snail":** Anyone can call the `touch_snail` function at any time. This function compares the token's *current* market cap (calculated from the liquidity pool reserves) against the *required* market cap.
4.  **Game Over:** If the current market cap is less than or equal to the required market cap, the game ends. The `touch_snail` function freezes the snail's liquidity pool token account and then **permanently revokes its own freeze authority**. This action is irreversible and ensures that once the game is over, the liquidity is locked forever.
5.  **Immutability and Trust:**
    *   **No Ownership Functions:** After the initial `initialize` instruction is called, there are no functions that allow an owner or admin to change the game's parameters, withdraw funds, or otherwise interfere with the game's logic. The `owner` field in the `GameState` is for informational purposes only and grants no special privileges.
    *   **Revoked Upgrade Authority:** The upgrade authority for the on-chain program has been permanently revoked. This means the code cannot be changed, ensuring that the game logic is immutable and will run as designed forever.

This design guarantees that the game is autonomous and transparent. The rules are enforced by the code on the blockchain, and since the program is verified and cannot be upgraded, you can be certain that what you see in this repository is exactly what is running on-chain.

## Program Verification

The programs in this repository have been verifiably linked on-chain to this source code. This means you can cryptographically prove that the deployed program matches the code in this repository.

### Snail Game Verification Status

-   **Program ID:** `2PgtpKBFjWgdk7wLxZD7xC8sc6qpsXmDw1dPKQnmdJPT`
-   **On-Chain Hash:** `f0f99112bc68f7f4c4a9c4023db47f2c1dad744df6b188ad4653c67f2d2c2e69`
-   **Verification PDA:** `4jkNUHTMFgimQCcN9tji3keLyrrGsgsqJdMpX6ngdnPf`

**Note on Solscan:** While the on-chain verification data has been successfully uploaded, Solscan's UI may not yet reflect the "Verified" status. This is due to an issue with the remote verification service (`verify.osec.io`) and its handling of Cargo workspace projects. The on-chain proof, however, is immutable and correct.

### How to Manually Verify

You can confirm the verification yourself using the `solana-verify` CLI.

#### 1. Install `solana-verify`

```bash
cargo install solana-verify
```

#### 2. Get the On-Chain Program Hash

This command queries the Solana blockchain for the hash of the deployed program.

```bash
solana-verify get-program-hash 2PgtpKBFjWgdk7wLxZD7xC8sc6qpsXmDw1dPKQnmdJPT -u mainnet
```

**Expected Output:**
```
f0f99112bc68f7f4c4a9c4023db47f2c1dad744df6b188ad4653c67f2d2c2e69
```

#### 3. Build the Program Locally and Get its Hash

This command clones the repository and builds the program in a deterministic environment to reproduce the binary hash.

```bash
# Clone the repo
git clone https://github.com/destructioneth/snail-programs.git
cd snail-programs

# Build the specific program
# This requires Docker to be installed and running.
solana-verify build --library-name snail_game

# Get the hash of the local build artifact
solana-verify get-executable-hash target/deploy/snail_game.so
```

The hash from this command should also match the on-chain hash.

#### 4. View the On-Chain Verification Data

This command fetches the verification metadata that was uploaded to the blockchain. This data links the program ID to this GitHub repository.

```bash
solana-verify get-program-pda --program-id 2PgtpKBFjWgdk7wLxZD7xC8sc6qpsXmDw1dPKQnmdJPT -u mainnet
```

**Expected Output:**
```
----------------------------------------------------------------
Address: 4jkNUHTMFgimQCcN9tji3keLyrrGsgsqJdMpX6ngdnPf
----------------------------------------------------------------
Program Id: 2PgtpKBFjWgdk7wLxZD7xC8sc6qpsXmDw1dPKQnmdJPT
Signer: GpP3LL8eHjMUXj7ZSnnWg6tTb3qp7Sz41kVDjGGNree9
Git Url: https://github.com/destructioneth/snail-programs
Commit: 2edc5d8376badfebed25a11fb11f6e1e040b29d4
Deployed Slot: 380917241
Args: ["--library-name", "snail_game"]
Version: 0.4.12
```

This on-chain data serves as the ultimate proof that the program is verifiably linked to this source code.
