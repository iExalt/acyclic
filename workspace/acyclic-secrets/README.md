# `acyclic-secrets`

Hands credentials from a SOPS-encrypted file to containers as Podman secrets, without the values ever touching a
command line or an unencrypted file.

## The secrets file

A dotenv file encrypted with SOPS to the user's age key, kept under `~/.config/acyclic/`, outside every repository.
Each line is `NAME=value`. `sops decrypt` needs the age identity, which SOPS finds the usual way (`SOPS_AGE_KEY_FILE`,
`SOPS_AGE_KEY_CMD` or its default key file); this crate passes no key of its own.

## How a value reaches a container

1. `sops decrypt --output-type dotenv <file>` runs with its output captured in memory, which is wiped when dropped.
2. Each value a run needs goes to `podman secret create --label acyclic.run=<run> <name> -` on standard input, byte
   for byte, with no trailing newline.
3. The container gets it with `--secret <name>,type=env,target=<VARIABLE>`, so it appears in the container's
   environment but not in its configuration or `podman inspect`.
4. The secret is removed with `podman secret rm` when the run ends.

While a secret exists, Podman's file driver keeps it unencrypted on the machine's disk, and anyone who can reach
Podman's socket can read it with `podman secret inspect --showsecret`; that is why secrets live for one run.
