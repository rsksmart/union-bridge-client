# Union Bridge - Check Fork Function

For repository-level setup and workflow context, start with the [Repository README](../README.md) and the
[Contributing Guide](../CONTRIBUTING.md). This README stays focused on the check-fork component itself.

The `check_fork` function (stateless) verifies a sequence of consecutive Rootstock blocks, which are provided as input.
This function plays a critical role in the validation processes of the **Union Bridge** implementation. Its primary
tasks are to:

- Confirm the presence of a single withdrawal command within the specified blocks.
- Validate that a specified amount of **Proof of Work (PoW)** has been accumulated across the sequence.

# Role in Union Bridge

The `check_fork` function is a fundamental component of the Union Bridge system. It operates as follows:

1. **Execution within a zkVM**: The `check_fork` function runs in a zero-knowledge virtual machine (zkVM) environment
   within [BitVMX](https://bitvmx.org/files/bitvmx-whitepaper.pdf).
2. **Generation of STARK Proofs**: The execution produces a STARK (Scalable Transparent Argument of Knowledge) proof.
3. **Conversion to SNARK**: The STARK proof is then converted into a SNARK (Succinct Non-interactive Argument of
   Knowledge), a more compact proof format.
4. **Verification in BitVMX**: The SNARK is verified within BitVMX. If the _prover_ (aka _operator_) and _verifier_ (aka
   _Watch Tower_) generate differing SNARKs, the verification process will fail, triggering an on-chain dispute
   challenge until the diverging step is found.
