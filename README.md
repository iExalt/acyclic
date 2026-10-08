# Rust Workspace Template

A reusable GitHub and `cargo-generate` template for Rust projects organized as a Cargo workspace.

## Create a workspace

Choose either creation method.

### GitHub

Select **Use this template** on GitHub and create a repository from
[`iExalt/rust-workspace`](https://github.com/iExalt/rust-workspace).

### cargo-generate

```sh
cargo generate \
  --git https://github.com/iExalt/rust-workspace.git \
  --name my-project
```

Then prepare the generated repository:

1. Update this README.
2. Add project-specific metadata, such as `authors` and `repository`, to
   `[workspace.package]` in `Cargo.toml`.
3. Update the copyright holder in `LICENSE` when needed.
4. Install the pinned development tools:

   ```sh
   mise trust
   mise install
   ```

## Add a crate

Generate a library crate from the included template:

```sh
mise exec -- cargo generate \
  --path workspace/template \
  --destination workspace \
  --name my-crate \
  --vcs none
```

Crates created directly under `workspace/` are included in the Cargo workspace automatically.
Generated crates inherit their version, Rust edition, and license from the workspace manifest.

## Develop

After adding a crate, run the standard workspace checks through the Rust toolchain managed by
`mise`:

```sh
mise exec -- cargo fmt --all --check
mise exec -- cargo clippy --workspace --all-targets --all-features -- -D warnings
mise exec -- cargo test --workspace --all-features
```
