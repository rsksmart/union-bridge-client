# General

## Agent Style
- Use brief and direct responses
- If you don't know the response, state it
- Please challenge me when appropriate, instead of just agreeing - I prefer critical thinking
- Point out when you think my approach might be overcomplicating things, even if I didn’t ask explicitly

# Rust

## General Standards
- Follow Rust API Guidelines and naming conventions (snake_case for functions/variables, PascalCase for types)
- Use `rustfmt` for consistent formatting (TODO: not yet - and `clippy` for linting)
- Prefer composition over inheritance when designing structs and traits
- Always handle `Result` and `Option` types explicitly - avoid `.unwrap()` in production code except where panic is intentional
- Use `?` operator for error propagation instead of manual unwrap/match when appropriate
- Prefer Rc, RefCell, Arc... etc. wrapping in inner fields rather than in parent struct

## Memory Safety & Ownership Review
- Verify ownership semantics are correct - question unnecessary `.clone()` calls
- Check that lifetime annotations make sense and aren't overly restrictive
- Ensure borrowing patterns don't create unnecessary complexity
- Look for potential use of `Cow<'_, T>` where data might be borrowed or owned
- Review shared state usage - question if `Arc<Mutex<T>>` is truly necessary

## Unsafe Code Review
- **Critical**: All `unsafe` blocks must have detailed comments explaining safety contracts
- Document what invariants are assumed and under what conditions the code would break
- Verify pointer arithmetic is bounds-checked and alignment requirements are met
- Ensure FFI calls properly handle null pointers and buffer sizes
- Check that unsafe code doesn't violate Rust's aliasing rules

## Security Review
- Validate all user inputs using proper parsing (avoid `.parse().unwrap()`)
- Use parameterized queries or prepared statements for database operations
- Implement proper authentication and authorization checks
- Review sensitive data handling - ensure secrets aren't logged or exposed
- Check for integer overflow in arithmetic operations (use checked arithmetic where needed)
- Audit dependencies regularly with `cargo audit`

## Bitcoin-Specific Considerations
- **Critical**: Avoid `TxId::from_slice`, `TxId::from_byte_array`, or any other method that relies on `hashes::hash_newtype!` - these reverse the byte order
- Any calls to those methods should be encapsulated in `common::types::TxIdParser` struct so it handles the reversal properly
- If importing `use bitcoin::hashes::Hash;`, be extra cautious with its usage due to potential byte order reversal issues
- When working with transaction IDs, be extremely careful about byte ordering to prevent silent data corruption

## Performance Considerations
- Identify unnecessary heap allocations and excessive cloning in hot paths
- Check for long-held mutex locks that could cause contention
- Review iterator chains for opportunities to avoid intermediate collections
- Look for blocking operations in async contexts
- Consider zero-copy optimizations where data is being unnecessarily copied
- Profile-guided optimization opportunities (especially in tight loops)

## Code Quality & Idioms
- Check for code duplication and opportunities for abstraction
- Ensure proper separation of concerns between modules
- Validate that trait implementations are necessary - avoid over-abstraction
- Use `match` expressions instead of complex `if let` chains when appropriate
- Prefer `impl Trait` over `Box<dyn Trait>` when possible for better performance
- Review error types - use `thiserror` or `anyhow` consistently for error handling
- Ensure comprehensive test coverage, especially for error paths and edge cases

## Concurrency & Async Review
- Check for potential deadlocks in multi-threaded code
- Verify proper use of atomic operations and memory ordering
- Review async code for `.await` points that might cause blocking
- Ensure channels are properly closed to avoid resource leaks
- Look for race conditions in lockfree code patterns

## Docker & Deployment
- Docker setup should not depend on .envrc or direnv env vars.
