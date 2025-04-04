# Key Manager

To generate a key pair, please run:

```
cargo run new-key -p test -d <PATH_TO_STORE_IT>
```

To derive a key pair from a mnemonic, please run:

```
cargo run derive-public-data -p test -k <PATH_TO_KEY>
```

Alternative options to local storage exist (check [here](https://alloy.rs/examples/wallets/keystore_signer.html)), but
they are not implemented yet:

- Yubi
- Trezor
- Ledger
- AWS
- etc
