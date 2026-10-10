# Update signing tool

This separate locked Cargo package is both the release signing CLI and Recorde's shared verification library. Production trusts only the public keys compiled from `packaging/update-keys.json`. Downloaded metadata cannot add keys.

## Key generation

Generate the production key on Linux in a private directory outside the repository. The CLI creates new files and never overwrites keys. The private file contains a base64-encoded 32-byte Ed25519 seed; the public file contains a schema-1 key set. Unix private files are mode 0600. On Windows, create a directory whose ACL grants access only to the signing operator before generating any private key; the CLI does not configure Windows ACLs.

```sh
umask 077
mkdir -m 700 /private/recorde-signing
cargo run --locked --manifest-path tools/update-signing/Cargo.toml -- generate \
  --key-id recorde-2026-01 \
  --private-key-file /private/recorde-signing/key.txt \
  --public-key-file packaging/update-keys.json
```

Never commit the private file. CI reads `RECORDE_UPDATE_SIGNING_KEY` from its secret store, or supplies `--key-file` pointing to a protected file. Key bytes are never command arguments and are never printed.

## Manifest, sign and verify

```sh
cargo run --locked --manifest-path tools/update-signing/Cargo.toml -- manifest \
  --artifact dist/recorde-1.2.3-linux-x86_64 \
  --asset-name recorde-1.2.3-linux-x86_64 \
  --version 1.2.3 --key-id recorde-2026-01 --channel stable \
  --issued-at 1791590400 --expires-at 1794182400 \
  --platform linux --arch x86_64 --kind portable --output manifest.json
cargo run --locked --manifest-path tools/update-signing/Cargo.toml -- sign \
  --manifest manifest.json --output recorde-update-linux-x86_64-portable.json
cargo run --locked --manifest-path tools/update-signing/Cargo.toml -- verify \
  --keys packaging/update-keys.json \
  --envelope recorde-update-linux-x86_64-portable.json \
  --artifact dist/recorde-1.2.3-linux-x86_64
```

Use current Unix seconds for `issued-at`; expiration must be later and at most 180 days away. The first release containing this updater requires manual installation. Release envelopes are named `recorde-update-{platform}-{arch}-{kind}.json`. Supported package kinds are Windows `portable`/`msi`, Linux `portable`/`appimage`/`deb`, macOS `portable`/`dmg` (download only). Raw portable artifacts update only the executable; use the full installer when adjacent bundled resources change.

## Exact signature format

The envelope has exactly `manifest_base64` and `signature_base64`, both standard padded base64. Ed25519 signs the concatenation of the ASCII bytes `Recorde update manifest v1`, one zero byte, and the **original manifest JSON bytes**. There is no JSON canonicalization, newline stripping, reserialization or prehash. The verifier checks the signature strictly against locally pinned keys before parsing manifest fields. Its `key_id` must identify the key that verified it.

The manifest contains `schema` (1), `key_id`, `channel`, `version`, `issued_at`, `expires_at`, `platform`, `arch`, `package_kind`, `artifact_name`, `url`, `size`, `sha256`. Unknown and duplicate fields are rejected. SHA-256 is lowercase hexadecimal. The URL must exactly name the artifact under `https://github.com/antonydoyle/Recorde/releases/download/v{version}/`. Stable excludes prerelease versions. Update clients also check package/platform/channel, time, the running version, and their persisted highest accepted version and issuance time.

For rotation, ship both old and new public keys in an authenticated application release before signing later releases with the new key. Keep key IDs unique. Never distribute a new trusted key solely through the update manifest.
